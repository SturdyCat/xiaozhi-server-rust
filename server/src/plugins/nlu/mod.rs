//! NLU 规则引擎：ASR → LLM 之间的本地意图识别层（零依赖、确定性）。
//!
//! ## 架构定位
//!
//! 本模块"含有"原有的 `[command]` 能力：`end_session`（退下/闭嘴/关闭）是其中一个**内置高危意图**，
//! 其余意图由 `[nlu]` 配置定义。识别到已知意图即执行其 `action`（本地指令，不再送 LLM）；
//! **未知意图才放行给 LLM**（与原有 command "挡在 LLM 之前"的初衷一致，但升级为意图 + 槽位）。
//!
//! ## 为什么是规则引擎而非概率模型
//!
//! 命令场景要求**确定性**：误触发（把正常请求当指令）比漏触发代价高得多（断线/误执行是灾难）。
//! 因此本引擎用字面锚点 + 槽位表的有序匹配，结果可单测、可解释、且零误判概率。
//!
//! ## 匹配口径（安全优先，沿用 command 的归一化）
//!
//! 识别文本先经 [`crate::plugins::command::normalize_utterance`] 归一化（去标点 + 句末语气助词），
//! 再去除空白后与模板锚点比较。
//!
//! ## 槽位模型
//!
//! 模板形如 `把{room}的灯{action}`：拆为字面锚点与槽位的交错序列。锚点在归一化文本中**按顺序**不重叠定位，
//! 锚点之间的子串作为槽位候选，用对应槽位表（封闭枚举）校验/抽取。
//! 开放槽位（无值表）取锚点后到结尾的剩余文本，留作扩展。

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::plugins::command::{normalize_utterance, CommandConfig, CommandGate};

mod matcher;
pub(crate) mod template;
#[cfg(test)]
mod tests;

pub(crate) mod bert;
// BertHit/bert_feature_enabled 仅 feature 构建内部使用，默认构建 re-export 会闲置。
#[allow(unused_imports)]
pub use bert::{
    BertClassify, BertHit, bert_feature_enabled, bert_signature, build_bert_checked,
};
// TinyBertNlu 供外部/测试直接用类型；本 crate 内部经 trait 对象使用（二进制构建会闲置）。
#[cfg(feature = "nlu")]
#[allow(unused_imports)]
pub use bert::TinyBertNlu;

use matcher::match_template;
pub use template::{parse_template, Template};

/// 槽位表：命名的值集合，用于模板槽位校验/抽取。
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SlotList {
    pub name: String,
    #[serde(default)]
    pub values: Vec<String>,
    #[serde(default)]
    pub optional: bool,
}

/// 命中后执行的动作（副作用种类）。命中后要说的话统一放在 [`IntentMatch::reply`]。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IntentAction {
    /// 结束会话：沿用 command 的 MCP 结束聆听（会话层据 `reply` 先说告别语再让设备退下）。
    EndSession,
    /// 其他本地指令（预留扩展位，由会话层按 `kind` 分发到设备 MCP 工具，如灯光控制）。
    LocalCommand { kind: String },
}

/// 已编译的意图（模板已解析为结构化 token 序列）。
#[derive(Debug, Clone)]
pub struct Intent {
    pub name: String,
    pub templates: Vec<Template>,
    pub action: IntentAction,
    /// 命中后要说的话（可能为空 = 直接执行副作用、不出声）。
    pub reply: String,
}

/// 一次命中的结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntentMatch {
    pub intent: String,
    pub slots: HashMap<String, String>,
    pub action: IntentAction,
    /// 命中后要说的话（可能为空 = 直接执行副作用、不出声）。
    pub reply: String,
}

/// `[nlu]` 配置段：自定义意图 + 槽位表。
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct NluConfig {
    /// 总开关。默认 **false**：关闭时本段不产生任何意图（end_session 仍由 `[command]` 决定），
    /// 行为与加本插件之前完全一致（零开销、零行为变化）。
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub intents: Vec<IntentDef>,
    #[serde(default)]
    pub slots: Vec<SlotList>,
    /// 轻量模型阶段的接受阈值（第二级识别）：规则未命中后，本地打分 ≥ 此值且领先次高分
    /// [`Self::model_margin`] 才执行；否则放行 LLM。取值 [0,1]；1.0 = 实际关闭该阶段。
    /// 默认 0.9：漏触发优于误触发（与规则层同一安全口径）。
    #[serde(default = "default_model_threshold")]
    pub model_threshold: f32,
    /// 轻量模型阶段的领先幅度：最高分与**其他技能**最高分的差值下限（防相近指令歧义）。
    #[serde(default = "default_model_margin")]
    pub model_margin: f32,
    /// TinyBERT 语义分类器总开关（第二级识别的模型实现，需 feature `nlu` 编译）。
    /// 关闭（默认）时第二级只用内置启发式打分（[`crate::plugins::skill::LightModel`]），零行为变化。
    #[serde(default)]
    pub bert_enabled: bool,
    /// TinyBERT ONNX 模型路径（int8 量化的 TinyBERT4，HF `transformers` 导出）。
    #[serde(default)]
    pub bert_model: String,
    /// 分词器词表路径（BERT `vocab.txt`，与导出模型配套）。
    #[serde(default)]
    pub bert_vocab: String,
    /// 标签表路径：一行一个标签，行号 = 分类输出下标；标签经 [`Self::bert_label_map`]
    /// 映射到技能名（未映射且同名的技能不存在时，该标签不会被提名执行）。
    #[serde(default)]
    pub bert_labels: String,
    /// TinyBERT 接受阈值：最高标签置信度 ≥ 此值才提名技能（[0,1]，默认 0.75）。
    #[serde(default = "default_bert_threshold")]
    pub bert_threshold: f32,
    /// ONNX Runtime 线程数（默认 1：单句毫秒级推理，避免与音频线程抢核）。
    #[serde(default = "default_bert_num_threads")]
    pub bert_num_threads: usize,
    /// 分词截断长度（含 [CLS]/[SEP]，默认 64；指令句远短于此）。
    #[serde(default = "default_bert_max_len")]
    pub bert_max_len: usize,
    /// ONNX 标签 → 技能名映射（缺省同名直配）。嵌套 map，管理台走 TOML 高级配置。
    #[serde(default)]
    pub bert_label_map: HashMap<String, String>,
}

