//! JSON UI/A2UI 契约测试（外置；A2UI 最小闭环的宿主侧回归）。
//!
//! 覆盖：WIT→DTO 转换、`registration.ui-json` 校验守门（合法 JSON 对象 + 上限）、
//! snapshot 投影的 Active 门控与 [`WasmHost::plugin_ui_json`] 的
//! 「未注册插件不返回业务 UI」语义。端点级回环测试在
//! `gateway/ws/plugins/tests.rs`（真实 HTTP + hello 夹具声明）。

use super::contract;
use super::WasmHost;
use crate::plugins::wasm::tests::{fast_limits, fixture};

use contract::xiaozhi::plugin::types as wit;

// ============================================================
// WIT → DTO 转换
// ============================================================

#[test]
fn wit_registration_ui_json_converts_to_dto() {
    let wit_reg = wit::Registration {
        menu: vec![],
        config_fields: vec![],
        actions: vec![],
        ui_json: Some(r#"{"a2ui":"0.1"}"#.to_string()),
    };
    let dto: contract::RegistrationDto = wit_reg.into();
    assert_eq!(dto.ui_json.as_deref(), Some(r#"{"a2ui":"0.1"}"#));
    assert!(contract::validate_registration(&dto).is_ok());

    // None（未声明 UI）是合法形状：menu/fields 兼容面不受影响
    let bare: contract::RegistrationDto = wit::Registration {
        menu: vec![],
        config_fields: vec![],
        actions: vec![],
        ui_json: None,
    }
    .into();
    assert_eq!(bare.ui_json, None);
    assert!(contract::validate_registration(&bare).is_ok());
}

// ============================================================
// UI 描述校验守门（与 guest SDK RegistrationBuilder::ui_json 同源）
// ============================================================

#[test]
fn ui_json_validation_accepts_object_and_rejects_bad_shapes() {
    let reg = |ui_json: Option<&str>| contract::RegistrationDto {
        ui_json: ui_json.map(str::to_string),
        ..Default::default()
    };
    // 合法：单个 JSON 对象（组件树/bindings 由插件自组织，宿主不解释内容）
    let descriptor = r#"{"a2ui":"0.1","pages":[{"id":"cfg","tree":{"type":"form"}}]}"#;
    assert!(contract::validate_registration(&reg(Some(descriptor))).is_ok());

    // 非法 JSON
    assert!(contract::validate_registration(&reg(Some("not json"))).is_err());
    // 顶层非对象（数组/数字/字符串/null 都拒绝——UI 描述必须整体可寻址）
    for schema in ["[]", "42", "\"s\"", "null", "true"] {
        assert!(
            contract::validate_registration(&reg(Some(schema))).is_err(),
            "顶层 {schema} 应拒绝"
        );
    }
    // 超大小上限
    let big = format!(r#"{{"pad":"{}"}}"#, " ".repeat(contract::MAX_UI_JSON_BYTES));
    assert!(contract::validate_registration(&reg(Some(&big))).is_err());
}

// ============================================================
// 投影语义：仅 Active 插件投影；未注册不返回业务 UI
// ============================================================

#[test]
fn hello_ui_json_projected_with_parsed_descriptor() {
    let host = WasmHost::new(fast_limits()).expect("host");
    host.load("hello.official", &fixture("hello"))
        .expect("load");

    // snapshot：registration.ui 是解析后的整份描述对象（Active 门控内）
    let snap = host.snapshot_json();
    let ui = &snap["plugins"][0]["registration"]["ui"];
    assert!(ui.is_object(), "Active 插件应投影解析后的 UI 对象：{ui}");
    assert_eq!(ui["a2ui"], "0.1");
    assert_eq!(
        ui["pages"].as_array().map(Vec::len),
        Some(2),
        "hello 夹具声明 config + test 两页：{ui}"
    );
    // menu/fields 兼容面不受 UI 改动影响
    assert_eq!(
        snap["plugins"][0]["registration"]["menu"][0]["slot"],
        "nav.main"
    );

    // 单插件只读投影同源
    let one = host.plugin_ui_json("hello.official").expect("已注册");
    assert_eq!(one["id"], "hello.official");
    assert_eq!(one["phase"], "active");
    assert_eq!(one["ui"]["a2ui"], "0.1");
    assert_eq!(one["ui"]["pages"][0]["bindings"]["config"][0], "greeting");
}

/// 嵌套组件树（A2UI 0.1 超集）：夹具两页均为 card→column 嵌套，test 页含
/// input/result 叶子与 actions id 数组——客户端基础组件词汇表的活样例。
#[test]
fn hello_ui_uses_nested_component_tree_with_input_and_result() {
    let host = WasmHost::new(fast_limits()).expect("host");
    host.load("hello.official", &fixture("hello"))
        .expect("load");
    let ui = &host.snapshot_json()["plugins"][0]["registration"]["ui"];

    // config 页：card → column[field, hint]
    let config = &ui["pages"][0];
    assert_eq!(config["tree"]["type"], "card", "嵌套容器树存在：{config}");
    assert_eq!(config["tree"]["title"], "问候配置");
    let column = &config["tree"]["child"];
    assert_eq!(column["type"], "column");
    assert_eq!(column["children"][0]["type"], "field");
    assert_eq!(column["children"][0]["key"], "greeting");
    assert_eq!(column["children"][1]["type"], "hint");

    // test 页：card → column[input, actions(id 数组), result]
    let test = &ui["pages"][1];
    assert_eq!(test["tree"]["type"], "card");
    let children = &test["tree"]["child"]["children"];
    assert_eq!(children[0]["type"], "input");
    assert_eq!(children[0]["id"], "name");
    assert_eq!(children[1]["type"], "actions");
    assert_eq!(
        children[1]["actions"].as_array().map(Vec::len),
        Some(1),
        "actions 是动作 id 字符串数组：{}",
        children[1]["actions"]
    );
    assert_eq!(children[1]["actions"][0], "greet");
    assert_eq!(children[2]["type"], "result");
    // bindings 兼容面：仍按动作 id 引用（与树内 actions 数组同源）
    assert_eq!(test["bindings"]["actions"][0], "greet");
}

#[test]
fn unregistered_plugin_returns_no_ui() {
    let host = WasmHost::new(fast_limits()).expect("host");
    // 完全空的注册表：任何 id 都不投影 UI（pure-base 未装载时 wasm.plugins=[] 同理）
    assert!(host.plugin_ui_json("ghost.plugin").is_none());
    assert_eq!(
        host.snapshot_json()["plugins"].as_array().map(Vec::len),
        Some(0),
        "未装载即无投影"
    );

    // 装载后再查询其他 id：仍为 None（投影严格按注册表条目）
    host.load("hello.official", &fixture("hello"))
        .expect("load");
    assert!(host.plugin_ui_json("ghost.plugin").is_none());

    // 卸载后 UI 投影随注册-回收消失
    assert!(host.unload("hello.official"));
    assert!(host.plugin_ui_json("hello.official").is_none());
}
