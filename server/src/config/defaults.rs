//! 标准行为默认值的**版本化对齐**（存量配置迁移）。
//!
//! ## 为什么需要本模块
//!
//! 配置真源是 SQLite 配置库：首启 bootstrap 会把**当时**的 `Config::default()` 整份
//! 序列化进 `cfg_documents`。此后即便代码把某个「标准行为」的默认值从 `false` 改成
//! `true`，老库里存的仍是**显式** `false`——serde 的 `default = "..."` 只对"键缺失"
//! 生效，对"键存在且为 false"永远不生效。
//!
//! 表现就是用户报的那类问题：重装系统 / 重建容器时数据卷（`./server-data:/data`）
//! 被保留下来 → 库还是老库 → 唤醒问候语「我在」等标准行为**依旧是关的**，每个部署都
//! 得手动开一遍。
//!
//! ## 语义（对齐 Rails 的 `load_defaults` / Vue 的 compat 配置思路）
//!
//! 每个库记录一个 `defaults_version`。启动时若库内版本 < [`CURRENT_DEFAULTS_VERSION`]，
//! 就按 [`BATCHES`] 里声明的变更逐条对齐，规则只有三条：
//!
//! 1. **键缺失（或为 `null`）** → 补成新默认值（`filled`）；
//! 2. **值等于旧默认值** → 视为"从来没被用户动过"，对齐到新默认值（`changed`）；
//! 3. **值不等于旧默认值** → 用户显式自定义，**原样保留**（`kept`），只记日志。
//!
//! 三条规则刻意不含"启发式猜测"：库里没有保存"谁改的"这种信息，把 `false` 一律当
//! 用户意图就永远无法完成默认值切换，把 `false` 一律当旧默认又会踩用户的选择。
//! 用"是否等于旧默认"作为判据是行业通行的折中，且结果**可解释**（报告会逐项列出）。
//!
//! ⚠️ 只在**启动**时执行一次，且写库时是原子事务 + 留一份 `cfg_revisions`
//! （`source = "defaults-migration"`），需要时可按 revision 回滚。
//!
//! ## 设备 overlay / 预设不参与
//!
//! - `cfg_device_overlays` 只在用户显式保存单设备增量时写入，不存在"带旧默认的整份
//!   文档"，没有可对齐的存量；
//! - `cfg_presets` 是用户主动拍下的快照，语义就是"恢复成那一刻的样子"，动它等于篡改。

use serde_json::{Map, Value};

/// 当前代码的默认值版本（**只增不改**）。
///
/// 每次把某个"标准行为"的默认值从关改到开（或其他默认值变更），就往 [`BATCHES`]
/// 追加一条同号批次，并把这里 +1；已发布的批次**永不修改**，否则老库会按新规则
/// 解释旧数据。
pub const CURRENT_DEFAULTS_VERSION: u32 = 1;

/// 默认值（`Value` 的常量表无法直接写在 `const` 里，故用轻量枚举 + 转换）。
///
/// 目前只有布尔开关用得上；`Int`/`Str` 是给后续批次（数值/文案默认值变更）预留的，
/// 保持同一套对齐语义，避免将来再造一遍。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum DefaultValue {
    Bool(bool),
    Int(i64),
    Str(&'static str),
}

impl DefaultValue {
    fn to_json(self) -> Value {
        match self {
            DefaultValue::Bool(b) => Value::Bool(b),
            DefaultValue::Int(i) => Value::from(i),
            DefaultValue::Str(s) => Value::from(s),
        }
    }

    /// 与库内值比较（宽口径：类型对不上即视为"不等于"，从而按用户自定义保留）。
    fn matches(self, v: &Value) -> bool {
        match self {
            DefaultValue::Bool(b) => v.as_bool() == Some(b),
            DefaultValue::Int(i) => v.as_i64() == Some(i),
            DefaultValue::Str(s) => v.as_str() == Some(s),
        }
    }
}

impl std::fmt::Display for DefaultValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DefaultValue::Bool(b) => write!(f, "{b}"),
            DefaultValue::Int(i) => write!(f, "{i}"),
            DefaultValue::Str(s) => write!(f, "\"{s}\""),
        }
    }
}

