//! 「配置固定话术」的管理槽位：落盘登记（[`HotAudioStore::put_managed`]）与启动校验
//! （[`HotAudioStore::sync_managed`]）。
//!
//! ## 为什么需要登记表
//!
//! `.hot` 文件名 = 全身份 FNV（含**文本**哈希），文案一改，旧文件就**永远不会再被查找**
//! ——通用淘汰只在超容量（4096 文件 / 512MiB）时发生，家用部署等于永久占位；启动时
//! 也没有任何校验。本模块按「配置里**当前声明**的固定文本」收敛一份侧车登记表
//! `managed.json`（与 `.hot` 同目录）：
//!
//! - **写路径**：固定话术（欢迎语 / 问候语池 / 技能应答含告别语）首次合成落盘时，把
//!   条目（文件 id + 哈希投影）登记进槽位；
//! - **启动校验**：条目的文本哈希不在当前配置清单 → 删文件删条目（文案改了 / 清空 /
//!   技能删了）；文本未变的成品读盘**解码校验**，损坏即删（下次使用重新合成）。
//!
//! ## 刻意的口径
//!
//! - **只按文本哈希判过时**：设备 overlay 允许逐设备音色/语速（`config/overlay.rs`），
//!   同槽位多音色成品都是合法缓存，不能按全局配置误删；换引擎/音色的隔离由缓存键
//!   天然完成（miss → 重合成），不在清理职责内。
//! - **登记表只存哈希与标量**：不含文本明文与凭据（与 `.hot` 文件同一口径）。
//! - 热改文案（`[wakeup]` 属 NextSession）的清理发生在**下次启动**——校验锚定启动，
//!   不为运行中配置引入文件监听。
//! - 清理失败（只读盘 / 权限）只告警：清理是卫生措施，绝不妨碍音频复用本身。

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::{decode, fnv1a64, HotAudioKey, HotAudioStore, HEADER_LEN, MAGIC, VERSION};

/// EchoMuse 连接欢迎语（`[echomuse].welcome`）。
pub const SLOT_ECHOMUSE_WELCOME: &str = "echomuse.welcome";

/// 唤醒问候语池（`[wakeup].greeting`，逐句一条）。
pub const SLOT_WAKEUP_GREETING: &str = "wakeup.greeting";

/// 技能应答 / 告别语（`[command].reply` + `[nlu]` 技能应答；清单从编译后目录
/// [`crate::plugins::skill::SkillRegistry::replies`] 枚举，与运行时应答同源）。
pub const SLOT_SKILL_REPLY: &str = "skill.reply";

/// 启动校验输入：槽位名 + 当前配置文本（同一槽位多条 = 多句池逐句登记）。
#[derive(Debug, Clone)]
pub struct ManagedText {
    pub slot: String,
    pub text: String,
}

/// 登记条目：一份成品音频的身份投影（只有哈希与标量，无文本明文/凭据）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct ManagedEntry {
    /// `.hot` 文件名（16hex，不含扩展名）。
    file: String,
    /// FNV(文本) 16hex。
    text: String,
    /// FNV(引擎签名) 16hex。
    sig: String,
    speaker: i32,
    /// f32 bits（避免 JSON 浮点精度丢身份）。
    speed: u32,
    sr: u32,
    frame_ms: u32,
}

/// 侧车登记表（`managed.json`）。`BTreeMap` 让落盘形态稳定（JSON 键序确定）。
#[derive(Debug, Default, Serialize, Deserialize)]
struct ManagedRegistry {
    #[serde(default)]
    slots: BTreeMap<String, Vec<ManagedEntry>>,
}

/// 从缓存键提取登记条目。
fn entry_of(key: &HotAudioKey) -> ManagedEntry {
    ManagedEntry {
        file: format!("{:016x}", key.file_id()),
        text: format!("{:016x}", fnv1a64(key.text.as_bytes())),
        sig: format!("{:016x}", fnv1a64(key.engine_sig.as_bytes())),
        speaker: key.speaker,
        speed: key.speed.to_bits(),
        sr: super::PCM_SAMPLE_RATE,
        frame_ms: 0,
    }
}

/// 16hex → u64；登记表由本进程写出，格式可信，防御性解析失败按 0 处理
///（不可能匹配任何真实文本哈希 → 等价"过时"，下次启动被清）。
fn parse_hex16(s: &str) -> u64 {
    u64::from_str_radix(s, 16).unwrap_or(0)
}

/// 从登记条目重建自校验头部：[`decode`] 比对 12..44 区间（签名/音色/语速/采样率/
/// 帧长/文本哈希）+ MAGIC；版本号在 `decode` 内单独接受 v1/v2。
fn header_of_entry(e: &ManagedEntry) -> [u8; HEADER_LEN] {
    let mut h = [0u8; HEADER_LEN];
    h[0..8].copy_from_slice(MAGIC);
    h[8..12].copy_from_slice(&VERSION.to_le_bytes());
    h[12..20].copy_from_slice(&parse_hex16(&e.sig).to_le_bytes());
    h[20..24].copy_from_slice(&e.speaker.to_le_bytes());
    h[24..28].copy_from_slice(&e.speed.to_le_bytes());
    h[28..32].copy_from_slice(&e.sr.to_le_bytes());
    h[32..36].copy_from_slice(&e.frame_ms.to_le_bytes());
    h[36..44].copy_from_slice(&parse_hex16(&e.text).to_le_bytes());
    h
}

