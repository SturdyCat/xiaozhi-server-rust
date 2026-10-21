//! 所有生产 TTS 文本共用的有界持久化音频库。
//!
//! **磁盘存的就是可直接播放的裸音频文件**：`<hash>.pcm` = 24kHz mono S16LE 整句 PCM。
//! 无自定义容器、无头部、无校验尾、无 json 登记表——`ffplay -f s16le -ar 24000
//! -ch_layout mono <file>.pcm`（或 Audacity 导入原始数据）即可直接播放/检查，删任意
//! 文件都不影响其余缓存。文件名 = 合成身份的稳定 FNV-1a 64（引擎签名/音色/语速/文本
//! 各自哈希后的稳定投影，文件内外都不出现凭据明文）。
//!
//! 磁盘身份刻意**不含下行编码参数**（采样率/帧长/编解码档）——那属于"投递"，不属于
//! "合成成品"。换设备档位、改 `[audio]` 帧长都不作废磁盘缓存；命中后在读路径按会话
//! 实际下行格式重采样 + 编码（与流式合成同一 [`crate::app::downlink::framer`] 链路）。
//!
//! 内存 LRU 由 `TtsCache` 管理（内存里是**已编码下行帧**，键含下行参数）；磁盘 miss
//! 重新流式合成。写临时文件后原子 rename，读者永远看不到半截文件；失败降级不影响下行。
//! 磁盘限制文件数/总字节/单句字节，超限按 mtime 淘汰（内容寻址缓存，不是永久归档）。
//! 清理策略刻意最简：文案/音色改了旧文件自然不再被命中，靠容量淘汰；**不做任何自动
//! 删除**——旧格式遗留（`.hot` 容器、`managed.json`）也不会被读取或清理，是否删掉
//! 由使用者手动决定。测试台强制真合成不复用缓存；AIUI 云闭环已在云端合成。

use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use crate::app::tts_cache::TtsCache;
#[cfg(test)]
use crate::config::Config;

/// 首选配置来源目录；无来源时复用记忆库目录（包括相对路径的当前目录）。
/// 这里只派生路径，不建目录或打开记忆库，memory.enabled=false 的零成本契约不变。
pub(crate) fn cache_dir(config_location: Option<&str>, memory_db_path: &str) -> PathBuf {
    Path::new(
        config_location
            .filter(|p| !p.is_empty())
            .unwrap_or(memory_db_path),
    )
    .parent()
    .filter(|p| !p.as_os_str().is_empty())
    .unwrap_or_else(|| Path::new("."))
    .join("hot-audio")
}

/// 磁盘固定存储采样率：TTS 引擎产物统一为 24k（kokoro/讯飞/内置全是 24k），
/// 写入时经重采样归一，读取时按会话实际下行格式再重采样。
pub const PCM_SAMPLE_RATE: u32 = 24_000;
/// 全文本缓存的固定安全限制；后续如需可配置，统一从配置注册表接入。
pub const MAX_FILES: usize = 4096;
pub const MAX_TOTAL_BYTES: u64 = 512 * 1024 * 1024;
pub const MAX_ENTRY_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_FRAMES: usize = 16_384;

/// 一句热句音频的**磁盘身份**：影响 TTS 合成结果的一切参数（不含下行编码参数——
/// 那是投递侧的，换设备档位/帧长必须能复用同一份 PCM 成品）。
///
/// 生产构造入口只有 [`HotAudioKey::from_lease`] 一个——预取合成路径与唤醒问候路径都走
/// 它，避免两处各写一遍参数顺序导致"键永远不相等、缓存永远不命中"（这类 bug 不会报错，
/// 只会表现为"缓存没用"，最难查）。
///
/// ⚠️ 不派生 `Eq`（`speed` 是 `f32`）；本结构只在同进程内做校验，`PartialEq` 足够。
#[derive(Debug, Clone, PartialEq)]
pub struct HotAudioKey {
    /// 引擎签名（**含凭据**，只在内存里流转；落盘只写其 FNV）。
    pub engine_sig: String,
    pub speaker: i32,
    pub speed: f32,
    pub text: String,
}

impl HotAudioKey {
    /// 生产唯一入口：身份来自实际实例租约，不来自可能重建失败的请求配置。
    pub fn from_lease(lease: &crate::engine::TtsLease, text: &str) -> Self {
        Self {
            engine_sig: lease.fingerprint().to_string(),
            speaker: lease.config().speaker,
            speed: lease.config().speed,
            text: text.to_string(),
        }
    }

