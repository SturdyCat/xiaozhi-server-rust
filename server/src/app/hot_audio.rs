//! 持久化**热句**音频库：把「已编码的下行 Opus 帧」按音频身份落盘，进程重启后仍可命中。
//!
//! ## 为什么需要它（内存缓存不够）
//!
//! [`crate::app::tts_cache`] 已经按句缓存了**编码后的下行帧**，命中即零合成——但它活在
//! 进程内存里，**重启即失**。而唤醒问候语（「我在」）是全系统最高频的一句：设备每次唤醒
//! 都会请求它。于是"每次重启后的第一次唤醒"必然付一次完整合成（在线 TTS 是几百 ms 的
//! 短连接握手 + 下载；本地 Kokoro 更慢），这在容器滚动更新、调试期反复重启时尤其刺眼。
//!
//! 本模块把这一层补上：**同一句、同一音色**的成品音频落盘一次，之后每次唤醒都是
//! 一次读盘（约 2KB）→ 零 LLM、零 TTS。
//!
//! ## 与 `tts_cache` 的分工（刻意的，不要合并）
//!
//! | | [`crate::app::tts_cache::TtsCache`] | [`HotAudioStore`] |
//! |---|---|---|
//! | 存什么 | **任意**句子（LRU 容量按条目数） | **仅显式写入的热句**（调用方决定） |
//! | 存哪 | 进程内存 | 磁盘（+ 命中后暖进内存缓存） |
//! | 生命周期 | 进程 | 跨进程（容器重建仍在挂载卷上） |
//! | 容量 | `[tts].cache_entries`（默认 256） | 固定条目上限（见 [`MAX_FILES`]） |
//!
//! 分开的理由：内存缓存要"什么都能装"，持久化缓存必须"**有界且可预测**"——把任意句子
//! 都写盘会让一个正常聊天会话在磁盘上留下几百个音频文件，且用户无法预期何时被写。
//! 因此落盘是**显式**的：只有调用方（唤醒问候路径）点名要存的热句才会进这里。
//!
//! ## 键：两条，各司其职
//!
//! - [`HotAudioKey::mem_key`]：与预取路径**逐字节同口径**的内存键（`TtsCache::make_key`），
//!   用于暖进内存缓存后让既有下行链路直接命中；
//! - [`HotAudioKey::file_id`]：**跨进程、跨 Rust 版本稳定**的文件名（FNV-1a 64）。
//!
//! 为什么文件名不能用内存键：`TtsCache::make_key` 走 `std::collections::hash_map::DefaultHasher`，
//! 它**不保证跨 Rust 版本稳定**（标准库明说 hash 算法可变）。用它当文件名，工具链一升级
//! 键就全变——不至于出错（全 miss → 重新合成），但磁盘上会留下永远读不到的垃圾文件。
//! FNV-1a 是本文件里 20 行实现的，没有新依赖，且**永久稳定**。
//!
//! ## 凭据绝不落盘
//!
//! 缓存键含**引擎签名**（讯飞密钥在里面）。落盘只写它的 **FNV 哈希**，不写明文；
//! 校验时同样只比哈希。文件里除音频外只有哈希与几个数字。
//!
//! ## 格式（自描述 + 自校验）
//!
//! ```text
//! offset  size  内容
//! 0       8     magic "XZHOTAU1"
//! 8       4     version (u32 LE) = 1
//! 12      8     引擎签名的 FNV-1a 64（LE）
//! 20      4     speaker (i32 LE)
//! 24      4     speed（f32 位模式，u32 LE——用位模式而非文本，避免 "1.0"/"1" 解析差异）
//! 28      4     downlink_sr (u32 LE)
//! 32      4     downlink_frame_ms (u32 LE)
//! 36      8     文本的 FNV-1a 64（LE）
//! 44      4     帧数 (u32 LE)
//! 48      ...   帧数 × (u32 LE 长度 + 该帧 Opus 负载)
//! ```
//!
//! 头部带全参数的理由：文件可能属于**上一个配置**（换了音色/采样率/帧长）。读出后先比对
//! 头部，任何一项不同即视为 **miss**（并删除该文件）——而不是"凑合播一段可能是别的音色的
//! 音频"。这与 `tts_cache` 的构造性保证同源：键不同 → 绝不串用。
//!
//! ## 已知边界
//!
//! - 写盘是**同步**的：单文件约 1–3KB，量级在微秒，不值得为它引入 `spawn_blocking`
//!   （且调用点在会话任务里，一次唤醒只发生一次）。若将来落盘的热句变大/变多，需改。
//! - 目录不可写（只读挂载/权限）只记 warn 并降级为**纯内存行为**，绝不影响会话。

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::app::tts_cache::TtsCache;
use crate::config::Config;

