//! `plugins/soul` 的回归测试（逻辑见 `mod.rs`；测试与逻辑分文件见 `AGENTS.md` §5.10）。

use super::*;
use crate::config::Config;

fn enabled_soul() -> SoulConfig {
    SoulConfig {
        enabled: true,
        name: "小智".into(),
        self_intro: "我是住在音箱里的小助手。".into(),
        traits: vec!["好奇：遇到新话题会追问一句".into(), "  ".into()],
        values: vec!["不编造事实".into()],
        boundaries: vec!["不聊血腥细节".into()],
        tone: "轻快".into(),
        address_user: "你".into(),
        examples: vec!["用户：今天几度？\n小智：我这边看不到天气，你手机上看一眼？".into()],
        ..SoulConfig::default()
    }
}

#[test]
fn disabled_soul_is_byte_identical_to_system_prompt() {
    let cfg = Config::default();
    let out = instructions(&cfg, "dev-1").unwrap();
    assert_eq!(out, cfg.llm.system_prompt, "关闭人格必须与现状逐字节一致");
    // 空 system_prompt 也不会拼出噪音
    let mut cfg2 = Config::default();
    cfg2.llm.system_prompt = String::new();
    assert_eq!(instructions(&cfg2, "dev-1").unwrap(), "");
}

#[test]
fn enabled_soul_renders_persona_and_platform_constraints() {
    let mut cfg = Config::default();
    cfg.soul = enabled_soul();
    cfg.soul.max_sentences = 3;
    cfg.soul.max_chars = 60;
    let out = instructions(&cfg, "dev-1").unwrap();
    assert!(out.starts_with("【语音对话的基本约束】"), "{out}");
    assert!(out.contains("最多 3 句、不超过 60 个字"), "{out}");
    assert!(out.contains("你的名字是「小智」"), "{out}");
    assert!(out.contains("我是住在音箱里的小助手。"), "{out}");
    assert!(out.contains("- 好奇：遇到新话题会追问一句"), "{out}");
    assert!(out.contains("## 你坚持的价值"), "{out}");
    assert!(out.contains("## 你不会做的事\n- 不聊血腥细节"), "{out}");
    assert!(out.contains("用户：今天几度？"), "{out}");
    // 平台段在人格段之前（order -1000 < 0）
    let platform_at = out.find("【语音对话的基本约束】").unwrap();
    let persona_at = out.find("# 你是谁").unwrap();
    assert!(platform_at < persona_at);
    // 空的 traits 项被过滤，不留空 bullet
    assert!(!out.contains("- \n"), "{out:?}");
    // 附加约束（旧 system_prompt）仍在，且排在人格之后
    assert!(out.contains(&cfg.llm.system_prompt), "{out}");
}

#[test]
fn enabled_soul_interpolates_name_and_address_user() {
    let mut cfg = Config::default();
    cfg.soul = enabled_soul();
    cfg.soul.self_intro = "我叫 {{name}}，你可以叫我 {{name}}。".into();
    cfg.soul.tone = "对 {{address_user}} 友好".into();
    let out = instructions(&cfg, "dev-1").unwrap();
    assert!(out.contains("我叫 小智，你可以叫我 小智。"), "{out}");
    assert!(out.contains("对 你 友好"), "{out}");
    assert!(!out.contains("{{"), "不应残留未展开变量: {out}");
}

#[test]
fn validate_rejects_bad_ranges_and_unknown_vars() {
    let mut s = enabled_soul();
    s.max_sentences = 0;
    assert!(format!("{:#}", s.validate().unwrap_err()).contains("max_sentences"));
    let mut s = enabled_soul();
    s.max_chars = 9999;
    assert!(format!("{:#}", s.validate().unwrap_err()).contains("max_chars"));
    // 空 `name`/`address_user` 只在**没有预设兜底**时才是错误（preset = "none"）
    let mut s = enabled_soul();
    s.preset = presets::NONE_ID.into();
    s.name = "  ".into();
    assert!(format!("{:#}", s.validate().unwrap_err()).contains("name"));
    let mut s = enabled_soul();
    s.preset = presets::NONE_ID.into();
    s.address_user = String::new();
    assert!(format!("{:#}", s.validate().unwrap_err()).contains("address_user"));
    let mut s = enabled_soul();
    s.self_intro = "我是 {{nickname}}".into();
    assert!(format!("{:#}", s.validate().unwrap_err()).contains("未声明的变量"));
    // 关闭时全部豁免（老配置不该因为新段的存在而无法启动；写错的 preset 同理）
    let off = SoulConfig {
        max_sentences: 0,
        name: String::new(),
        preset: "typo-not-a-preset".into(),
        ..SoulConfig::default()
    };
    off.validate().unwrap();
}

// ============================================================
// 内置默认灵魂（preset）：留空=用默认、填写=逐字段覆盖、none=完全自定义
// ============================================================

#[test]
fn blank_fields_fall_back_to_default_preset() {
    // 只打开开关、一个字段都不填：开箱就该得到一份像样的人格（而不是空壳或报错）
    let mut cfg = Config::default();
    cfg.soul = SoulConfig {
        enabled: true,
        name: String::new(),         // 连自称都不填
        address_user: String::new(), // 连称呼都不填
        ..SoulConfig::default()
    };
    cfg.soul.normalize();
    let out = instructions(&cfg, "dev-1").unwrap();
    assert!(out.contains("【语音对话的基本约束】"), "{out}");
    assert!(out.contains("你的名字是「小智」"), "预设的自称应补上：{out}");
    assert!(out.contains("音箱里的小助手"), "预设的 self_intro 应补上：{out}");
    assert!(out.contains("- 称呼用户为「你」。"), "预设的称呼应补上：{out}");
    assert!(out.contains("## 你不会做的事"), "{out}");
    assert!(!out.contains("{{"), "不应残留未展开变量: {out}");
    // 校验也该通过（空 name 不再是错误——它由预设兜底）
    cfg.soul.validate().unwrap();
}

