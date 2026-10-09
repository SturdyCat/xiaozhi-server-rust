//! `app/ws/soul` 的端点逻辑测试（只测纯函数 `preview_body`；HTTP 层由 axum 路由保证）。
//! 测试与逻辑分文件见 `AGENTS.md` §5.10。

use super::*;
use serde_json::json;

fn live_with_soul(soul: SoulConfig) -> crate::config::Config {
    let mut cfg = crate::config::Config::default();
    cfg.soul = soul;
    cfg.normalize();
    cfg
}

#[test]
fn preview_reports_system_prompt_when_soul_disabled() {
    let cfg = live_with_soul(SoulConfig::default());
    let body = preview_body(&cfg, &json!({})).unwrap();
    assert_eq!(body["ok"], true);
    assert_eq!(body["enabled"], false);
    assert_eq!(body["source"], "system_prompt");
    assert_eq!(body["instructions"], cfg.llm.system_prompt);
    assert!(
        body["note"].as_str().unwrap().contains("未启用"),
        "{}",
        body["note"]
    );
    // 未启用时不做"字段来源"统计（"来自预设"没有意义，别误导）
    assert_eq!(body["from_preset"].as_array().unwrap().len(), 0);
    assert_eq!(body["overridden"].as_array().unwrap().len(), 0);
}

#[test]
fn preview_uses_default_preset_for_blank_draft() {
    // 草稿只打开开关，一个字段都不填 → 应当看到内置默认人格（这就是"默认灵魂"）
    let cfg = live_with_soul(SoulConfig::default());
    let body = preview_body(&cfg, &json!({ "soul": { "enabled": true } })).unwrap();
    assert_eq!(body["source"], "soul");
    assert_eq!(body["preset"], "xiaozhi");
    assert_eq!(body["preset_name"], "小智（内置默认人格）");
    let text = body["instructions"].as_str().unwrap();
    assert!(text.contains("【语音对话的基本约束】"), "{text}");
    assert!(text.contains("你的名字是「小智」"), "{text}");
    assert!(text.contains("音箱里的小助手"), "预设的 self_intro 应生效：{text}");
    assert!(body["approx_tokens"].as_u64().unwrap() > 0);
    assert_eq!(body["chars"].as_u64().unwrap() as usize, text.chars().count());
    // 字段来源：全部由预设补上
    let from_preset: Vec<String> = serde_json::from_value(body["from_preset"].clone()).unwrap();
    for k in ["self_intro", "tone", "traits", "examples"] {
        assert!(from_preset.iter().any(|f| f == k), "{k} 应来自预设：{from_preset:?}");
    }
    assert_eq!(body["overridden"].as_array().unwrap().len(), 0);
    // 分段视图：order 升序，且含平台约束与人格段
    let segs = body["segments"].as_array().unwrap();
    let orders: Vec<i64> = segs.iter().map(|s| s["order"].as_i64().unwrap()).collect();
    assert_eq!(orders, {
        let mut sorted = orders.clone();
        sorted.sort();
        sorted
    });
    let names: Vec<&str> = segs.iter().map(|s| s["name"].as_str().unwrap()).collect();
    assert!(names.contains(&"platform_constraints"), "{names:?}");
    assert!(names.contains(&"soul_prefix"), "{names:?}");
}

#[test]
fn preview_shows_field_level_override() {
    let cfg = live_with_soul(SoulConfig::default());
    let body = preview_body(
        &cfg,
        &json!({ "soul": { "enabled": true, "tone": "我自己填的语气", "traits": ["我自己的性格"] } }),
    )
    .unwrap();
    let text = body["instructions"].as_str().unwrap();
    assert!(text.contains("语气：我自己填的语气。"), "{text}");
    assert!(text.contains("- 我自己的性格"), "{text}");
    assert!(!text.contains("轻快、温和、带一点点好奇"), "被覆盖的预设字段不该出现：{text}");
    let overridden: Vec<String> = serde_json::from_value(body["overridden"].clone()).unwrap();
    assert!(overridden.contains(&"tone".to_string()), "{overridden:?}");
    assert!(overridden.contains(&"traits".to_string()), "{overridden:?}");
    let from_preset: Vec<String> = serde_json::from_value(body["from_preset"].clone()).unwrap();
    assert!(from_preset.contains(&"self_intro".to_string()), "{from_preset:?}");
    assert!(!from_preset.contains(&"tone".to_string()), "填了的字段不该算来自预设");
}

