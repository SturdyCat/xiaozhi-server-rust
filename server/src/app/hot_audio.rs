//! 所有生产 TTS 文本共用的有界持久化音频库（兼容既有 wakeup `.hot` 文件）。
//!
//! 身份覆盖引擎签名、音色、语速、文本与下行编码参数；只保存哈希和音频，
//! 不保存原始文本或凭据。内存 LRU 由 `TtsCache` 管理，磁盘 miss 重新流式合成。
//! 格式 v2 在原 v1 的 48 字节头部和帧正文后增加 FNV 校验和，仍能读取旧 v1 文件。
//! 头部依次为 magic[8]、version[u32]、signature_hash[u64]、speaker[i32]、speed_bits[u32]、
//! sr[u32]、frame_ms[u32]、text_hash[u64]、count[u32]；正文为 count × (len[u32]+payload)。
//! 数字均 LE；v2 末尾校验和[u64]覆盖前面的全部字节（非加密认证）。
//! 所有生产读写应通过 `TtsCache::{get_async,insert_async}` 在 blocking 池执行。
//! 写临时文件后原子 rename；失败降级不影响下行。磁盘同时限制文件数、总字节、
//! 单句字节与帧数，超限按写入时间淘汰（不是永久归档，重复文本可能再次合成）。
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

/// 文件魔数（8 字节，含版本号便于人眼识别）。
const MAGIC: &[u8; 8] = b"XZHOTAU1";
/// 格式版本。改格式必须递增；兼容版本 1，其余未知版本视为 miss 并删除。
const VERSION: u32 = 2;
/// 固定头部长度（字段顺序见模块文档）。
const HEADER_LEN: usize = 48;
/// 单帧长度上限（防御损坏/恶意文件把 `Vec` 撑爆；Opus 帧不会超过 8KB）。
const MAX_FRAME_LEN: u32 = 8 * 1024;
/// 全文本缓存的固定安全限制；后续如需可配置，统一从配置注册表接入。
pub const MAX_FILES: usize = 4096;
pub const MAX_TOTAL_BYTES: u64 = 512 * 1024 * 1024;
pub const MAX_ENTRY_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_FRAMES: usize = 16_384;
const MAX_FILE_BYTES: usize = HEADER_LEN + MAX_ENTRY_BYTES + 4 * MAX_FRAMES + 8;

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
    pub fn from_session(
        live: &Config,
        downlink_sr: u32,
        downlink_frame_ms: u32,
        text: &str,
    ) -> Self {
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
        fnv1a64(&self.header()[12..44])
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

    /// 读一条；命中返回**已编码的下行 Opus 帧负载**（不含二进制协议头）。
    ///
    /// 任何异常（文件不存在 / 头不匹配 / 长度非法 / IO 错误）都返回 `None` 并记日志——
    /// 读不出来只意味着"要重新合成一次"，**永远不该影响会话**。
    pub fn get(&self, key: &HotAudioKey) -> Option<Vec<Vec<u8>>> {
        self.get_identity(&HotAudioIdentity::from_key(key))
    }

    pub(super) fn get_identity(&self, identity: &HotAudioIdentity) -> Option<Vec<Vec<u8>>> {
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
        self.put_identity(&HotAudioIdentity::from_key(key), frames)
    }

    pub(super) fn put_identity(&self, identity: &HotAudioIdentity, frames: &[Vec<u8>]) -> bool {
        let Some(dir) = self.dir.as_ref() else {
            return false;
        };
        if frames.is_empty()
            || frames.len() > MAX_FRAMES
            || frames
                .iter()
                .any(|f| f.is_empty() || f.len() > MAX_FRAME_LEN as usize)
            || frames.iter().map(Vec::len).sum::<usize>() > MAX_ENTRY_BYTES
        {
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
        if let Err(e) = std::fs::write(&tmp, encode_identity(identity, frames)) {
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
#[cfg(test)]
fn encode(key: &HotAudioKey, frames: &[Vec<u8>]) -> Vec<u8> {
    encode_identity(&HotAudioIdentity::from_key(key), frames)
}

fn encode_identity(identity: &HotAudioIdentity, frames: &[Vec<u8>]) -> Vec<u8> {
    let mut h = identity.header;
    h[44..48].copy_from_slice(&(frames.len() as u32).to_le_bytes());
    let total: usize = HEADER_LEN + frames.iter().map(|f| 4 + f.len()).sum::<usize>();
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(&h);
    for f in frames {
        out.extend_from_slice(&(f.len() as u32).to_le_bytes());
        out.extend_from_slice(f);
    }
    let checksum = fnv1a64(&out);
    out.extend_from_slice(&checksum.to_le_bytes());
    out
}

/// 解码并**校验头部与期望键一致**。`Err` 的字符串只用于日志（不含凭据明文）。
fn decode(raw: &[u8], expect_header: &[u8; HEADER_LEN]) -> Result<Vec<Vec<u8>>, String> {
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
    if ver != VERSION && ver != 1 {
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
    if count == 0 || count > MAX_FRAMES {
        return Err("帧数为空或超过上限".into());
    }
    let mut frames = Vec::with_capacity(count);
    let mut payload_bytes = 0usize;
    let mut pos = HEADER_LEN;
    for i in 0..count {
        if pos + 4 > raw.len() {
            return Err(format!("第 {i} 帧长度字段越界"));
        }
        let len = u32::from_le_bytes(raw[pos..pos + 4].try_into().unwrap());
        pos += 4;
        if len == 0 || len > MAX_FRAME_LEN {
            return Err(format!("第 {i} 帧长度 {len} 超过上限 {MAX_FRAME_LEN}"));
        }
        let len = len as usize;
        payload_bytes += len;
        if payload_bytes > MAX_ENTRY_BYTES {
            return Err("帧负载总量超过上限".into());
        }
        if pos + len > raw.len() {
            return Err(format!("第 {i} 帧正文越界"));
        }
        frames.push(raw[pos..pos + len].to_vec());
        pos += len;
    }
    if ver == VERSION {
        if raw.len() != pos + 8 || raw[pos..] != fnv1a64(&raw[..pos]).to_le_bytes() {
            return Err("校验和不匹配或存在尾随数据".into());
        }
    } else if raw.len() != pos {
        return Err("旧格式存在尾随数据".into());
    }
    Ok(frames)
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