#[test]
fn filled_fields_override_preset_one_by_one() {
    let mut cfg = Config::default();
    cfg.soul = SoulConfig {
        enabled: true,
        tone: "我自己填的语气".into(),
        self_intro: "我自己写的定位。".into(),
        ..SoulConfig::default()
    };
    let out = instructions(&cfg, "dev-1").unwrap();
    // 填了的字段用用户的
    assert!(out.contains("语气：我自己填的语气。"), "{out}");
    assert!(out.contains("我自己写的定位。"), "{out}");
    // 没填的字段仍来自预设
    assert!(out.contains("- 说真话：不确定的地方标出来"), "{out}");
    assert!(out.contains("音箱里的小助手"), "{out}");
    // 被覆盖的预设文本不再出现
    assert!(!out.contains("轻快、温和、带一点点好奇"), "{out}");
}

#[test]
fn preset_none_means_no_baseline() {
    let mut cfg = Config::default();
    cfg.soul = SoulConfig {
        enabled: true,
        preset: presets::NONE_ID.into(),
        name: "小爱".into(),
        self_intro: "只有我自己写的。".into(),
        ..SoulConfig::default()
    };
    let out = instructions(&cfg, "dev-1").unwrap();
    assert!(out.contains("只有我自己写的。"), "{out}");
    assert!(!out.contains("音箱里的小助手"), "preset=none 不该注入预设内容：{out}");
    assert!(!out.contains("- 说真话：不确定的地方标出来"), "{out}");
    cfg.soul.validate().unwrap();
}

#[test]
fn unknown_preset_is_rejected_when_enabled_but_exempt_when_disabled() {
    let mut cfg = Config::default();
    cfg.soul = SoulConfig {
        enabled: true,
        preset: "xiaozi".into(),
        ..SoulConfig::default()
    };
    let err = format!("{:#}", cfg.soul.validate().unwrap_err());
    assert!(err.contains("xiaozi") && err.contains("xiaozhi") && err.contains("none"), "{err}");
    // 组装同样报错 → 会话层退回 system_prompt 并告警（绝不静默换成人格）
    assert!(instructions(&cfg, "dev-1").is_err());
    cfg.soul.enabled = false;
    cfg.soul.validate().unwrap();
    assert_eq!(instructions(&cfg, "dev-1").unwrap(), cfg.llm.system_prompt);
}

#[test]
fn effective_never_lets_preset_change_booleans_or_numbers() {
    let mut cfg = Config::default();
    cfg.soul = SoulConfig {
        enabled: true,
        emoji: true, // 预设是 false，用户开了就必须保持开着
        colloquial: false,
        max_sentences: 5,
        max_chars: 120,
        ..SoulConfig::default()
    };
    let eff = cfg.soul.effective().unwrap();
    assert!(eff.emoji, "预设不得覆盖用户的布尔字段");
    assert!(!eff.colloquial);
    assert_eq!(eff.max_sentences, 5);
    assert_eq!(eff.max_chars, 120);
    // 有效档案本身仍是"启用"状态，且 preset 未被改写
    assert!(eff.enabled);
    assert_eq!(eff.preset, presets::DEFAULT_ID);
    // 幂等：对已合并的结果再合并一次，结果不变
    assert_eq!(eff.signature(), eff.effective().unwrap().signature());
}

#[test]
fn preset_normalizes_empty_to_default_and_keeps_explicit_none() {
    let mut s = SoulConfig {
        preset: "  ".into(),
        ..SoulConfig::default()
    };
    assert_eq!(s.preset_id(), presets::DEFAULT_ID);
    s.normalize();
    assert_eq!(s.preset, presets::DEFAULT_ID);
    let mut none = SoulConfig {
        preset: " none ".into(),
        ..SoulConfig::default()
    };
    none.normalize();
    assert_eq!(none.preset, presets::NONE_ID, "显式 none 不能被折成默认预设");
    assert!(presets::is_none(none.preset_id()));
}

#[test]
fn default_config_uses_the_default_preset() {
    let s = SoulConfig::default();
    assert_eq!(s.preset, presets::DEFAULT_ID);
    assert!(!s.enabled, "默认仍不启用人格（关掉=回到今天）");
    // 老配置（**没有** preset 键）必须落到内置默认灵魂，而不是"没有基线"——升级不改行为的前提
    let legacy: SoulConfig = serde_json::from_str(r#"{"enabled":true}"#).unwrap();
    assert_eq!(legacy.preset, presets::DEFAULT_ID);
    let explicit: SoulConfig =
        serde_json::from_str(r#"{"enabled":true,"preset":"none"}"#).unwrap();
    assert_eq!(explicit.preset, presets::NONE_ID);
}

#[test]
fn emoji_defaults_off_and_is_stated_in_prompt() {
    let mut cfg = Config::default();
    cfg.soul = enabled_soul();
    let out = instructions(&cfg, "dev-1").unwrap();
    assert!(out.contains("不要用 emoji"), "{out}");
    cfg.soul.emoji = true;
    let out = instructions(&cfg, "dev-1").unwrap();
    assert!(out.contains("可以用 emoji"), "{out}");
    assert!(!SoulConfig::default().emoji, "语音默认必须关闭 emoji");
}