impl HotAudioStore {
    fn registry_path(dir: &Path) -> PathBuf {
        dir.join("managed.json")
    }

    fn load_registry(dir: &Path) -> ManagedRegistry {
        let Ok(raw) = std::fs::read(Self::registry_path(dir)) else {
            return ManagedRegistry::default();
        };
        serde_json::from_slice(&raw).unwrap_or_else(|e| {
            tracing::warn!(
                "热句音频登记表损坏，按空表重建（音频复用不受影响，仅陈旧清理失效一次）: {e}"
            );
            ManagedRegistry::default()
        })
    }

    /// 原子写登记表（与 `.hot` 同口径：tmp + rename，读者永远看不到半截文件）。
    fn save_registry(dir: &Path, reg: &ManagedRegistry) -> std::io::Result<()> {
        std::fs::create_dir_all(dir)?;
        let tmp = dir.join(format!(".tmp-managed-{}", std::process::id()));
        let bytes = serde_json::to_vec_pretty(reg).expect("ManagedRegistry 序列化不可能失败");
        std::fs::write(&tmp, bytes)?;
        match std::fs::rename(&tmp, Self::registry_path(dir)) {
            Ok(()) => Ok(()),
            Err(e) => {
                let _ = std::fs::remove_file(&tmp);
                Err(e)
            }
        }
    }

    /// 管理槽位写盘：与 [`HotAudioStore::put`] 相同的原子落盘（`pcm` = 整句 S16LE
    /// 字节），并把成品**登记**进 `managed.json`（同一条目幂等，不重复；逐设备音色
    /// 各自的条目并存）。
    ///
    /// 返回值与 `put` 同义（是否落盘）；登记失败只告警——音频复用不受影响，仅该项
    /// 旧文案的启动清理失效。
    pub fn put_managed(&self, slot: &str, key: &HotAudioKey, pcm: &[u8]) -> bool {
        if !self.put(key, pcm) {
            return false;
        }
        let Some(dir) = self.dir.as_ref() else {
            return true; // 纯内存降级：put 恒 false，实际到不了这里
        };
        let entry = entry_of(key);
        let _guard = self.write_lock.lock().unwrap_or_else(|e| e.into_inner());
        let mut reg = Self::load_registry(dir);
        let list = reg.slots.entry(slot.to_string()).or_default();
        if !list.contains(&entry) {
            list.push(entry);
        }
        if let Err(e) = Self::save_registry(dir, &reg) {
            tracing::warn!("热句音频登记表写入失败（{slot} 的陈旧清理将失效）: {e}");
        }
        true
    }

    /// 启动校验（`main.rs` 在监听前调用）：按当前配置的固定话术清单收敛登记表与磁盘。
    /// 返回一行人读摘要（进启动日志）。
    ///
    /// 规则（对齐"配置固定语音一律落盘 + 内容哈希启动校验 + 改文案清旧"）：
    /// ① 条目文本哈希不在当前清单 → **过时**：删文件、删条目；
    /// ② 槽位整体不在清单（文案清空 / 技能删除）→ 整槽清理；
    /// ③ 保留条目读盘解码校验，损坏 → 删文件、删条目（下次使用重新合成落盘）；
    /// ④ 当前清单里尚无音频的文本不在此处理——首次使用时合成并经 `put_managed` 登记。
    pub fn sync_managed(&self, managed: &[ManagedText]) -> String {
        let Some(dir) = self.dir.as_ref() else {
            return "未持久化，跳过".into();
        };
        // 当前配置清单：槽位 → 文本哈希集合（trim 口径与问候池/欢迎语/应答一致）。
        let mut current: BTreeMap<&str, BTreeSet<u64>> = BTreeMap::new();
        for m in managed {
            let text = m.text.trim();
            if !text.is_empty() {
                current
                    .entry(m.slot.as_str())
                    .or_default()
                    .insert(fnv1a64(text.as_bytes()));
            }
        }
        let _guard = self.write_lock.lock().unwrap_or_else(|e| e.into_inner());
        let mut reg = Self::load_registry(dir);
        let mut stale = 0usize;
        let mut corrupt = 0usize;
        let mut kept = 0usize;
        let mut dirty = false;
        let slots = std::mem::take(&mut reg.slots);
        for (slot, list) in slots {
            let texts = current.get(slot.as_str());
            let mut out = Vec::with_capacity(list.len());
            for e in list {
                let keep = texts.is_some_and(|s| s.contains(&parse_hex16(&e.text)));
                let path = dir.join(format!("{}.hot", e.file));
                let valid = keep
                    && std::fs::read(&path).ok().is_some_and(|raw| {
                        decode(&raw, &header_of_entry(&e)).is_ok_and(|f| !f.is_empty())
                    });
                if valid {
                    kept += 1;
                    out.push(e);
                    continue;
                }
                dirty = true;
                if keep {
                    corrupt += 1;
                } else {
                    stale += 1;
                }
                if let Err(err) = std::fs::remove_file(&path) {
                    if err.kind() != std::io::ErrorKind::NotFound {
                        tracing::warn!("热句音频清理失败 {}: {err}", path.display());
                    }
                }
            }
            if !out.is_empty() {
                reg.slots.insert(slot, out);
            }
        }
        if dirty {
            if let Err(e) = Self::save_registry(dir, &reg) {
                tracing::warn!("热句音频登记表回写失败: {e}");
            }
        }
        format!("{kept} 条有效，清理 {stale} 条过时 / {corrupt} 条损坏")
    }
}
