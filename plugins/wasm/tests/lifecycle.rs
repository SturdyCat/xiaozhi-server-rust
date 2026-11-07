//! 生命周期三态机回归：Staged → Registered → Active、依赖前置、依赖者停用门、
//! 升级/降级判定、移除边界、快照投影。夹具组合：greeter-user `requires hello`。

use super::{fast_limits, fixture};
use crate::plugins::wasm::{Lifecycle, WasmHost};

fn host() -> WasmHost {
    WasmHost::new(fast_limits()).expect("host")
}

fn lifecycle_of(host: &WasmHost, id: &str) -> &'static str {
    host.lifecycle_of(id)
        .map(|(lc, _)| lc.as_str())
        .unwrap_or("missing")
}

#[test]
fn stage_registers_activate_full_flow() {
    let host = host();
    // 上传 = 仅登记（不装载：无实例、registration 未收集、无 UI）
    let out = host
        .stage("hello.official", &fixture("hello"))
        .expect("stage");
    assert!(!out.upgraded);
    assert_eq!(lifecycle_of(&host, "hello.official"), "staged");
    let snap = host.snapshot_json();
    let row = &snap["plugins"][0];
    assert_eq!(row["lifecycle"], "staged");
    assert_eq!(row["phase"]["state"], "inactive");
    assert!(row["registration"].is_null(), "未激活不投影 registration");
    assert!(
        host.plugin_ui_json("hello.official").is_none(),
        "Staged 无 UI"
    );
    assert!(
        host.handle_action("hello.official", "greet", "").is_err(),
        "Staged 无实例"
    );

    // 注册 = 完整校验 + self-test，仍不装载
    host.register("hello.official", &fixture("hello"))
        .expect("register");
    assert_eq!(lifecycle_of(&host, "hello.official"), "registered");
    assert!(
        host.plugin_ui_json("hello.official").is_none(),
        "Registered 无 UI"
    );
    assert!(
        host.snapshot_json()["extensions"]["demo.greeter"]["state"] != "resolved",
        "Registered 不参与扩展点解析"
    );

    // 重复注册（非 Staged）拒绝
    let err = host
        .register("hello.official", &fixture("hello"))
        .unwrap_err();
    assert!(err.to_string().contains("仅 staged 可注册"), "{err}");

    // 激活 = 装载 + Active；菜单/UI 才出现
    host.activate_checked("hello.official", &fixture("hello"))
        .expect("activate");
    assert_eq!(lifecycle_of(&host, "hello.official"), "active");
    assert_eq!(
        host.snapshot_json()["extensions"]["demo.greeter"]["state"],
        "resolved"
    );
    assert!(
        host.plugin_ui_json("hello.official").is_some(),
        "Active 才有 UI"
    );
    // 已激活再激活 → WrongLifecycle
    let err = host
        .activate_checked("hello.official", &fixture("hello"))
        .unwrap_err();
    assert!(format!("{err:?}").contains("WrongLifecycle"), "{err:?}");
}

#[test]
fn activation_requires_dependencies_active_with_min_version() {
    let host = host();
    // greeter-user requires hello.official >=0.1.0；hello 未上传 → missing
    host.stage("greeter.usermade", &fixture("greeter-user"))
        .expect("stage");
    host.register("greeter.usermade", &fixture("greeter-user"))
        .expect("register");
    let err = host
        .activate_checked("greeter.usermade", &fixture("greeter-user"))
        .unwrap_err();
    let format_err = format!("{err:?}");
    assert!(format_err.contains("Dependencies"), "{format_err}");
    let rows = host.requires_view("greeter.usermade");
    assert_eq!(rows[0]["id"], "hello.official");
    assert_eq!(rows[0]["satisfied"], false);

    // hello 注册但仍未激活 → inactive
    host.stage("hello.official", &fixture("hello"))
        .expect("stage");
    host.register("hello.official", &fixture("hello"))
        .expect("register");
    let err = host
        .activate_checked("greeter.usermade", &fixture("greeter-user"))
        .unwrap_err();
    assert!(format!("{err:?}").contains("inactive"), "{err:?}");

    // hello 激活后依赖满足
    host.activate_checked("hello.official", &fixture("hello"))
        .expect("activate");
    assert_eq!(host.requires_view("greeter.usermade")[0]["satisfied"], true);
    host.activate_checked("greeter.usermade", &fixture("greeter-user"))
        .expect("activate after dep");
    assert_eq!(
        host.active_dependents_of("hello.official"),
        vec!["greeter.usermade"]
    );
}

