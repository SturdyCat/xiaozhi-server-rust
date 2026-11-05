//! 扩展点解析/回退 + P1a 替换演练（官方默认 vs 用户自建）与 kv 权限对照。

use super::{fast_limits, fixture};
use crate::plugins::wasm::resolver::{FlatWorld, ResolutionTable};
use crate::plugins::wasm::{Resolution, WasmHost};

#[test]
fn extension_falls_back_to_official_when_preferred_unavailable() {
    let host = WasmHost::new(fast_limits()).expect("host");
    host.load("hello.official", &fixture("hello"))
        .expect("load");
    // 首选一个未装载的"用户插件" → 回退官方默认
    host.set_extension_preference("demo.greeter", Some("greeter.usermade".into()));
    let r = host.resolve("demo.greeter");
    match r {
        crate::plugins::wasm::Resolution::Fallback {
            official,
            preferred,
            ..
        } => {
            assert_eq!(official, "hello.official");
            assert_eq!(preferred, "greeter.usermade");
        }
        other => panic!("应回退官方默认：{other:?}"),
    }
    // 快照同时可见回退原因
    let snap = host.snapshot_json();
    assert_eq!(snap["extensions"]["demo.greeter"]["state"], "fallback");
    assert_eq!(
        snap["extensions"]["demo.greeter"]["implementation"],
        "hello.official"
    );
}

#[test]
fn extension_unresolved_without_any_implementation() {
    let host = WasmHost::new(fast_limits()).expect("host");
    let r = host.resolve("asr.provider");
    assert!(
        matches!(r, crate::plugins::wasm::Resolution::Unresolved(_)),
        "{r:?}"
    );
}

#[test]
fn resolver_table_semantics_on_flat_world() {
    let mut world = FlatWorld::default();
    world.insert("asr.official", true, vec!["asr.provider"], true);
    world.insert("asr.usermade", true, vec!["asr.provider"], false);
    let mut table = ResolutionTable::new();

    // 无首选：唯一官方默认直接顶上
    assert_eq!(
        table.resolve("asr.provider", &world),
        crate::plugins::wasm::Resolution::Resolved("asr.official".into())
    );

    // 首选用户插件可用 → 解析到用户插件
    table.set_preference("asr.provider", Some("asr.usermade".into()));
    assert_eq!(
        table.resolve("asr.provider", &world),
        crate::plugins::wasm::Resolution::Resolved("asr.usermade".into())
    );

    // 用户插件失效（未激活）→ 回退官方默认
    world.insert("asr.usermade", false, vec!["asr.provider"], false);
    match table.resolve("asr.provider", &world) {
        crate::plugins::wasm::Resolution::Fallback { official, .. } => {
            assert_eq!(official, "asr.official")
        }
        other => panic!("应回退：{other:?}"),
    }
}

// ============================================================
// P1：kv 读回闭环 / deny-as-absent / 用户插件替换演练
// ============================================================

#[test]
fn hello_kv_roundtrip_via_host_import() {
    let host = WasmHost::new(fast_limits()).expect("host");
    host.load("hello.official", &fixture("hello"))
        .expect("load");

    // 事件 → kv-put 效果（写）
    host.handle_event("hello.official", "demo.kv", "杭州 25 度")
        .expect("kv event");
    // 动作 → kv-get 宿主导入（读）：真实跨 wasm 边界的读写闭环
    let out = host
        .handle_action("hello.official", "recall", "")
        .expect("recall");
    assert!(out.contains("杭州 25 度"), "应读回事件写入的值：{out}");
}

#[test]
fn kv_get_denied_as_absent_without_permission() {
    let host = WasmHost::new(fast_limits()).expect("host");
    host.load("greeter.usermade", &fixture("greeter-user"))
        .expect("load");

    // 无 kv 权限：宿主导入返回 none（deny-as-absent），插件当「没有缓存」处理——
    // 不是装载失败、不是运行期错误、不降级
    let out = host
        .handle_action("greeter.usermade", "recall", "")
        .expect("recall 不应失败");
    assert!(
        out.contains("deny-as-absent") || out.contains("无缓存"),
        "温和拒绝形态：{out}"
    );
    assert_eq!(
        host.snapshot_json()["plugins"][0]["phase"]["state"],
        "active"
    );
}

#[test]
fn usermade_plugin_replaces_official_and_falls_back_on_unload() {
    let host = WasmHost::new(fast_limits()).expect("host");
    host.load("hello.official", &fixture("hello"))
        .expect("load");
    host.load("greeter.usermade", &fixture("greeter-user"))
        .expect("load usermade");

    // ① 无首选：唯一官方默认直接解析
    assert!(matches!(
        host.resolve("demo.greeter"),
        crate::plugins::wasm::Resolution::Resolved(ref id) if id == "hello.official"
    ));

    // ② 用户插件设为首选 → 解析切换到用户实现；两实现动作可各自调用（真实 wasm）
    host.set_extension_preference("demo.greeter", Some("greeter.usermade".into()));
    assert!(matches!(
        host.resolve("demo.greeter"),
        crate::plugins::wasm::Resolution::Resolved(ref id) if id == "greeter.usermade"
    ));
    let official = host
        .handle_action("hello.official", "greet", "小智")
        .expect("official");
    let usermade = host
        .handle_action("greeter.usermade", "greet", "小智")
        .expect("usermade");
    assert!(official.contains("你好"), "官方默认输出：{official}");
    assert!(usermade.contains("用户插件"), "用户实现输出：{usermade}");

    // ③ 卸载用户插件（注册-回收）→ 扩展点实时回退官方默认，快照可见回退原因
    assert!(host.unload("greeter.usermade"), "卸载应成功");
    match host.resolve("demo.greeter") {
        crate::plugins::wasm::Resolution::Fallback {
            official,
            preferred,
            ..
        } => {
            assert_eq!(official, "hello.official");
            assert_eq!(preferred, "greeter.usermade");
        }
        other => panic!("应回退官方默认：{other:?}"),
    }
    let snap = host.snapshot_json();
    assert_eq!(snap["extensions"]["demo.greeter"]["state"], "fallback");
    assert_eq!(
        snap["extensions"]["demo.greeter"]["implementation"],
        "hello.official"
    );

    // ④ 用户插件 kv 已随卸载回收（注册-回收语义）
    assert_eq!(host.kv().len_for("greeter.usermade"), 0);
}

// ============================================================