/// 一条默认值变更声明。
pub struct DefaultChange {
    /// 点分路径（如 `wakeup.enabled`），与 `/api/config` 的 JSON 结构一一对应。
    pub path: &'static str,
    /// 变更前的默认值（判断"用户是否动过"的基准）。
    pub from: DefaultValue,
    /// 变更后的默认值（= 当前代码默认值，由测试护栏守卫）。
    pub to: DefaultValue,
    /// 这项行为是什么（进日志与报告，给部署者看）。
    pub note: &'static str,
}

/// 一批默认值变更（`version` = 完成这批之后库里应记录的版本）。
pub struct DefaultsBatch {
    pub version: u32,
    pub changes: &'static [DefaultChange],
}

/// v1：**标准行为默认开启**（开机即用，不想要可显式关）。
///
/// 覆盖范围 = 全部"标准体验"开关，不是只改唤醒问候语：唤醒问候、指令闸门、
/// 技能级联、人格档案、记忆、EchoMuse 入口、LLM 回声。`[aiui].enabled` 与
/// `[nlu].bert_enabled` **不在其内**——前者缺凭据即启动失败（fatal_if_enabled），
/// 后者缺 ONNX 模型文件即校验失败，都不是"开箱即用"的默认项。
pub const STANDARD_BEHAVIOR_ON: &[DefaultChange] = &[
    DefaultChange {
        path: "wakeup.enabled",
        from: DefaultValue::Bool(false),
        to: DefaultValue::Bool(true),
        note: "唤醒问候语（唤醒后立刻说「我在」）",
    },
    DefaultChange {
        path: "command.enabled",
        from: DefaultValue::Bool(false),
        to: DefaultValue::Bool(true),
        note: "语音指令闸门（退下 / 闭嘴 / 关闭）",
    },
    DefaultChange {
        path: "nlu.enabled",
        from: DefaultValue::Bool(false),
        to: DefaultValue::Bool(true),
        note: "技能级联（规则模板 + 本地轻量模型）",
    },
    DefaultChange {
        path: "soul.enabled",
        from: DefaultValue::Bool(false),
        to: DefaultValue::Bool(true),
        note: "人格档案（内置预设打底）",
    },
    DefaultChange {
        path: "memory.enabled",
        from: DefaultValue::Bool(false),
        to: DefaultValue::Bool(true),
        note: "记忆（写入本地 SQLite）",
    },
    DefaultChange {
        path: "memory.recall_enabled",
        from: DefaultValue::Bool(false),
        to: DefaultValue::Bool(true),
        note: "记忆召回（只写不读 → 读写）",
    },
    DefaultChange {
        path: "memory.extraction_enabled",
        from: DefaultValue::Bool(false),
        to: DefaultValue::Bool(true),
        note: "记忆抽取（原文 → 可召回摘要）",
    },
    DefaultChange {
        path: "echomuse.enabled",
        from: DefaultValue::Bool(false),
        to: DefaultValue::Bool(true),
        note: "EchoMuse 设备入口（/em 路由 + mDNS）",
    },
    DefaultChange {
        path: "llm.enabled",
        from: DefaultValue::Bool(false),
        to: DefaultValue::Bool(true),
        note: "LLM 对话（关闭时退化为 ASR 原话回声）",
    },
];

/// 全部批次（按版本升序）。新增批次只允许**追加**。
pub const BATCHES: &[DefaultsBatch] = &[DefaultsBatch {
    version: 1,
    changes: STANDARD_BEHAVIOR_ON,
}];

/// 一次对齐的结果（可读报告：谁被改了、谁被保留、谁补齐了）。
#[derive(Debug, Clone, Default)]
pub struct AlignReport {
    pub from_version: u32,
    pub to_version: u32,
    /// 旧默认 → 新默认（用户没动过）。
    pub changed: Vec<ChangeLog>,
    /// 键缺失，补成新默认。
    pub filled: Vec<String>,
    /// 值不等于旧默认，按用户自定义保留。
    pub kept: Vec<String>,
    /// 段结构不是对象（坏配置），跳过不动。
    pub broken: Vec<String>,
}

/// 一条"被对齐"的记录。
#[derive(Debug, Clone)]
pub struct ChangeLog {
    pub path: String,
    pub from: DefaultValue,
    pub to: DefaultValue,
    pub note: &'static str,
}