impl Default for NluConfig {
    /// 手写而非 `derive`：`f32` 的 derive 默认是 `0.0`——那会让"阈值 0 + 边际 0"把
    /// 轻量模型层变成"沾边就执行"。默认值必须与 serde default 函数同源（单一权威）。
    fn default() -> Self {
        NluConfig {
            enabled: false,
            intents: Vec::new(),
            slots: Vec::new(),
            model_threshold: default_model_threshold(),
            model_margin: default_model_margin(),
            bert_enabled: false,
            bert_model: String::new(),
            bert_vocab: String::new(),
            bert_labels: String::new(),
            bert_threshold: default_bert_threshold(),
            bert_num_threads: default_bert_num_threads(),
            bert_max_len: default_bert_max_len(),
            bert_label_map: HashMap::new(),
        }
    }
}

fn default_model_threshold() -> f32 {
    0.9
}
fn default_model_margin() -> f32 {
    0.1
}
fn default_bert_threshold() -> f32 {
    0.75
}
fn default_bert_num_threads() -> usize {
    1
}
fn default_bert_max_len() -> usize {
    64
}

impl NluConfig {
    /// TinyBERT 层是否声明可用（开关 + 三个文件路径齐备）。**不含** feature/文件存在性
    /// 检查——那两步在 [`bert::build_bert`] / 注册表 `validate_nlu`，错误文案要带"怎么修"。
    pub fn bert_configured(&self) -> bool {
        self.bert_enabled
            && !self.bert_model.trim().is_empty()
            && !self.bert_vocab.trim().is_empty()
            && !self.bert_labels.trim().is_empty()
    }

    /// ONNX 标签 → 技能名（缺省同名直配；映射后查不到的技能由 `resolve` 拒绝）。
    pub fn bert_map_label<'a>(&'a self, label: &'a str) -> &'a str {
        self.bert_label_map
            .get(label)
            .map(|s| s.as_str())
            .unwrap_or(label)
    }
}

