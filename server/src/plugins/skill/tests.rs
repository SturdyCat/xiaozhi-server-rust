//! `server/src/plugins/skill` 的单元测试。
//!
//! 覆盖三级级联的**目录契约**（识别源只提名、副作用统一校验）：
//! ① 规则层命中（`[command]` end_session + `[nlu]` 精确模板）；
//! ② 轻量模型层：规则未命中的口语化说法达阈值执行；否定/歧义/阈值关闭不放行；
//! ③ `resolve` 是唯一安全边界：未知技能、多余槽位、坏槽位值一律拒绝；
//! ④ LLM 侧 `execute_skill` 工具：目录形状、白名单外工具与坏 JSON 拒绝；
//! ⑤ `validate`：坏阈值 / 重名 / 未知 action.kind 在保存期被拦下。

use std::collections::HashMap;

use serde_json::json;

use super::*;
use crate::plugins::command::CommandConfig;
use crate::plugins::llm::ToolCall;
use crate::plugins::nlu::{IntentDef, NluConfig, SlotList};

/// 两技能一槽位表的标准夹具：control_light（开/关灯）+ control_ac（空调，用于歧义竞争）。
fn fixture() -> (CommandConfig, NluConfig) {
    let command = CommandConfig {
        enabled: true,
        ..CommandConfig::default()
    };
    let nlu = NluConfig {
        enabled: true,
        intents: vec![
            IntentDef {
                name: "control_light".into(),
                templates: vec!["把{room}的灯{action}".into()],
                action: crate::plugins::nlu::ActionDef {
                    kind: "mcp:mcp_light_control".into(),
                    reply: "好的".into(),
                },
            },
            IntentDef {
                name: "control_ac".into(),
                templates: vec!["把{room}的空调{action}".into()],
                action: Default::default(),
            },
        ],
        slots: vec![
            SlotList {
                name: "room".into(),
                values: vec!["客厅".into(), "卧室".into()],
                optional: false,
            },
            SlotList {
                name: "action".into(),
                values: vec!["打开".into(), "关闭".into()],
                optional: false,
            },
        ],
        ..NluConfig::default()
    };
    (command, nlu)
}

fn registry() -> SkillRegistry {
    let (command, nlu) = fixture();
    SkillRegistry::new(&command, &nlu, None)
}

fn slots(pairs: &[(&str, &str)]) -> HashMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

/// ① 规则层：end_session（`[command]`）与精确模板（`[nlu]`）都走 Rules 且动作编译正确。
#[test]
fn rules_hit_end_session_and_exact_template() {
    let r = registry();
    // end_session 来自 `[command]`（exact、短词）。
    let hit = r.parse_staged("退下。").expect("end_session 应命中").1;
    assert_eq!(hit.name, "end_session");
    assert_eq!(hit.action, SkillAction::EndSession);
    // 精确模板：槽位取值 + 规范化为配置原文 + mcp 动作。
    let hit = r
        .parse_staged("把客厅的灯关闭")
        .expect("精确模板应命中（规则层）")
        .1;
    assert_eq!(hit.name, "control_light");
    assert_eq!(
        hit.action,
        SkillAction::LocalCommand {
            kind: "mcp:mcp_light_control".into()
        }
    );
    assert_eq!(hit.slots.get("room").map(String::as_str), Some("客厅"));
    assert_eq!(hit.slots.get("action").map(String::as_str), Some("关闭"));
}

/// ② 模型层：规则拒绝的口语化说法达阈值执行（中间槽位"大客厅"含表内"客厅"），
/// 槽位取值表规范值——这是模型层相对规则层（中间槽位仅精确匹配）的真实增量。
#[test]
fn model_hits_colloquial_fillers_rules_missed() {
    let r = registry();
    let (stage, hit) = r
        .parse_staged("帮我把大客厅的灯打开")
        .expect("口语化说法应由模型层命中");
    assert_eq!(stage, SkillStage::Model);
    assert_eq!(hit.name, "control_light");
    assert_eq!(hit.slots.get("room").map(String::as_str), Some("客厅"));
    assert_eq!(hit.slots.get("action").map(String::as_str), Some("打开"));
}

/// ② 模型层安全边界：否定词、多值歧义、锚点缺失都不得提名（宁可漏给 LLM）。
/// 规则层的尾随槽位包含同样带否定/歧义闸（见 `matcher`），两层层层设防。
#[test]
fn model_abstains_on_negation_ambiguity_and_missing_anchor() {
    let r = registry();
    // 否定：「别打开」含否定词 → 不执行（规则层先行拒绝，模型层同样拒绝）。
    assert!(r.parse_staged("把客厅的灯别打开").is_none());
    assert!(r.parse_staged("把大客厅的灯不要打开").is_none());
    // 多值歧义：间隙同时含「打开/关闭」→ 不执行。
    assert!(r.parse_staged("把客厅的灯打开还是关闭").is_none());
    // 锚点缺失（说的不是"灯"也不是"空调"）→ 两技能都弃权。
    assert!(r.parse_staged("把客厅的电视打开").is_none());
}