impl AlignReport {
    /// 本次是否需要写库（版本已是最新且无任何变更 → false）。
    pub fn is_noop(&self) -> bool {
        self.from_version >= self.to_version
            && self.changed.is_empty()
            && self.filled.is_empty()
            && self.broken.is_empty()
    }

    /// 一行日志摘要（info 档，部署者能据此知道"哪些默认值被对齐了"）。
    pub fn summary(&self) -> String {
        let mut s = format!(
            "配置库默认值对齐 v{} → v{}",
            self.from_version, self.to_version
        );
        if self.changed.is_empty() && self.filled.is_empty() && self.kept.is_empty() {
            s.push_str("：无变更");
            return s;
        }
        if !self.changed.is_empty() {
            let items: Vec<String> = self
                .changed
                .iter()
                .map(|c| format!("{} {}→{}（{}）", c.path, c.from, c.to, c.note))
                .collect();
            s.push_str(&format!("；对齐 {} 项：{}", items.len(), items.join("、")));
        }
        if !self.filled.is_empty() {
            s.push_str(&format!(
                "；补缺 {} 项：{}",
                self.filled.len(),
                join(&self.filled)
            ));
        }
        if !self.kept.is_empty() {
            s.push_str(&format!(
                "；保留用户自定义 {} 项：{}",
                self.kept.len(),
                join(&self.kept)
            ));
        }
        if !self.broken.is_empty() {
            s.push_str(&format!(
                "；结构异常跳过 {} 项：{}",
                self.broken.len(),
                join(&self.broken)
            ));
        }
        s
    }
}

/// 列表摘要（超过 8 项时折叠，避免日志刷屏）。
fn join(items: &[String]) -> String {
    if items.len() <= 8 {
        return items.join("、");
    }
    format!("{}…（共 {} 项）", items[..8].join("、"), items.len())
}

/// 对整份配置 JSON 执行默认值对齐（纯函数，便于单测）。
///
/// `from_version` = 库里记录的默认值版本；`0` 表示"从未对齐过"（旧库）。
pub fn align(doc: &mut Value, from_version: u32) -> AlignReport {
    let mut report = AlignReport {
        from_version,
        to_version: from_version.max(CURRENT_DEFAULTS_VERSION),
        ..AlignReport::default()
    };
    let Some(root) = doc.as_object_mut() else {
        report.broken.push("<root>".to_string());
        return report;
    };
    for batch in BATCHES {
        if batch.version <= from_version {
            continue;
        }
        for change in batch.changes {
            apply_change(root, change, &mut report);
        }
    }
    report
}

/// 单条变更落地（见模块文档的三条规则）。
fn apply_change(root: &mut Map<String, Value>, change: &DefaultChange, report: &mut AlignReport) {
    let mut segs: Vec<&str> = change.path.split('.').collect();
    let Some(leaf) = segs.pop() else {
        report.broken.push(change.path.to_string());
        return;
    };
    let Some(parent) = section_mut(root, &segs) else {
        report.broken.push(change.path.to_string());
        return;
    };
    match parent.get(leaf) {
        None | Some(Value::Null) => {
            parent.insert(leaf.to_string(), change.to.to_json());
            report.filled.push(change.path.to_string());
        }
        Some(v) if change.from.matches(v) => {
            parent.insert(leaf.to_string(), change.to.to_json());
            report.changed.push(ChangeLog {
                path: change.path.to_string(),
                from: change.from,
                to: change.to,
                note: change.note,
            });
        }
        Some(_) => report.kept.push(change.path.to_string()),
    }
}

/// 定位（并按需创建）中间段对象；已存在但不是对象 → `None`（坏配置，跳过）。
///
/// `null` 段按"缺失"处理（就地换成空对象）：TOML/JSON 手写配置里 `[wakeup]` 写空时
/// 常被读成 `null`，它表达的就是"这级没配"，不该算坏配置。
fn section_mut<'a>(
    root: &'a mut Map<String, Value>,
    segs: &[&str],
) -> Option<&'a mut Map<String, Value>> {
    let mut cur = root;
    for seg in segs {
        let slot = cur
            .entry(seg.to_string())
            .or_insert_with(|| Value::Object(Map::new()));
        if slot.is_null() {
            *slot = Value::Object(Map::new());
        }
        cur = slot.as_object_mut()?;
    }
    Some(cur)
}

#[cfg(test)]
mod tests;
