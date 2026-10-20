//! [`crate::app::discovery`] 的纯函数测试：ServiceInfo 组装与常量口径。

use super::{service_info, HOST_NAME, INSTANCE_NAME, SERVICE_TYPE};
use std::net::Ipv4Addr;

#[test]
fn service_type_is_bonjour_browsable() {
    // 客户端（macApp NWBrowser）与服务端两侧契约：类型必须带 _tcp 传输与 local 域。
    assert!(SERVICE_TYPE.starts_with("_xiaozhi-admin._tcp"));
    assert!(SERVICE_TYPE.ends_with(".local."));
}

#[test]
fn service_info_carries_lan_ip_and_instance() {
    let ip = Ipv4Addr::new(192, 168, 99, 250);
    let info = service_info(ip, 8000).expect("合法 ip/port 应能构造 ServiceInfo");
    assert_eq!(info.get_hostname(), HOST_NAME);
    assert!(info.get_addresses_v4().contains(&ip));
    // fullname = 实例名 + 服务类型（mdns-sd 组装口径），客户端浏览结果即以此展示。
    assert!(info.get_fullname().starts_with(INSTANCE_NAME));
    assert!(info.get_fullname().contains(SERVICE_TYPE));
    assert_eq!(info.get_property_val_str("app"), Some("xiaozhi-admin"));
}

#[test]
fn instance_name_is_stable_for_clients() {
    // 客户端发现列表直接展示实例名；改名会造成老客户端显示混乱，故钉住。
    assert_eq!(INSTANCE_NAME, "xiaozhi-server");
    assert_eq!(HOST_NAME, "xiaozhi-admin.local.");
}
