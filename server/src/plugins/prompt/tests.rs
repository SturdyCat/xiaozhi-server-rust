//! `plugins/prompt` 的回归测试（逻辑见 `mod.rs`；测试与逻辑分文件见 `AGENTS.md` §5.10）。

use super::*;

fn vars() -> Vars {
    let mut v = Vars::new();
    v.set("name", "小智");
    v.set("address_user", "你");
    v.set("device_id", "dev-1");
    v
}

#[test]
fn assemble_sorts_by_order_then_name_and_drops_empty() {
    let sections = vec![
        PromptSection::new(ORDER_SOUL_SUFFIX, "soul_suffix", "后缀"),
        PromptSection::new(ORDER_PLATFORM, "platform", "平台"),
        PromptSection::new(ORDER_SOUL_PREFIX, "soul_prefix", "人格"),
        PromptSection::new(ORDER_EXTRA, "extra_empty", "   "),
    ];
    let out = assemble(&sections, &vars()).unwrap();
    assert_eq!(out, "平台\n\n人格\n\n后缀");
}

#[test]
fn assemble_is_deterministic_for_equal_orders() {
    let a = vec![
        PromptSection::new(10, "b", "BBB"),
        PromptSection::new(10, "a", "AAA"),
    ];
    assert_eq!(assemble(&a, &vars()).unwrap(), "AAA\n\nBBB");
    // 输入顺序反转后结果不变（同 order 用 name 兜底）
    let b = vec![
        PromptSection::new(10, "a", "AAA"),
        PromptSection::new(10, "b", "BBB"),
    ];
    assert_eq!(assemble(&b, &vars()).unwrap(), "AAA\n\nBBB");
}

#[test]
fn interpolate_replaces_declared_vars_and_does_not_rescan() {
    let v = vars();
    assert_eq!(
        interpolate("t", "你好 {{name}}，{{ address_user }} 好", &v).unwrap(),
        "你好 小智，你 好"
    );
    // 变量值里出现 `{{...}}` 不再二次展开
    let mut v2 = Vars::new();
    v2.set("name", "{{address_user}}");
    assert_eq!(
        interpolate("t", "{{name}}", &v2).unwrap(),
        "{{address_user}}"
    );
}

#[test]
fn interpolate_treats_lone_delimiters_as_text() {
    // 孤立的 `{{`（后面没有 `}}`）按普通文本处理；单独的 `}}` 也一样
    let out = interpolate("t", "花括号 {{ 单边", &vars()).unwrap();
    assert_eq!(out, "花括号 {{ 单边");
    let out = interpolate("t", "闭括号 }} 单边", &vars()).unwrap();
    assert_eq!(out, "闭括号 }} 单边");
}

#[test]
fn interpolate_rejects_unknown_and_malformed_vars() {
    let err = interpolate("persona", "{{nickname}}", &vars()).unwrap_err();
    let msg = format!("{err:#}");
    assert!(msg.contains("未声明的变量"), "{msg}");
    assert!(msg.contains("nickname"), "{msg}");
    assert!(
        msg.contains("KNOWN_VARS") || msg.contains("可用变量"),
        "{msg}"
    );

    let err = interpolate("persona", "{{a}b}}", &vars()).unwrap_err();
    assert!(format!("{err:#}").contains("畸形变量"), "{err:#}");

    let err = interpolate("persona", "{{}}", &vars()).unwrap_err();
    assert!(format!("{err:#}").contains("空变量"), "{err:#}");
}

#[test]
fn interpolate_rejects_declared_var_without_value() {
    // 声明了但当前上下文没给值：同样是"吵闹地失败"，而不是渲染出空串
    let empty = Vars::new();
    let err = interpolate("persona", "{{device_id}}", &empty).unwrap_err();
    let msg = format!("{err:#}");
    assert!(msg.contains("没有它的值"), "{msg}");
    assert!(!msg.contains("未声明的变量"), "{msg}");
}

#[test]
fn literal_section_keeps_braces_verbatim() {
    let sections = vec![PromptSection::literal(
        ORDER_MEMORY,
        "memory",
        "用户说过 {{未声明}}",
    )];
    assert_eq!(assemble(&sections, &vars()).unwrap(), "用户说过 {{未声明}}");
    // 但校验同样放行（literal 段不参与插值纪律）
    validate_sections(&sections).unwrap();
}

#[test]
fn validate_sections_catches_unknown_var_at_startup() {
    let bad = vec![PromptSection::new(
        ORDER_SOUL_PREFIX,
        "soul_prefix",
        "我是 {{nick}}",
    )];
    assert!(validate_sections(&bad).is_err());
    let ok = vec![PromptSection::new(
        ORDER_SOUL_PREFIX,
        "soul_prefix",
        "我是 {{name}}",
    )];
    validate_sections(&ok).unwrap();
}

#[test]
fn memory_position_parses_with_after_history_default() {
    assert_eq!(
        MemoryPosition::parse("before_history"),
        MemoryPosition::BeforeHistory
    );
    assert_eq!(
        MemoryPosition::parse("after_history"),
        MemoryPosition::AfterHistory
    );
    assert_eq!(MemoryPosition::parse(""), MemoryPosition::AfterHistory);
    assert_eq!(
        MemoryPosition::parse("whatever"),
        MemoryPosition::AfterHistory
    );
    assert_eq!(MemoryPosition::default(), MemoryPosition::AfterHistory);
}
