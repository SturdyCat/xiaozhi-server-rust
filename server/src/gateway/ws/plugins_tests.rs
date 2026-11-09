//! 插件清单与默认配置边界测试。
//!
//! 静态设备插件投影已被新架构删除；底座只返回核心能力与已装载的动态 manifest。

#[cfg(not(any(
    feature = "native-plugins",
    feature = "echomuse-plugin",
    feature = "tuya-plugin",
    feature = "esp_home"
)))]
#[test]
fn base_does_not_advertise_static_business() {
    let value = serde_json::to_value(crate::config::Config::default()).unwrap();
    let object = value.as_object().unwrap();
    for section in ["echomuse", "tuya", "esp_home"] {
        assert!(
            !object.contains_key(section),
            "clean base must not serialize plugin section {section}"
        );
    }

    let ids: Vec<_> = crate::plugins::registry::ALL_CAPABILITIES
        .iter()
        .map(|cap| cap.id())
        .collect();
    assert!(ids.iter().all(|id| !matches!(*id, "echomuse" | "tuya")));
}

/// 阶段二收口：默认构建必须声明 `pure-base`（客户端据此隐藏静态业务导航），
/// 且 `static_business=false` 与底座事实一致（此前恒返回 `base`，与客户端漂移）。
#[cfg(not(any(
    feature = "native-plugins",
    feature = "echomuse-plugin",
    feature = "tuya-plugin",
    feature = "esp_home"
)))]
#[test]
fn base_ui_projection_declares_pure_base() {
    let ui = super::plugins::ui_projection_json();
    assert_eq!(ui["mode"], "pure-base");
    assert_eq!(ui["static_business"], false);
}

/// native 设备插件构建如实声明 `base` + `static_business=true`（不谎报纯底座）。
#[cfg(any(
    feature = "native-plugins",
    feature = "echomuse-plugin",
    feature = "tuya-plugin",
    feature = "esp_home"
))]
#[test]
fn plugin_build_ui_projection_declares_static_business() {
    let ui = super::plugins::ui_projection_json();
    assert_eq!(ui["mode"], "base");
    assert_eq!(ui["static_business"], true);
}

#[cfg(any(
    feature = "native-plugins",
    feature = "echomuse-plugin",
    feature = "tuya-plugin",
    feature = "esp_home"
))]
#[test]
fn plugin_build_keeps_static_projection_out_of_base_contract() {
    let value = serde_json::to_value(crate::config::Config::default()).unwrap();
    assert!(value.is_object());
}
