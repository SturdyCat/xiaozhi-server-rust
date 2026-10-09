//! `server/src/plugins/command/mod.rs` 的单元测试。
//!
//! 外移自该文件（AGENTS.md §5.10：测试一律独立文件）。
//! 断言锚在**行为**上：命中/不命中的边界（「退下吧」要命中，「关闭闹钟」不能命中）、
//! **长度闸门**（超过 5 个字的长句一律放行）、默认关、词表归一化与去重、
//! 以及"空词表绝不拦截一切"这条安全底线。

use super::*;

fn enabled_cfg() -> CommandConfig {
    CommandConfig {
        enabled: true,
        ..CommandConfig::default()
    }
}

/// 默认配置必须**不启用**：加插件不改变现有部署的任何行为。
#[test]
fn disabled_by_default_never_matches() {
    let cfg = CommandConfig::default();
    assert!(!cfg.enabled);
    assert!(!cfg.active());
    let gate = CommandGate::new(&cfg);
    assert_eq!(gate.hit("退下"), None);
    // 默认词表就是需求里那三个
    assert_eq!(cfg.keywords, vec!["退下", "闭嘴", "关闭"]);
    assert!(!cfg.reply.trim().is_empty(), "默认应有一句告别语");
    // 默认长度闸门 = 5 个字
    assert_eq!(cfg.max_chars, DEFAULT_MAX_CHARS);
    assert_eq!(cfg.max_chars, 5);
}

/// 归一化：标点/空白/语气助词都要能容忍（语音识别几乎总是带标点）。
#[test]
fn normalize_strips_punctuation_space_and_particles() {
    assert_eq!(normalize_utterance("退下。"), "退下");
    assert_eq!(normalize_utterance(" 退下吧 "), "退下");
    assert_eq!(normalize_utterance("退下吧！"), "退下");
    assert_eq!(normalize_utterance("闭嘴啊"), "闭嘴");
    assert_eq!(normalize_utterance("关闭了"), "关闭");
    assert_eq!(normalize_utterance("关闭。"), "关闭");
    assert_eq!(normalize_utterance("退下……"), "退下");
    // 词内语气字不会被误删（只剥句末）
    assert_eq!(normalize_utterance("了解情况"), "了解情况");
    assert_eq!(normalize_utterance("hello!"), "hello");
    assert_eq!(normalize_utterance("🐶退下"), "退下");
}

/// `exact`（默认）只认整句：不会误伤「关闭闹钟」这类正常请求。
#[test]
fn exact_mode_matches_whole_utterance_only() {
    let gate = CommandGate::new(&enabled_cfg());
    for ok in ["退下", "退下吧。", "闭嘴", "关闭", "关闭了"] {
        assert!(gate.hit(ok).is_some(), "应命中: {ok}");
    }
    for no in [
        "关闭闹钟",
        "把灯关闭",
        "先退下吧",
        "小智退下",
        "退下之后干嘛",
    ] {
        assert!(gate.hit(no).is_none(), "不应命中（exact）: {no}");
    }
    // 空/纯标点/纯静音不命中
    assert_eq!(gate.hit(""), None);
    assert_eq!(gate.hit("。。。"), None);
}

/// `contains` 是显式放宽：用户自己承担「关闭闹钟」被命中的后果（配置 help 已写明）。
#[test]
fn contains_mode_matches_substring() {
    let cfg = CommandConfig {
        match_mode: MatchMode::Contains,
        ..enabled_cfg()
    };
    let gate = CommandGate::new(&cfg);
    assert!(gate.hit("把灯关闭").is_some());
    assert!(gate.hit("关闭闹钟").is_some());
    assert!(gate.hit("退下吧").is_some());
    assert!(gate.hit("今天天气不错").is_none());
}

/// **长度闸门**：只有"短话"才做指令判断——长句携带信息，绝不能因为里面出现指令词就断线。
#[test]
fn long_utterance_skips_command_matching() {
    // 用最危险的 contains 模式验证：长度闸门是独立于匹配方式的第二道保险
    let cfg = CommandConfig {
        match_mode: MatchMode::Contains,
        ..enabled_cfg()
    };
    let gate = CommandGate::new(&cfg);
    // 恰好 5 个字 = 上限之内，照常命中
    assert_eq!(
        gate.hit("关闭一下灯").map(|h| h.keyword),
        Some("关闭".to_string())
    );
    // 6 个字起 = 放行给 LLM（不再断开）
    for long in [
        "关闭一下灯具",
        "把卧室的灯关闭",
        "帮我关闭卧室的灯",
        "请帮我关闭卧室的灯",
        "退下吧我知道了",
    ] {
        assert_eq!(gate.hit(long), None, "超过 5 字不应命中: {long}");
    }
}

