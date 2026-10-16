//! `server/src/plugins/nlu/mod.rs` 的单元测试（外移自该文件，AGENTS.md §5.10）。
//!
//! 锚定行为：模板命中与槽位抽取、封闭槽位值校验失败（漏触发优于误触发）、
//! 归一化对标点/语气词的容忍、end_session 向后兼容（短词命中 / 长句放行给 LLM）、
//! 以及"未知意图返回 None = 放行给 LLM"这条核心不变量。

use super::*;
use crate::plugins::command::{CommandConfig, MatchMode};

/// 构造一个带 `control_light` 意图 + room/action 槽位的引擎（command 关闭，纯 NLU 验证）。
fn control_light_engine() -> NluEngine {
    let nlu = NluConfig {
        enabled: true,
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
        intents: vec![IntentDef {
            name: "control_light".into(),
            templates: vec!["把{room}的灯{action}".into()],
            action: ActionDef {
                kind: "local_command".into(),
                ..Default::default()
            },
        }],
        ..NluConfig::default()
    };
    NluEngine::new(&CommandConfig::default(), &nlu)
}

#[test]
fn template_intent_extracts_slots() {
    let eng = control_light_engine();
    let m = eng.parse("把卧室的灯关闭").expect("应命中 control_light");
    assert_eq!(m.intent, "control_light");
    assert_eq!(m.slots.get("room").map(String::as_str), Some("卧室"));
    assert_eq!(m.slots.get("action").map(String::as_str), Some("关闭"));
    assert_eq!(
        m.action,
        IntentAction::LocalCommand {
            kind: "local_command".into()
        }
    );
}

#[test]
fn template_fails_when_slot_value_not_in_list() {
    // "厨房" 不在 room 值表里 → 模板失败 → 未命中 → 放行 LLM（漏触发优于误触发）。
    let eng = control_light_engine();
    assert_eq!(eng.parse("把厨房的灯关闭"), None);
    // "关掉" 不在 action 值表里同理。
    assert_eq!(eng.parse("把卧室的灯关掉"), None);
}

#[test]
fn template_normalization_tolerates_punctuation_and_particles() {
    let eng = control_light_engine();
    let m = eng.parse("把，卧室，的灯关闭！").expect("标点应被归一化掉");
    assert_eq!(m.slots.get("room").map(String::as_str), Some("卧室"));
    assert_eq!(m.slots.get("action").map(String::as_str), Some("关闭"));

    // 句末语气助词不该影响锚点（"关闭啊" 归一化为 "关闭"）
    let m2 = eng.parse("把卧室的灯关闭啊").expect("语气词应被剥离");
    assert_eq!(m2.slots.get("action").map(String::as_str), Some("关闭"));
}

#[test]
fn prefix_and_suffix_are_ignored() {
    // 语音识别常多说"请"/"谢谢"，模板不强制消费全文。
    let eng = control_light_engine();
    let m = eng
        .parse("请帮我把卧室的灯关闭谢谢")
        .expect("前后缀应被容忍");
    assert_eq!(m.intent, "control_light");
    assert_eq!(m.slots.get("room").map(String::as_str), Some("卧室"));
}

#[test]
fn optional_slot_allows_empty() {
    let nlu = NluConfig {
        enabled: true,
        slots: vec![SlotList {
            name: "singer".into(),
            values: vec![],
            optional: true,
        }],
        intents: vec![IntentDef {
            name: "play_music".into(),
            templates: vec!["播放{singer}的歌".into()],
            ..Default::default()
        }],
        ..NluConfig::default()
    };
    let eng = NluEngine::new(&CommandConfig::default(), &nlu);
    // "播放的歌" → singer 为空，optional 允许。
    let m = eng.parse("播放的歌").expect("optional 空槽位应命中");
    assert_eq!(m.slots.get("singer").map(String::as_str), Some(""));
    // 给了值也能命中（开放槽位：值表为空则原样抽取）
    let m2 = eng.parse("播放周杰伦的歌").expect("有值也应命中");
    assert_eq!(m2.slots.get("singer").map(String::as_str), Some("周杰伦"));
}

#[test]
fn end_session_backward_compat_short_hit() {
    // 默认 command 启用 + 默认词表（退下/闭嘴/关闭），短句应命中 end_session。
    let cmd = CommandConfig {
        enabled: true,
        ..Default::default()
    };
    let eng = NluEngine::new(&cmd, &NluConfig::default());
    let m = eng.parse("退下吧。").expect("应命中 end_session");
    assert_eq!(m.intent, "end_session");
    assert_eq!(m.action, IntentAction::EndSession);
    assert_eq!(m.reply, "好的，我先退下了。");
}

#[test]
fn end_session_long_utterance_passes_to_llm() {
    // command 关闭时，连退下都不该拦。
    let eng = NluEngine::new(&CommandConfig::default(), &NluConfig::default());
    assert_eq!(eng.parse("退下"), None);

    // contains 模式下，长句含指令词仍被长度闸门挡住（避免误断会话）。
    let cmd = CommandConfig {
        enabled: true,
        match_mode: MatchMode::Contains,
        ..Default::default()
    };
    let eng2 = NluEngine::new(&cmd, &NluConfig::default());
    assert_eq!(eng2.parse("帮我关闭卧室的灯"), None);
}

#[test]
fn unknown_intent_returns_none() {
    // 既非 end_session、也非任何模板 → None = 放行给 LLM。
    let eng = control_light_engine();
    assert_eq!(eng.parse("今天天气怎么样"), None);
    assert_eq!(eng.parse("把灯关掉"), None); // action 值不在表内，漏触发
}

#[test]
fn inactive_engine_never_matches() {
    // command 关闭 且 无自定义意图 → 完全不拦。
    // 「是否有技能可拦」由 `SkillRegistry::active`（目录非空）判定；引擎层的可观测契约：
    // 全关时任何文本都不命中（包括高危的 end_session 词）。
    let eng = NluEngine::new(&CommandConfig::default(), &NluConfig::default());
    assert_eq!(eng.parse("退下"), None);
    assert_eq!(eng.parse("把卧室的灯关闭"), None);
}

/// —— `template.rs` 的解析测试（自内联块外置，AGENTS.md §5.10 无例外口径）——

#[test]
fn template_parses_lit_slot_lit_slot() {
    let t = parse_template("把{room}的灯{action}");
    assert_eq!(
        t.tokens,
        vec![
            template::Tok::Lit("把".into()),
            template::Tok::Slot("room".into()),
            template::Tok::Lit("的灯".into()),
            template::Tok::Slot("action".into()),
        ]
    );
}

#[test]
fn template_normalizes_literals_and_strips_punctuation() {
    // 字面里的标点应被归一化掉，槽位名不受影响
    let t = parse_template("打开{room}的灯！");
    assert_eq!(
        t.tokens,
        vec![
            template::Tok::Lit("打开".into()),
            template::Tok::Slot("room".into()),
            template::Tok::Lit("的灯".into()),
        ]
    );
}

#[test]
fn template_trailing_slot_without_literal() {
    let t = parse_template("{content}");
    assert_eq!(t.tokens, vec![template::Tok::Slot("content".into())]);
}
