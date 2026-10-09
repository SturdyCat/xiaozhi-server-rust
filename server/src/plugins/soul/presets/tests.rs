//! `plugins/soul/presets` 的回归测试（逻辑见 `presets.rs`；测试与逻辑分文件见 `AGENTS.md` §5.10）。

use super::*;
use crate::plugins::prompt;

/// 内置预设是**数据**：内容空洞（只有名字）就等于没提供默认灵魂，必须有实质内容。
#[test]
fn presets_have_substance_and_unique_ids() {
    assert!(!PRESETS.is_empty(), "至少要有一个内置预设");
    let ids: Vec<&str> = PRESETS.iter().map(|p| p.id).collect();
    assert_eq!(ids.iter().collect::<std::collections::HashSet<_>>().len(), ids.len(), "preset id 必须唯一：{ids:?}");
    assert!(ids.contains(&DEFAULT_ID), "默认预设 {DEFAULT_ID} 必须在表里");
    assert!(get(DEFAULT_ID).is_some());
    for p in PRESETS {
        assert!(!p.name.trim().is_empty() && !p.description.trim().is_empty(), "{} 缺少展示文案", p.id);
        let s = (p.profile)();
        assert!(!s.self_intro.trim().is_empty(), "{} 缺 self_intro", p.id);
        assert!(!s.tone.trim().is_empty(), "{} 缺 tone", p.id);
        assert!(!s.name.trim().is_empty(), "{} 缺 name", p.id);
        assert!(!s.address_user.trim().is_empty(), "{} 缺 address_user", p.id);
        assert!(s.traits.len() >= 3, "{} 性格条目太少（只写形容词会失效）", p.id);
        assert!(s.values.len() >= 2, "{} 价值条目太少", p.id);
        assert!(s.boundaries.len() >= 2, "{} 红线太少", p.id);
        assert!(s.examples.len() >= 2, "{} 风格范例太少（锁风格最有效的手段）", p.id);
        // 范例必须是 Q/A 两行，单行范例起不到锁风格的作用
        for e in &s.examples {
            assert!(e.contains('\n'), "{} 的范例应为「用户：…\\n<名字>：…」两行：{e:?}", p.id);
        }
    }
}

/// 预设文本是提示词的一部分：变量拼错必须在测试期就暴露（而不是等用户保存时才发现）。
#[test]
fn preset_text_uses_only_declared_variables() {
    for p in PRESETS {
        let mut s = (p.profile)();
        s.enabled = true;
        prompt::validate_sections(&s.sections("")).unwrap_or_else(|e| {
            panic!("预设 {} 的文本里有非法变量引用：{e:#}", p.id);
        });
    }
}

/// `provided_fields` 决定"预设提供了什么"，因此它**只能**含可填写字段：
/// 混进 `enabled`/布尔/数值会在客户端"载入预设"时把用户的选择冲掉。
#[test]
fn provided_fields_only_carries_fillable_fields() {
    let all_keys: Vec<String> = serde_json::to_value(SoulConfig::default())
        .unwrap()
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect();
    let not_fillable = [
        "enabled",
        "preset",
        "colloquial",
        "emoji",
        "max_sentences",
        "max_chars",
    ];
    for p in PRESETS {
        let fields = provided_fields(&(p.profile)());
        for (k, v) in &fields {
            assert!(all_keys.contains(k), "{} 提供了不存在的字段 {k}", p.id);
            assert!(!not_fillable.contains(&k.as_str()), "{} 提供了不可留空的字段 {k}", p.id);
            match v {
                Value::String(s) => assert!(!s.trim().is_empty(), "{} 的 {k} 是空串", p.id),
                Value::Array(a) => assert!(!a.is_empty(), "{} 的 {k} 是空数组", p.id),
                other => panic!("{} 的 {k} 类型不该出现：{other:?}", p.id),
            }
        }
        // 默认灵魂的"实质"就在这些字段里：缺一个都说明 provided_fields 漏了
        for k in ["self_intro", "tone", "traits", "values", "boundaries", "examples"] {
            assert!(fields.contains_key(k), "{} 的 {k} 应被提供", p.id);
        }
        // 客户端把它直接喂给表单（`fill`），因此必须能反序列化回配置
        let round: SoulConfig =
            serde_json::from_value(Value::Object(fields.clone())).expect("profile 必须能反序列化");
        assert_eq!(round.self_intro, (p.profile)().self_intro);
        assert_eq!(round.traits, (p.profile)().traits);
        // 未提供的字段保持 serde 默认（证明"没被冲掉"）
        assert_eq!(round.max_sentences, SoulConfig::default().max_sentences);
        assert!(!round.emoji);
    }
}

/// `GET /api/soul/presets` 的 wire 形状（客户端按 `id`/`name`/`profile` 取值，改键名不会编译报错）。
#[test]
fn json_list_exposes_id_name_description_profile() {
    let list = json_list();
    let arr = list.as_array().expect("应为数组");
    assert_eq!(arr.len(), PRESETS.len());
    for (i, p) in PRESETS.iter().enumerate() {
        let e = &arr[i];
        assert_eq!(e["id"], p.id);
        assert_eq!(e["name"], p.name);
        assert_eq!(e["description"], p.description);
        assert!(e["profile"].is_object());
        assert!(e["profile"]["self_intro"].is_string());
    }
}

/// `none` 是哨兵而不是预设；提示文案必须把哨兵和全部可选值都列出来（错误文案要能照着改）。
#[test]
fn none_is_a_sentinel_not_a_preset() {
    assert!(get(NONE_ID).is_none());
    assert!(is_none(NONE_ID));
    assert!(is_none(" none "));
    assert!(!is_none(DEFAULT_ID));
    assert!(get("xiaozhi-typo").is_none());
    let hint = ids_hint();
    assert!(hint.contains(NONE_ID), "{hint}");
    for p in PRESETS {
        assert!(hint.contains(p.id), "{hint}");
    }
}
