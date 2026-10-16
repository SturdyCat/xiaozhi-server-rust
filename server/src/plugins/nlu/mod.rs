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
        }
    }
}

fn default_model_threshold() -> f32 {
    0.9
}
fn default_model_margin() -> f32 {
    0.1
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