/// 计数口径 = 归一化**之后**（与匹配一致）：标点、空白、句末语气词都不算字。
#[test]
fn length_gate_counts_normalized_chars_only() {
    // contains 模式：若没有长度闸门，下面三条都会命中
    let cfg = CommandConfig {
        match_mode: MatchMode::Contains,
        ..enabled_cfg()
    };
    let gate = CommandGate::new(&cfg);
    // 原始 8 个字符，但归一化后是「退下」= 2 个字 → 照常命中
    assert!(gate.hit("退下！！！！！！！").is_some());
    assert!(gate.hit(" 退 下 吧 吧 吧 ").is_some());
    // 长句（归一化后 9 个字 = 请退下之后把灯关掉）放行，不再误断
    assert_eq!(gate.hit("请退下之后把灯关掉"), None);
}

/// `max_chars = 0` = 不限制（显式拆掉这道保险；不是"什么都不拦"）。
#[test]
fn max_chars_zero_means_unlimited() {
    let cfg = CommandConfig {
        match_mode: MatchMode::Contains,
        max_chars: 0,
        ..enabled_cfg()
    };
    let gate = CommandGate::new(&cfg);
    assert!(gate.hit("请帮我把卧室的灯关闭一下好吗").is_some());
}

/// **静默失效护栏**：指令词比 `max_chars` 还长时永远命中不了，必须能被说成"配置问题"。
#[test]
fn validate_rejects_keyword_longer_than_limit() {
    let cfg = CommandConfig {
        keywords: vec!["关闭卧室的灯".into()],
        max_chars: 5,
        ..enabled_cfg()
    };
    let err = cfg.validate().unwrap_err().to_string();
    assert!(err.contains("max_chars"), "报错要指明是哪个字段: {err}");
    assert!(err.contains("6"), "报错要说清差多少: {err}");
    // 0 = 不限制时不报
    let unlimited = CommandConfig {
        keywords: vec!["关闭卧室的灯".into()],
        max_chars: 0,
        ..enabled_cfg()
    };
    unlimited.validate().unwrap();
    // 没启用时只是"躺在配置里"，不该拦启动
    let off = CommandConfig {
        keywords: vec!["关闭卧室的灯".into()],
        ..CommandConfig::default()
    };
    off.validate().unwrap();
}

/// 命中的 `CommandHit` 带上指令词与告别语（会话层据此说告别语 + 记日志）。
#[test]
fn hit_reports_keyword_and_reply() {
    let cfg = CommandConfig {
        reply: "好的，告退。".into(),
        ..enabled_cfg()
    };
    let gate = CommandGate::new(&cfg);
    let hit = gate.hit("退下吧。").expect("应命中");
    assert_eq!(hit.keyword, "退下");
    assert_eq!(hit.reply, "好的，告退。");
    // 自定义词表生效（默认词不再命中）
    let custom = CommandConfig {
        keywords: vec!["别说了".into()],
        ..enabled_cfg()
    };
    let gate2 = CommandGate::new(&custom);
    assert!(gate2.hit("别说了").is_some());
    assert!(gate2.hit("退下").is_none());
}

/// **安全底线**：词表为空时绝不拦截任何话。
/// （若按"空词表 = 命中一切"处理，用户清空词表保存一次就会把所有会话立刻断掉。）
#[test]
fn empty_keywords_never_intercept() {
    let cfg = CommandConfig {
        keywords: vec![],
        ..enabled_cfg()
    };
    assert!(!cfg.active(), "启用但词表为空 = 不生效");
    let gate = CommandGate::new(&cfg);
    assert_eq!(gate.hit("退下"), None);
    assert_eq!(gate.hit("随便说点什么"), None);
    // 配置问题必须能被上报（而不是静默不生效）
    let err = cfg.validate().unwrap_err().to_string();
    assert!(err.contains("keywords"), "{err}");
    assert!(err.contains("enabled"), "报错要说清怎么改: {err}");
}

/// `normalize()` 幂等 + 清洗：整行去空白、去标点、去重、丢空项；默认配置原样通过。
#[test]
fn normalize_is_idempotent_and_dedupes() {
    let mut cfg = CommandConfig {
        keywords: vec![" 退下 ".into(), "退下吧".into(), "".into(), "闭嘴！".into()],
        reply: "  好的  ".into(),
        ..CommandConfig::default()
    };
    cfg.normalize();
    assert_eq!(
        cfg.keywords,
        vec!["退下", "闭嘴"],
        "归一化后应去重（退下吧 → 退下）"
    );
    assert_eq!(cfg.reply, "好的");
    let once = cfg.keywords.clone();
    cfg.normalize();
    assert_eq!(cfg.keywords, once, "normalize 必须幂等");

    let mut d = CommandConfig::default();
    let before = d.signature();
    d.normalize();
    assert_eq!(d.signature(), before, "默认配置经 normalize 不应变化");
}

/// 签名随关键字段变化（热切换/状态上报口径）。
#[test]
fn signature_changes_with_config() {
    let a = CommandConfig::default().signature();
    let b = enabled_cfg().signature();
    assert_ne!(a, b);
    let c = CommandConfig {
        keywords: vec!["退下".into()],
        ..enabled_cfg()
    };
    assert_ne!(b, c.signature());
    // max_chars 会改变拦截行为，改了它必须被视为配置变化（否则热切换/状态上报口径会漏）
    let d = CommandConfig {
        max_chars: 3,
        ..enabled_cfg()
    };
    assert_ne!(b, d.signature());
}