/// 文件魔数（8 字节，含版本号便于人眼识别）。
const MAGIC: &[u8; 8] = b"XZHOTAU1";
/// 格式版本。**改格式必须递增**（旧文件按版本不匹配处理为 miss 并删除）。
const VERSION: u32 = 1;
/// 头部长度（见模块文档的偏移表）。
const HEADER_LEN: usize = 48;
/// 单帧长度上限（防御损坏/恶意文件把 `Vec` 撑爆；Opus 帧不会超过 8KB）。
const MAX_FRAME_LEN: u32 = 8 * 1024;
/// 文件数上限：超出后按 mtime 删最旧。
///
/// 取值理由：热句是「唤醒问候语」这类**极少数**句子，条目数 = 不同音色数 × 不同问候语数，
/// 现实中个位数。64 给了充足的余量（用户反复改问候语/换音色也不会立刻顶到上限），
/// 同时把最坏情况钉死在"几十个 2KB 文件"的量级。
pub const MAX_FILES: usize = 64;

/// 一句热句音频的**身份**：影响编码结果的一切参数。
///
/// 构造入口只有 [`HotAudioKey::from_session`] 一个——预取合成路径与唤醒问候路径都走它，
/// 避免两处各写一遍参数顺序导致"键永远不相等、缓存永远不命中"（这类 bug 不会报错，只会
/// 表现为"缓存没用"，最难查）。
///
/// ⚠️ 不派生 `Eq`（`speed` 是 `f32`）；本结构只在同进程内做校验，`PartialEq` 足够。
#[derive(Debug, Clone, PartialEq)]
pub struct HotAudioKey {
    /// 引擎签名（**含凭据**，只在内存里流转；落盘只写其 FNV）。
    pub engine_sig: String,
    pub speaker: i32,
    pub speed: f32,
    pub downlink_sr: u32,
    pub downlink_frame_ms: u32,
    pub text: String,
}

impl HotAudioKey {
    /// **唯一权威**的键构造口径。
    ///
    /// `live` = 会话级最新配置（`Engines::live_config()`），因此引擎/音色/语速的热切换
    /// 天然反映在键里：换了音色 → 键不同 → 不会命中上一个音色的音频。
    pub fn from_session(live: &Config, downlink_sr: u32, downlink_frame_ms: u32, text: &str) -> Self {
        Self {
            engine_sig: crate::plugins::registry::signature_of(
                live,
                crate::plugins::registry::Capability::Tts,
            ),
            speaker: live.tts.speaker,
            speed: live.tts.speed,
            downlink_sr,
            downlink_frame_ms,
            text: text.to_string(),
        }
    }

    /// 内存缓存键：与 [`crate::app::prefetch`] 的合成路径**完全同口径**。
    ///
    /// 命中它 = 走既有下行链路的零合成路径（预取 worker 直接投递已编码帧）。
    pub fn mem_key(&self) -> u64 {
        TtsCache::make_key(
            &self.engine_sig,
            self.speaker,
            self.speed,
            self.downlink_sr,
            self.downlink_frame_ms,
            &self.text,
        )
    }