    /// 配置投影只供离线格式测试；生产无法调用，必须使用实际租约构造身份。
    #[cfg(test)]
    pub fn from_session(live: &Config, text: &str) -> Self {
        Self {
            engine_sig: crate::plugins::registry::signature_of(
                live,
                crate::plugins::registry::Capability::Tts,
            ),
            speaker: live.tts.speaker,
            speed: live.tts.speed,
            text: text.to_string(),
        }
    }

    /// 内存缓存键：与 [`crate::app::prefetch`] 的合成路径**完全同口径**。
    ///
    /// 内存里缓存的是**已编码下行帧**，所以键含**整个**下行格式（采样率/帧长之外还有
    /// 编解码档与契约版本；磁盘身份不含它们）；命中它 = 走既有下行链路的零合成零编码
    /// 路径（预取 worker 直接投递已编码帧）。
    pub fn mem_key(&self, format: crate::app::audio::media::AudioFormat) -> u64 {
        TtsCache::make_key(
            &self.engine_sig,
            self.speaker,
            self.speed,
            format,
            &self.text,
        )
    }

    /// 落盘文件名（不含扩展名）：跨进程、跨 Rust 版本稳定的 FNV-1a 64。
    ///
    /// 输入是各字段的**稳定投影**（签名与文本先各自 FNV 一次，再连同标量打包哈希），
    /// 因此文件里外都不会出现凭据明文。
    pub fn file_id(&self) -> u64 {
        let mut packed = Vec::with_capacity(24);
        packed.extend_from_slice(&fnv1a64(self.engine_sig.as_bytes()).to_le_bytes());
        packed.extend_from_slice(&self.speaker.to_le_bytes());
        packed.extend_from_slice(&self.speed.to_bits().to_le_bytes());
        packed.extend_from_slice(&fnv1a64(self.text.as_bytes()).to_le_bytes());
        fnv1a64(&packed)
    }
}

/// FNV-1a 64 位。选它是因为**实现只有几行、无依赖、且规范冻结**——持久化文件名要求
/// 跨版本稳定，标准库的 `DefaultHasher` 不提供这个保证。
fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// 落盘文件名：`<身份哈希>.pcm`。裸 PCM 文件，任何播放器/工具都能直接处理。
fn pcm_name(key: &HotAudioKey) -> String {
    format!("{:016x}.pcm", key.file_id())
}

/// 裸 PCM 载荷合法性：非空、整 S16LE 样本、不超过单句上限。
fn valid_pcm(pcm: &[u8]) -> bool {
    !pcm.is_empty() && pcm.len() % 2 == 0 && pcm.len() <= MAX_ENTRY_BYTES
}

/// 持久化热句音频库。`dir == None` = 纯内存降级（未加载配置文件时）。
pub struct HotAudioStore {
    dir: Option<PathBuf>,
    write_lock: Mutex<()>,
}

impl HotAudioStore {
    /// 按目录构造；`None` = 不持久化（`get` 恒 `None`、`put` 恒 `false`）。
    pub fn new(dir: Option<PathBuf>) -> Self {
        Self {
            dir,
            write_lock: Mutex::new(()),
        }
    }

    /// 是否真的在落盘（管理界面/启动日志据此说明"重启后仍在"）。
    pub fn is_persistent(&self) -> bool {
        self.dir.is_some()
    }

    /// 数据目录（观测/测试用）。
    pub fn dir(&self) -> Option<&Path> {
        self.dir.as_deref()
    }