/// 配置层意图定义（模板为字符串形式）。
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct IntentDef {
    pub name: String,
    #[serde(default)]
    pub templates: Vec<String>,
    #[serde(default)]
    pub action: ActionDef,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct ActionDef {
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub reply: String,
}

/// 归一化（与 command 同口径）：去标点/语气词 + 去空白。
pub(crate) fn norm(s: &str) -> String {
    normalize_utterance(s)
        .split_whitespace()
        .collect::<String>()
}

/// 内置提醒技能（硬编码默认意图，`[nlu].enabled` 时与配置意图合并生效）：
/// `reminder_countdown`（倒计时）/ `reminder_schedule`（定时）/ `reminder_repeat`（循环定时）。
/// 槽位全部开放（时间/内容是自然语言表达，不设封闭值表）；动作均为 `reply` 本地应答——
/// 这组技能当前用于跑通「技能定义 → 模型训练 → BERT 识别」的完整链路，
/// 真正的定时派发后续按 `mcp:` 动作接入。
pub(crate) fn builtin_defs() -> (Vec<IntentDef>, Vec<SlotList>) {
    let open = |name: &str| SlotList {
        name: name.to_string(),
        values: Vec::new(),
        optional: false,
    };
    let intents = vec![
        IntentDef {
            name: "reminder_countdown".into(),
            templates: vec![
                "{delay}后提醒我{task}".into(),
                "过{delay}提醒我{task}".into(),
                "{delay}之后提醒我{task}".into(),
            ],
            action: ActionDef {
                kind: "reply".into(),
                reply: "好的，到点我会提醒你。".into(),
            },
        },
        IntentDef {
            name: "reminder_schedule".into(),
            templates: vec!["{time}提醒我{task}".into()],
            action: ActionDef {
                kind: "reply".into(),
                reply: "好的，已设好定时提醒。".into(),
            },
        },
        IntentDef {
            name: "reminder_repeat".into(),
            templates: vec![
                "每隔{repeat}提醒我{task}".into(),
                "{repeat}提醒我{task}".into(),
            ],
            action: ActionDef {
                kind: "reply".into(),
                reply: "好的，循环提醒已设置。".into(),
            },
        },
    ];
    let slots = vec![open("delay"), open("time"), open("repeat"), open("task")];
    (intents, slots)
}

impl NluConfig {
    /// 配置意图 ⊕ 内置提醒技能（内置在前、被配置同名者跳过；槽位同名同理）。
    /// 规则引擎 / 技能目录 / 模型训练导出三处共用同一份合并结果——训练标签与运行时技能名
    /// 必然一致，`bert_label_map` 缺省同名直配即可。
    pub fn with_builtins(&self) -> NluConfig {
        let (mut intents, mut slots) = builtin_defs();
        intents.retain(|b| !self.intents.iter().any(|d| d.name == b.name));
        slots.retain(|b| !self.slots.iter().any(|d| d.name == b.name));
        let mut merged = self.clone();
        intents.extend(self.intents.iter().cloned());
        merged.intents = intents;
        slots.extend(self.slots.iter().cloned());
        merged.slots = slots;
        merged
    }
}

/// 否定词表：封闭槽位间隙出现即整条弃权（方向性副作用宁可漏给 LLM，不可反向执行）。
pub(crate) const NEGATION_CUES: [&str; 6] = ["别", "不要", "不用", "不准", "不可以", "不能"];

/// 文本是否含否定词（规则层与轻量模型层共用同一张表，防两层口径漂移）。
pub(crate) fn contains_negation(s: &str) -> bool {
    NEGATION_CUES.iter().any(|c| s.contains(c))
}

/// 编译后的 NLU 引擎：持有 end_session 闸门（`[command]`）+ 自定义意图（`[nlu]`）+ 槽位表。
#[derive(Debug, Clone)]
pub struct NluEngine {
    end_session: CommandGate,
    intents: Vec<Intent>,
    slot_values: HashMap<String, Vec<String>>,
    slot_optional: HashMap<String, bool>,
}

impl NluEngine {
    /// 按两处配置快照构造：`command`（end_session 来源，向后兼容）+ `nlu`（自定义意图）。
    pub fn new(command: &CommandConfig, nlu: &NluConfig) -> Self {
        // 内置提醒技能合并（见 [`NluConfig::with_builtins`]）：与配置意图同一管道编译。
        let merged = nlu.with_builtins();
        let nlu = &merged;
        let end_session = CommandGate::new(command);
        // 自定义意图受 `[nlu].enabled` 总开关约束：关闭时视为无意图（end_session 仍由 command 决定）。
        let (intents, slot_values, slot_optional) = if nlu.enabled {
            let slot_values: HashMap<String, Vec<String>> = nlu
                .slots
                .iter()
                .map(|s| (s.name.clone(), s.values.iter().map(|v| norm(v)).collect()))
                .collect();
            let slot_optional: HashMap<String, bool> = nlu
                .slots
                .iter()
                .map(|s| (s.name.clone(), s.optional))
                .collect();
            let intents: Vec<Intent> = nlu
                .intents
                .iter()
                .map(|def| Intent {
                    name: def.name.clone(),
                    templates: def.templates.iter().map(|t| parse_template(t)).collect(),
                    action: if def.action.kind == "end_session" {
                        IntentAction::EndSession
                    } else {
                        IntentAction::LocalCommand {
                            kind: def.action.kind.clone(),
                        }
                    },
                    reply: def.action.reply.clone(),
                })
                .collect();
            (intents, slot_values, slot_optional)
        } else {
            (Vec::new(), HashMap::new(), HashMap::new())
        };
        NluEngine {
            end_session,
            intents,
            slot_values,
            slot_optional,
        }
    }

    /// 解析识别文本：命中返回 `IntentMatch`，未命中（未知意图）返回 `None`（调用方放行给 LLM）。
    pub fn parse(&self, text: &str) -> Option<IntentMatch> {
        // 1) end_session 优先（短词高危，沿用 command 的长度闸门 + exact/contains 安全模型）。
        if let Some(hit) = self.end_session.hit(text) {
            return Some(IntentMatch {
                intent: "end_session".to_string(),
                slots: HashMap::new(),
                action: IntentAction::EndSession,
                reply: hit.reply,
            });
        }
        // 2) 自定义意图：模板 + 槽位的有序匹配。
        let utt = norm(text);
        if utt.is_empty() {
            return None;
        }
        for intent in &self.intents {
            for tpl in &intent.templates {
                if let Some(slots) =
                    match_template(&utt, tpl, &self.slot_values, &self.slot_optional)
                {
                    return Some(IntentMatch {
                        intent: intent.name.clone(),
                        slots,
                        action: intent.action.clone(),
                        reply: intent.reply.clone(),
                    });
                }
            }
        }
        None
    }
}