    /// 落盘文件名（不含扩展名）：跨进程、跨 Rust 版本稳定的 FNV-1a 64。
    ///
    /// 输入是各字段的**稳定投影**（签名与文本先各自 FNV 一次，再连同标量打包哈希），
    /// 因此文件里外都不会出现凭据明文。
    pub fn file_id(&self) -> u64 {
        let mut buf = Vec::with_capacity(48);
        buf.extend_from_slice(&fnv1a64(self.engine_sig.as_bytes()).to_le_bytes());
        buf.extend_from_slice(&self.speaker.to_le_bytes());
        buf.extend_from_slice(&self.speed.to_bits().to_le_bytes());
        buf.extend_from_slice(&self.downlink_sr.to_le_bytes());
        buf.extend_from_slice(&self.downlink_frame_ms.to_le_bytes());
        buf.extend_from_slice(&fnv1a64(self.text.as_bytes()).to_le_bytes());
        fnv1a64(&buf)
    }

    /// 自校验头部（写入时编码、读出时比对）。
    fn header(&self) -> [u8; HEADER_LEN] {
        let mut h = [0u8; HEADER_LEN];
        h[0..8].copy_from_slice(MAGIC);
        h[8..12].copy_from_slice(&VERSION.to_le_bytes());
        h[12..20].copy_from_slice(&fnv1a64(self.engine_sig.as_bytes()).to_le_bytes());
        h[20..24].copy_from_slice(&self.speaker.to_le_bytes());
        h[24..28].copy_from_slice(&self.speed.to_bits().to_le_bytes());
        h[28..32].copy_from_slice(&self.downlink_sr.to_le_bytes());
        h[32..36].copy_from_slice(&self.downlink_frame_ms.to_le_bytes());
        h[36..44].copy_from_slice(&fnv1a64(self.text.as_bytes()).to_le_bytes());
        // 帧数由调用方在写正文时填（h[44..48]）
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

/// 持久化热句音频库。`dir == None` = 纯内存降级（未加载配置文件时）。
pub struct HotAudioStore {
    dir: Option<PathBuf>,
}

impl HotAudioStore {
    /// 按目录构造；`None` = 不持久化（`get` 恒 `None`、`put` 恒 `false`）。
    pub fn new(dir: Option<PathBuf>) -> Self {
        Self { dir }
    }

    /// 是否真的在落盘（管理界面/启动日志据此说明"重启后仍在"）。
    pub fn is_persistent(&self) -> bool {
        self.dir.is_some()
    }

    /// 数据目录（观测/测试用）。
    pub fn dir(&self) -> Option<&Path> {
        self.dir.as_deref()
    }

    /// 读一条；命中返回**已编码的下行 Opus 帧负载**（不含二进制协议头）。
    ///
    /// 任何异常（文件不存在 / 头不匹配 / 长度非法 / IO 错误）都返回 `None` 并记日志——
    /// 读不出来只意味着"要重新合成一次"，**永远不该影响会话**。
    pub fn get(&self, key: &HotAudioKey) -> Option<Vec<Vec<u8>>> {
        let dir = self.dir.as_ref()?;
        let path = dir.join(format!("{:016x}.hot", key.file_id()));
        let raw = match std::fs::read(&path) {
            Ok(v) => v,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
            Err(e) => {
                tracing::warn!("热句音频读取失败（将重新合成）{}: {e}", path.display());
                return None;
            }
        };
        match decode(&raw, &key.header()) {
            Ok(frames) if !frames.is_empty() => Some(frames),
            Ok(_) => None,
            Err(why) => {
                // 头不匹配 = 文件属于别的配置（换音色/采样率/帧长）：删掉，避免长期占位。
                tracing::debug!("热句音频作废（{why}），删除 {}: {why}", path.display());
                let _ = std::fs::remove_file(&path);
                None
            }
        }
    }

    /// 写一条（覆盖同名）。返回是否真的落盘。
    ///
    /// 先写临时文件再 `rename`：读者**永远看不到半截文件**（唤醒可能在写入的同一瞬间发生）。
    pub fn put(&self, key: &HotAudioKey, frames: &[Vec<u8>]) -> bool {
        let Some(dir) = self.dir.as_ref() else {
            return false;
        };
        if frames.is_empty() {
            return false;
        }
        if let Err(e) = std::fs::create_dir_all(dir) {
            tracing::warn!("热句音频目录创建失败（本次不落盘）{}: {e}", dir.display());
            return false;
        }
        let path = dir.join(format!("{:016x}.hot", key.file_id()));
        let tmp = dir.join(format!(
            ".tmp-{}-{:016x}",
            std::process::id(),
            next_tmp_seq()
        ));
        if let Err(e) = std::fs::write(&tmp, encode(key, frames)) {
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

/// 编码为落盘字节（头部帧数字段在此填上）。
fn encode(key: &HotAudioKey, frames: &[Vec<u8>]) -> Vec<u8> {
    let mut h = key.header();
    h[44..48].copy_from_slice(&(frames.len() as u32).to_le_bytes());
    let total: usize = HEADER_LEN + frames.iter().map(|f| 4 + f.len()).sum::<usize>();
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(&h);
    for f in frames {
        out.extend_from_slice(&(f.len() as u32).to_le_bytes());
        out.extend_from_slice(f);
    }
    out
}

/// 解码并**校验头部与期望键一致**。`Err` 的字符串只用于日志（不含凭据明文）。
fn decode(raw: &[u8], expect_header: &[u8; HEADER_LEN]) -> Result<Vec<Vec<u8>>, String> {
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
    // 逐项比对**全部**影响编码结果的参数；任何一项不同都作废（绝不"凑合播"）。
    for (name, range) in [
        ("引擎签名", 12..20usize),
        ("speaker", 20..24),
        ("speed", 24..28),
        ("下行采样率", 28..32),
        ("帧长", 32..36),
        ("文本", 36..44),
    ] {
        if raw[range.clone()] != expect_header[range] {
            return Err(format!("{name} 与当前配置不一致"));
        }
    }
    let count = u32::from_le_bytes(raw[44..48].try_into().unwrap()) as usize;
    let mut frames = Vec::with_capacity(count.min(4096));
    let mut pos = HEADER_LEN;
    for i in 0..count {
        if pos + 4 > raw.len() {
            return Err(format!("第 {i} 帧长度字段越界"));
        }
        let len = u32::from_le_bytes(raw[pos..pos + 4].try_into().unwrap());
        pos += 4;
        if len > MAX_FRAME_LEN {
            return Err(format!("第 {i} 帧长度 {len} 超过上限 {MAX_FRAME_LEN}"));
        }
        let len = len as usize;
        if pos + len > raw.len() {
            return Err(format!("第 {i} 帧正文越界"));
        }
        frames.push(raw[pos..pos + len].to_vec());
        pos += len;
    }
    Ok(frames)
}

/// 超出 [`MAX_FILES`] 时按 mtime 删最旧（只在 `put` 之后调用，失败只记日志）。
fn evict_overflow(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut files: Vec<(std::time::SystemTime, PathBuf)> = entries
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.path()
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("hot"))
        })
        .filter_map(|e| {
            let mtime = e.metadata().ok()?.modified().ok()?;
            Some((mtime, e.path()))
        })
        .collect();
    if files.len() <= MAX_FILES {
        return;
    }
    files.sort_by_key(|(t, _)| *t);
    let remove = files.len() - MAX_FILES;
    for (_, path) in files.into_iter().take(remove) {
        if let Err(e) = std::fs::remove_file(&path) {
            tracing::warn!("热句音频淘汰失败 {}: {e}", path.display());
        }
    }
}

#[cfg(test)]
mod tests;