    /// 读一条；命中返回**整句裸 PCM S16LE 字节**（[`PCM_SAMPLE_RATE`] 单声道）。
    ///
    /// 任何异常（文件不存在 / 非法载荷 / IO 错误）都返回 `None` 并删掉坏文件——读不出来
    /// 只意味着"要重新合成一次"，**永远不该影响会话**。
    pub fn get(&self, key: &HotAudioKey) -> Option<Vec<u8>> {
        let dir = self.dir.as_ref()?;
        // 防止损坏文件读者在另一个 writer 原子替换后误删刚生成的有效成品。
        let _guard = self.write_lock.lock().unwrap_or_else(|e| e.into_inner());
        let path = dir.join(pcm_name(key));
        let read = || -> std::io::Result<Vec<u8>> {
            let mut raw = Vec::new();
            std::fs::File::open(&path)?
                .take((MAX_ENTRY_BYTES + 1) as u64)
                .read_to_end(&mut raw)?;
            Ok(raw)
        };
        match read() {
            Ok(pcm) if valid_pcm(&pcm) => Some(pcm),
            Ok(pcm) => {
                tracing::debug!(
                    "热句音频作废（{} 字节，空/非整样本/超上限），删除 {}",
                    pcm.len(),
                    path.display()
                );
                let _ = std::fs::remove_file(&path);
                None
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => {
                tracing::warn!("热句音频读取失败（将重新合成）{}: {e}", path.display());
                None
            }
        }
    }

    /// 写一条（覆盖同名）。`pcm` = 整句裸 S16LE 字节（[`PCM_SAMPLE_RATE`] 单声道）。
    /// 返回是否真的落盘。
    ///
    /// 先写临时文件再 `rename`：读者**永远看不到半截文件**（唤醒可能在写入的同一瞬间发生）。
    pub fn put(&self, key: &HotAudioKey, pcm: &[u8]) -> bool {
        let Some(dir) = self.dir.as_ref() else {
            return false;
        };
        if !valid_pcm(pcm) {
            return false;
        }
        let _guard = self.write_lock.lock().unwrap_or_else(|e| e.into_inner());
        if let Err(e) = std::fs::create_dir_all(dir) {
            tracing::warn!("热句音频目录创建失败（本次不落盘）{}: {e}", dir.display());
            return false;
        }
        let path = dir.join(pcm_name(key));
        let tmp = dir.join(format!(
            ".tmp-{}-{:016x}",
            std::process::id(),
            next_tmp_seq()
        ));
        if let Err(e) = std::fs::write(&tmp, pcm) {
            tracing::warn!("热句音频写入失败（本次不落盘）{}: {e}", tmp.display());
            let _ = std::fs::remove_file(&tmp);
            return false;
        }
        if let Err(e) = std::fs::rename(&tmp, &path) {
            tracing::warn!("热句音频落位失败（本次不落盘）{}: {e}", path.display());
            let _ = std::fs::remove_file(&tmp);
            return false;
        }
        evict_overflow(dir);
        true
    }
}

/// 临时文件序号：同进程并发写不会撞名（不同进程靠 pid 区分）。
fn next_tmp_seq() -> u64 {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    SEQ.fetch_add(1, Ordering::Relaxed)
}

/// 落盘 PCM S16LE 字节 → 分帧器吃的 f32（口径与 `framer` 的 PCM 编码互逆）。
pub(crate) fn pcm_bytes_to_f32(pcm: &[u8]) -> Vec<f32> {
    pcm.chunks_exact(2)
        .map(|b| i16::from_le_bytes([b[0], b[1]]) as f32 / 32_767.0)
        .collect()
}

/// f32 样本 → 落盘 PCM S16LE 字节（钳位口径与 `framer` 的 PCM 编码一致）。
pub(crate) fn f32_to_pcm16_bytes(samples: &[f32]) -> Vec<u8> {
    samples
        .iter()
        .flat_map(|s| ((s.clamp(-1.0, 1.0) * 32_767.0) as i16).to_le_bytes())
        .collect()
}

/// 超出 [`MAX_FILES`] / [`MAX_TOTAL_BYTES`] 时按 mtime 删最旧（只在 `put` 之后调用，
/// 失败只记日志）。只看 `.pcm`——旧格式文件由 [`HotAudioStore::sweep_legacy`] 负责。
fn evict_overflow(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut files: Vec<(std::time::SystemTime, PathBuf, u64)> = entries
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.path()
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("pcm"))
        })
        .filter_map(|e| {
            let meta = e.metadata().ok()?;
            if !meta.is_file() {
                return None;
            }
            Some((meta.modified().ok()?, e.path(), meta.len()))
        })
        .collect();
    let mut bytes: u64 = files.iter().map(|(_, _, bytes)| *bytes).sum();
    let mut count = files.len();
    files.sort_by_key(|(t, _, _)| *t);
    for (_, path, size) in files {
        if count <= MAX_FILES && bytes <= MAX_TOTAL_BYTES {
            break;
        }
        match std::fs::remove_file(&path) {
            Ok(()) => {
                count -= 1;
                bytes = bytes.saturating_sub(size);
            }
            Err(e) => tracing::warn!("热句音频淘汰失败 {}: {e}", path.display()),
        }
    }
}

#[cfg(test)]
mod tests;
