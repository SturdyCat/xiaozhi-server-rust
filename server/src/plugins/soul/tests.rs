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
    let mut s = enabled_soul();
    s.name = "  ".into();
    assert!(format!("{:#}", s.validate().unwrap_err()).contains("name"));
    let mut s = enabled_soul();
    s.address_user = String::new();
    assert!(format!("{:#}", s.validate().unwrap_err()).contains("address_user"));
    let mut s = enabled_soul();
    s.self_intro = "我是 {{nickname}}".into();
    assert!(format!("{:#}", s.validate().unwrap_err()).contains("未声明的变量"));
    // 关闭时全部豁免（老配置不该因为新段的存在而无法启动）
    let off = SoulConfig {
        max_sentences: 0,
        name: String::new(),
        ..SoulConfig::default()
    };
    off.validate().unwrap();
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
