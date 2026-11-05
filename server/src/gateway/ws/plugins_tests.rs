//! [`super::plugins`] 端点的单测：设备插件注册投影（`device_plugins`）。

use crate::config::Config;
use crate::gateway::ws::plugins::device_plugins_json;

/// `device_plugins` 投影契约：管理台按 `registered` 决定「设备管理」菜单与各插件
/// 分区是否出现。客户端是**字符串取键**的（`p.optString("label")` 等），服务端
/// 改名/漏字段不会编译报错，只会让设备管理页静默空白——与 capabilities 契约测试
/// （`plugins/host/tests.rs::capabilities_json_carries_every_key_the_overview_page_reads`）
/// 同一条理由：缺键即测试失败。
#[test]
fn device_plugins_projection_matches_registration_gate() {
    let json = device_plugins_json(&Config::default());
    let arr = json.as_array().expect("device_plugins 数组");
    assert_eq!(arr.len(), 2, "echomuse + tuya 各一行");
    let ids: Vec<&str> = arr.iter().map(|p| p["id"].as_str().unwrap()).collect();
    assert_eq!(ids, vec!["echomuse", "tuya"]);
    for p in arr {
        for k in [
            "id",
            "label",
            "registered",
            "phase",
            "config_section",
            "note",
            "last_error",
        ] {
            assert!(p.get(k).is_some(), "设备插件 {} 缺键 {k}", p["id"]);
        }
    }
    // 默认注册（两段 enabled 默认 true）：管理台菜单出现
    assert!(arr[0]["registered"].as_bool().unwrap(), "echomuse 默认注册");
    assert_eq!(arr[0]["phase"], "active");
    assert!(arr[1]["registered"].as_bool().unwrap(), "tuya 默认注册");
    assert_eq!(arr[1]["phase"], "active");
    assert_eq!(arr[0]["config_section"], "echomuse");
    assert_eq!(arr[1]["config_section"], "tuya");

    // 显式关闭 = 未注册：菜单隐藏，phase 如实报 disabled（不冒充失败——用户自己的选择）
    let mut cfg = Config::default();
    cfg.echomuse.enabled = false;
    cfg.tuya.enabled = false;
    let json = device_plugins_json(&cfg);
    assert!(!json[0]["registered"].as_bool().unwrap());
    assert_eq!(json[0]["phase"], "disabled");
    assert!(!json[1]["registered"].as_bool().unwrap());
    assert_eq!(json[1]["phase"], "disabled");
}
