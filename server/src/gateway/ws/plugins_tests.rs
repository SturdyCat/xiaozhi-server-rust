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
