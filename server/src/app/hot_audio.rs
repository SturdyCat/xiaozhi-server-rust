//! 所有生产 TTS 文本共用的有界持久化音频库（兼容既有 wakeup `.hot` 文件名口径）。
//!
//! **磁盘存的是 TTS 引擎的原始产物**：24kHz 单声道 S16LE PCM 整句（格式 v3），不是
//! 已编码的下行 Opus 帧——下行参数（采样率/帧长/编解码档）只属于"投递"，不属于"合成
//! 成品"。换设备档位、改 `[audio]` 帧长都不再作废磁盘缓存；命中后在读路径按会话实际
//! 下行格式重采样 + 编码（与流式合成走同一 [`crate::app::downlink::framer`] 链路）。
//!
//! 身份覆盖引擎签名、音色、语速与文本；只保存哈希和音频，不保存原始文本或凭据。
//! 内存 LRU 由 `TtsCache` 管理（内存里是**已编码下行帧**，键含下行参数）；磁盘 miss
//! 重新流式合成。格式 v3 头部依次为 magic[8]、version[u32]、signature_hash[u64]、
//! speaker[i32]、speed_bits[u32]、sr[u32]=24000（固定存储采样率）、frame_ms[u32]=0
//!（PCM 无帧概念，恒 0）、text_hash[u64]、pcm_bytes[u32]；正文为 PCM 字节。
//! 数字均 LE；末尾 FNV 校验和[u64]覆盖前面的全部字节（非加密认证）。
//! 所有生产读写应通过 `TtsCache::{get_async,insert_async}` 在 blocking 池执行。
//! 写临时文件后原子 rename；失败降级不影响下行。磁盘同时限制文件数、总字节、
//! 单句字节，超限按写入时间淘汰（不是永久归档，重复文本可能再次合成）。
//! 测试台强制真合成不复用缓存；AIUI 云闭环已在云端合成，不能靠本地缓存撤销。
//!
//! **配置固定话术的管理槽位**（[`Self::put_managed`] / [`Self::sync_managed`]，实现与
//! 槽位常量见 [`managed`]）：欢迎语 / 问候语池 / 技能应答在落盘时登记 `managed.json`，
//! 启动时按当前配置文本做内容哈希校验——文案改了/清空的旧音频直接删除，损坏成品删待
//! 重合成。普通生产文本不经登记，仍走通用缓存与溢出淘汰。

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