#[test]
fn preview_rejects_unknown_preset_with_actionable_message() {
    let cfg = live_with_soul(SoulConfig::default());
    let err = preview_body(&cfg, &json!({ "soul": { "enabled": true, "preset": "xiaozi" } }))
        .expect_err("未知 preset 必须报错（绝不静默回退默认）");
    assert!(err.contains("xiaozi"), "{err}");
    assert!(err.contains("xiaozhi"), "错误文案要列出可选值：{err}");
    assert!(err.contains("none"), "{err}");
}

#[test]
fn preview_rejects_malformed_draft() {
    let cfg = live_with_soul(SoulConfig::default());
    let err = preview_body(&cfg, &json!({ "soul": { "enabled": true, "traits": "多行字符串" } }))
        .expect_err("列表字段写成字符串应报错");
    assert!(err.contains("草稿格式不正确"), "{err}");
    assert!(err.contains("必须是数组"), "错误文案要含怎么修：{err}");
}

#[test]
fn preview_without_device_id_uses_placeholder() {
    let cfg = live_with_soul(SoulConfig::default());
    let body = preview_body(&cfg, &json!({ "soul": { "enabled": true, "device_hint": "设备 {{device_id}}" } }))
        .unwrap();
    assert_eq!(body["device_id"], "");
    assert!(
        body["instructions"].as_str().unwrap().contains("设备 <device_id>"),
        "{}",
        body["instructions"]
    );
    let body2 = preview_body(
        &live_with_soul(SoulConfig::default()),
        &json!({ "device_id": "dev-1", "soul": { "enabled": true, "device_hint": "设备 {{device_id}}" } }),
    )
    .unwrap();
    assert_eq!(body2["device_id"], "dev-1");
    assert!(body2["instructions"].as_str().unwrap().contains("设备 dev-1"));
}

/// 客户端「载入预设内容到表单」把 `effective` **整段**喂给表单，因此它必须是**完整档案**：
/// 少一个数组键，表单映射就会把它当成空数组、把用户原文冲掉（这正是复用 `fill` 的代价）。
#[test]
fn preview_returns_complete_effective_profile_for_form_load() {
    let cfg = live_with_soul(SoulConfig::default());
    let body = preview_body(
        &cfg,
        &json!({ "soul": { "enabled": true, "catchphrases": ["嗯……"] } }),
    )
    .unwrap();
    let eff = body["effective"].as_object().expect("effective 必须是对象");
    let all_keys: Vec<String> = serde_json::to_value(SoulConfig::default())
        .unwrap()
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect();
    for k in &all_keys {
        assert!(
            eff.contains_key(k),
            "effective 缺字段 {k}：客户端整段回填表单时会把它当成空值"
        );
    }
    assert_eq!(eff["catchphrases"][0], "嗯……", "用户填的字段必须保留");
    assert_eq!(eff["tone"], "轻快、温和、带一点点好奇；不油嘴滑舌，也不端着", "留空字段由预设补");
    assert_eq!(eff["enabled"], true);
    assert_eq!(eff["preset"], "xiaozhi");
}

#[test]
fn preview_none_preset_has_no_baseline() {
    let cfg = live_with_soul(SoulConfig::default());
    let body = preview_body(
        &cfg,
        &json!({ "soul": { "enabled": true, "preset": "none", "self_intro": "只有我自己写的" } }),
    )
    .unwrap();
    assert_eq!(body["preset"], "none");
    assert_eq!(body["preset_name"], "不使用预设（完全自定义）");
    let text = body["instructions"].as_str().unwrap();
    assert!(text.contains("只有我自己写的"), "{text}");
    assert!(!text.contains("音箱里的小助手"), "preset=none 不该出现预设内容：{text}");
    assert_eq!(body["from_preset"].as_array().unwrap().len(), 0);
}