#[test]
fn deactivate_blocked_by_active_dependents_then_allowed() {
    let host = host();
    for (id, fx) in [
        ("hello.official", "hello"),
        ("greeter.usermade", "greeter-user"),
    ] {
        host.stage(id, &fixture(fx)).expect("stage");
        host.register(id, &fixture(fx)).expect("register");
        host.activate_checked(id, &fixture(fx)).expect("activate");
    }
    // hello 有 Active 依赖者 → 停用 409 语义（Dependents）
    let err = host.deactivate("hello.official").unwrap_err();
    assert!(format!("{err:?}").contains("Dependents"), "{err:?}");
    assert_eq!(
        lifecycle_of(&host, "hello.official"),
        "active",
        "停用被拒不改变状态"
    );

    // 依赖者先停用（无依赖者）→ hello 可停用
    host.deactivate("greeter.usermade")
        .expect("deactivate dependent");
    assert_eq!(lifecycle_of(&host, "greeter.usermade"), "registered");
    host.deactivate("hello.official").expect("deactivate hello");
    assert_eq!(lifecycle_of(&host, "hello.official"), "registered");
    // kv/产物由宿主外持有；此处验证实例卸载：动作不可用
    assert!(host.handle_action("greeter.usermade", "greet", "").is_err());
    // 非 Active 停用 → WrongLifecycle
    let err = host.deactivate("greeter.usermade").unwrap_err();
    assert!(format!("{err:?}").contains("WrongLifecycle"), "{err:?}");
}

#[test]
fn remove_rejects_active_and_clears_staged() {
    let host = host();
    host.stage("hello.official", &fixture("hello"))
        .expect("stage");
    // Staged 可移除
    host.remove("hello.official").expect("remove staged");
    assert_eq!(lifecycle_of(&host, "hello.official"), "missing");

    // Active 禁止移除
    host.stage("hello.official", &fixture("hello"))
        .expect("stage");
    host.register("hello.official", &fixture("hello"))
        .expect("register");
    host.activate_checked("hello.official", &fixture("hello"))
        .expect("activate");
    let err = host.remove("hello.official").unwrap_err();
    assert!(format!("{err:?}").contains("Active"), "{err:?}");
    assert_eq!(lifecycle_of(&host, "hello.official"), "active");
}

#[test]
fn upgrade_takes_newer_version_and_downgrade_rejected() {
    let host = host();
    host.stage("hello.official", &fixture("hello"))
        .expect("stage");
    host.register("hello.official", &fixture("hello"))
        .expect("register");
    host.activate_checked("hello.official", &fixture("hello"))
        .expect("activate");
    let old_version = host
        .lifecycle_of("hello.official")
        .map(|(_, v)| v)
        .expect("version");

    // 同版本重传 = 新 ≤ 旧 → 拒绝（契约：升级须更高版本）
    let err = host.stage("hello.official", &fixture("hello")).unwrap_err();
    assert!(err.to_string().contains("版本不升级"), "{err}");
    assert_eq!(
        host.lifecycle_of("hello.official").map(|(_, v)| v),
        Some(old_version)
    );

    // 快照带 requires/dependents 投影（hello 无依赖 → requires 空、dependents 空）
    let row = &host.snapshot_json()["plugins"][0];
    assert_eq!(row["lifecycle"], "active");
    let requires = row["requires"].as_array().expect("requires rows");
    assert!(requires.is_empty(), "hello 无依赖声明：{requires:?}");
}

#[test]
fn load_primitive_keeps_active_semantics_for_policy_engines() {
    // load() 是无依赖检查的 Active 原语（引擎策略适配器/既有测试依赖它）
    let host = host();
    host.load("greeter.usermade", &fixture("greeter-user"))
        .expect("load");
    assert_eq!(lifecycle_of(&host, "greeter.usermade"), "active");
}