/// 文件魔数（8 字节，人眼可识别；v3 起为 PCM 专属，旧 Opus 帧文件天然失配被清）。
const MAGIC: &[u8; 8] = b"XZOTPCM1";
/// 格式版本。改格式必须递增；不读旧版本（v2 及以前是 Opus 帧，无法廉价转回 PCM，
/// 失配即删，首次使用重新合成一次即可）。
const VERSION: u32 = 3;
/// 固定头部长度（字段顺序见模块文档）。
const HEADER_LEN: usize = 48;
/// 磁盘固定存储采样率：TTS 引擎产物统一为 24k（kokoro/讯飞/内置全是 24k），
/// 写入时经重采样归一，读取时按会话实际下行格式再重采样。
pub const PCM_SAMPLE_RATE: u32 = 24_000;
/// 全文本缓存的固定安全限制；后续如需可配置，统一从配置注册表接入。
pub const MAX_FILES: usize = 4096;
pub const MAX_TOTAL_BYTES: u64 = 512 * 1024 * 1024;
pub const MAX_ENTRY_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_FRAMES: usize = 16_384;
const MAX_FILE_BYTES: usize = HEADER_LEN + MAX_ENTRY_BYTES + 8;

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
        fnv1a64(&self.header()[12..44])
    }

    /// 自校验头部（写入时编码、读出时比对）。`sr`/`frame_ms` 是磁盘格式常量，
    /// 不从任何会话配置来——这是"下行参数不参与磁盘身份"的落点。
    fn header(&self) -> [u8; HEADER_LEN] {
        let mut h = [0u8; HEADER_LEN];
        h[0..8].copy_from_slice(MAGIC);
        h[8..12].copy_from_slice(&VERSION.to_le_bytes());
        h[12..20].copy_from_slice(&fnv1a64(self.engine_sig.as_bytes()).to_le_bytes());
        h[20..24].copy_from_slice(&self.speaker.to_le_bytes());
        h[24..28].copy_from_slice(&self.speed.to_bits().to_le_bytes());
        h[28..32].copy_from_slice(&PCM_SAMPLE_RATE.to_le_bytes());
        h[32..36].copy_from_slice(&0u32.to_le_bytes());
        h[36..44].copy_from_slice(&fnv1a64(self.text.as_bytes()).to_le_bytes());
        // 正文长度由调用方在写正文时填（h[44..48]）
        h
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

/// 固定长度的身份投影：登记表不保留原始文本和含凭据的引擎签名。
#[derive(Clone)]
pub(super) struct HotAudioIdentity {
    header: [u8; HEADER_LEN],
    file_id: u64,
}

impl HotAudioIdentity {
    pub(super) fn from_key(key: &HotAudioKey) -> Self {
        Self {
            header: key.header(),
            file_id: key.file_id(),
        }
    }
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

    /// 读一条；命中返回**整句 PCM S16LE 字节**（[`PCM_SAMPLE_RATE`] 单声道）。
    ///
    /// 任何异常（文件不存在 / 头不匹配 / 长度非法 / IO 错误）都返回 `None` 并记日志——
    /// 读不出来只意味着"要重新合成一次"，**永远不该影响会话**。
    pub fn get(&self, key: &HotAudioKey) -> Option<Vec<u8>> {
        self.get_identity(&HotAudioIdentity::from_key(key))
    }

    pub(super) fn get_identity(&self, identity: &HotAudioIdentity) -> Option<Vec<u8>> {
        let dir = self.dir.as_ref()?;
        // 防止损坏文件读者在另一个 writer 原子替换后误删刚生成的有效成品。
        let _guard = self.write_lock.lock().unwrap_or_else(|e| e.into_inner());
        let path = dir.join(format!("{:016x}.hot", identity.file_id));
        let read = || -> std::io::Result<Vec<u8>> {
            let mut raw = Vec::new();
            std::fs::File::open(&path)?
                .take((MAX_FILE_BYTES + 1) as u64)
                .read_to_end(&mut raw)?;
            Ok(raw)
        };
        let raw = match read() {
            Ok(v) => v,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
            Err(e) => {
                tracing::warn!("热句音频读取失败（将重新合成）{}: {e}", path.display());
                return None;
            }
        };
        match decode(&raw, &identity.header) {
            Ok(pcm) if !pcm.is_empty() => Some(pcm),
            Ok(_) => None,
            Err(why) => {
                // 头不匹配 = 文件属于别的配置（换音色/引擎/文案，或旧 Opus 格式）：
                // 删掉，避免长期占位。
                tracing::debug!("热句音频作废（{why}），删除 {}: {why}", path.display());
                let _ = std::fs::remove_file(&path);
                None
            }
        }
    }

    /// 写一条（覆盖同名）。`pcm` = 整句 S16LE 字节（[`PCM_SAMPLE_RATE`] 单声道）。
    /// 返回是否真的落盘。
    ///
    /// 先写临时文件再 `rename`：读者**永远看不到半截文件**（唤醒可能在写入的同一瞬间发生）。
    pub fn put(&self, key: &HotAudioKey, pcm: &[u8]) -> bool {
        self.put_identity(&HotAudioIdentity::from_key(key), pcm)
    }

    pub(super) fn put_identity(&self, identity: &HotAudioIdentity, pcm: &[u8]) -> bool {
        let Some(dir) = self.dir.as_ref() else {
            return false;
        };
        if pcm.is_empty() || pcm.len() % 2 != 0 || pcm.len() > MAX_ENTRY_BYTES {
            return false;
        }
        let _guard = self.write_lock.lock().unwrap_or_else(|e| e.into_inner());
        if let Err(e) = std::fs::create_dir_all(dir) {
            tracing::warn!("热句音频目录创建失败（本次不落盘）{}: {e}", dir.display());
            return false;
        }
        let path = dir.join(format!("{:016x}.hot", identity.file_id));
        let tmp = dir.join(format!(
            ".tmp-{}-{:016x}",
            std::process::id(),
            next_tmp_seq()
        ));
        if let Err(e) = std::fs::write(&tmp, encode_identity(identity, pcm)) {
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

/// 编码为落盘字节（头部 PCM 长度字段在此填上）。
#[cfg(test)]
fn encode(key: &HotAudioKey, pcm: &[u8]) -> Vec<u8> {
    encode_identity(&HotAudioIdentity::from_key(key), pcm)
}

fn encode_identity(identity: &HotAudioIdentity, pcm: &[u8]) -> Vec<u8> {
    let mut h = identity.header;
    h[44..48].copy_from_slice(&(pcm.len() as u32).to_le_bytes());
    let mut out = Vec::with_capacity(HEADER_LEN + pcm.len() + 8);
    out.extend_from_slice(&h);
    out.extend_from_slice(pcm);
    let checksum = fnv1a64(&out);
    out.extend_from_slice(&checksum.to_le_bytes());
    out
}

/// 解码并**校验头部与期望键一致**。`Err` 的字符串只用于日志（不含凭据明文）。
fn decode(raw: &[u8], expect_header: &[u8; HEADER_LEN]) -> Result<Vec<u8>, String> {
    if raw.len() > MAX_FILE_BYTES {
        return Err("文件超过单句上限".into());
    }
    if raw.len() < HEADER_LEN {
        return Err(format!("文件过短（{} < {HEADER_LEN} 字节）", raw.len()));
    }
    if &raw[0..8] != MAGIC {
        return Err("魔数不匹配".to_string());
    }
    let ver = u32::from_le_bytes(raw[8..12].try_into().unwrap());
    if ver != VERSION {
        return Err(format!("格式版本 {ver} ≠ {VERSION}"));
    }
    // 逐项比对**全部**身份字段（签名/音色/语速/文本）与格式常量（采样率/帧长）；
    // 任何一项不同都作废（绝不"凑合播"）。
    for (name, range) in [
        ("引擎签名", 12..20usize),
        ("speaker", 20..24),
        ("speed", 24..28),
        ("存储采样率", 28..32),
        ("帧长", 32..36),
        ("文本", 36..44),
    ] {
        if raw[range.clone()] != expect_header[range] {
            return Err(format!("{name} 与当前配置不一致"));
        }
    }
    let pcm_len = u32::from_le_bytes(raw[44..48].try_into().unwrap()) as usize;
    if pcm_len == 0 || pcm_len % 2 != 0 || pcm_len > MAX_ENTRY_BYTES {
        return Err("PCM 长度为空、非整样本或超过上限".into());
    }
    if raw.len() != HEADER_LEN + pcm_len + 8 {
        return Err("正文长度与头部声明不一致".into());
    }
    let pcm = raw[HEADER_LEN..HEADER_LEN + pcm_len].to_vec();
    if raw[HEADER_LEN + pcm_len..] != fnv1a64(&raw[..HEADER_LEN + pcm_len]).to_le_bytes() {
        return Err("校验和不匹配或存在尾随数据".into());
    }
    Ok(pcm)
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

/// 超出 [`MAX_FILES`] 时按 mtime 删最旧（只在 `put` 之后调用，失败只记日志）。
fn evict_overflow(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut files: Vec<(std::time::SystemTime, PathBuf, u64)> = entries
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.path()
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("hot"))
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

mod managed;

pub use managed::{ManagedText, SLOT_ECHOMUSE_WELCOME, SLOT_SKILL_REPLY, SLOT_WAKEUP_GREETING};

#[cfg(test)]
mod tests;