/// ② 阈值封顶（1.0）= 关闭模型层：模型层独有的中间槽位模糊也不再本地执行（放行 LLM）。
#[test]
fn threshold_one_disables_model_stage() {
    let (command, mut nlu) = fixture();
    nlu.model_threshold = 1.0;
    let r = SkillRegistry::new(&command, &nlu, None);
    assert!(r.parse_staged("帮我把大客厅的灯打开").is_none());
}

/// ③ `resolve` 安全边界：未知技能 / 多余槽位 / 坏槽位值 / 超长开放槽位都拒绝。
#[test]
fn resolve_rejects_unknown_skill_bad_slots_and_values() {
    let r = registry();
    assert!(
        r.resolve("nope", HashMap::new()).is_err(),
        "未知技能必须拒绝"
    );
    // 必填槽位缺失。
    assert!(r.resolve("control_light", HashMap::new()).is_err());
    // 多余字段（LLM 增造槽位）。
    let mut extra = slots(&[("room", "客厅"), ("action", "打开")]);
    extra.insert("extra".into(), "x".into());
    assert!(r.resolve("control_light", extra).is_err());
    // 封闭槽位值不在表内。
    assert!(r
        .resolve(
            "control_light",
            slots(&[("room", "厨房"), ("action", "打开")])
        )
        .is_err());
}

/// ③ 槽位值规范化回写：模型/LLM 提供归一化值时，resolve 改写为配置原文（canonical）。
#[test]
fn resolve_rewrites_slot_to_canonical_value() {
    let r = registry();
    let inv = r
        .resolve(
            "control_light",
            slots(&[("room", "客 厅"), ("action", "关 闭")]),
        )
        .expect("归一化等值的槽位应被接受");
    assert_eq!(inv.slots.get("room").map(String::as_str), Some("客厅"));
    assert_eq!(inv.slots.get("action").map(String::as_str), Some("关闭"));
}

/// ④ LLM 工具目录：只有一个 `execute_skill`，技能枚举与槽位表齐全；目录为空时不暴露工具。
#[test]
fn tools_expose_single_execute_skill_spec() {
    let r = registry();
    let tools = r.tools();
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].name, TOOL_NAME);
    let params = &tools[0].parameters;
    let skill_enum: Vec<&str> = params["properties"]["skill"]["enum"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
    assert!(skill_enum.contains(&"control_light"), "{skill_enum:?}");
    assert!(skill_enum.contains(&"end_session"), "{skill_enum:?}");
    assert!(
        tools[0].description.contains("control_ac"),
        "目录须包含全部技能名"
    );
    // 全关目录：无工具（会话层不传 tools，LLM 纯聊天）。
    let empty = SkillRegistry::new(&CommandConfig::default(), &NluConfig::default(), None);
    assert!(empty.tools().is_empty());
}

/// ④ `from_tool_call`：合法调用走同一 resolve；白名单外工具 / 坏 JSON / 超长参数拒绝。
#[test]
fn from_tool_call_enforces_whitelist_and_shape() {
    let r = registry();
    let ok = r
        .from_tool_call(&ToolCall {
            id: "call_1".into(),
            name: TOOL_NAME.into(),
            arguments:
                json!({"skill": "control_light", "slots": {"room": "卧室", "action": "打开"}})
                    .to_string(),
        })
        .expect("合法工具调用应被接受");
    assert_eq!(ok.name, "control_light");
    // 白名单外工具名。
    assert!(r
        .from_tool_call(&ToolCall {
            id: "call_2".into(),
            name: "rm_rf".into(),
            arguments: "{}".into(),
        })
        .is_err());
    // 坏 JSON / 多余字段。
    assert!(r
        .from_tool_call(&ToolCall {
            id: "call_3".into(),
            name: TOOL_NAME.into(),
            arguments: "not json".into()
        })
        .is_err());
    assert!(r
        .from_tool_call(&ToolCall {
            id: "call_4".into(),
            name: TOOL_NAME.into(),
            arguments: json!({"skill": "control_light", "slots": {}, "extra": 1}).to_string(),
        })
        .is_err());
    // 参数体超限。
    let big = "x".repeat(17 * 1024);
    assert!(r
        .from_tool_call(&ToolCall {
            id: "call_5".into(),
            name: TOOL_NAME.into(),
            arguments: big
        })
        .is_err());
}

/// ⑤ `validate`：坏阈值 / 重名 / 未知 action.kind / 空名在保存期被拦，文案可行动。
#[test]
fn validate_blocks_bad_threshold_names_and_kinds() {
    let (command, mut nlu) = fixture();
    nlu.model_threshold = 1.5;
    assert!(validate(&command, &nlu).is_err(), "阈值超出 [0,1] 必须被拦");
    let (command, mut nlu) = fixture();
    nlu.intents.push(IntentDef {
        name: "control_light".into(),
        templates: vec!["开灯".into()],
        action: Default::default(),
    });
    assert!(validate(&command, &nlu).is_err(), "重名技能必须被拦");
    let (command, mut nlu) = fixture();
    nlu.intents[0].action.kind = "shutdown -h".into();
    assert!(
        validate(&command, &nlu).is_err(),
        "未知 action.kind 必须被拦"
    );
    let (command, mut nlu) = fixture();
    nlu.intents[0].name = "  ".into();
    assert!(validate(&command, &nlu).is_err(), "空技能名必须被拦");
}
