//! SDK 构建器与可失败分配的外置测试（本 crate 只在 native 跑单元级测试；
//! guest 全链路由 tools/wasm-fixtures 真实组件 + server wasm 测试覆盖）。

use xiaozhi_plugin_sdk::builder::{FieldSpec, ManifestBuilder, RegistrationBuilder};
use xiaozhi_plugin_sdk::fallible;
use xiaozhi_plugin_sdk::xiaozhi::plugin::types::Hot;

// ============================================================
// manifest 构建器
// ============================================================

#[test]
fn manifest_happy_path_defaults() {
    let m = ManifestBuilder::new("weather.official", "天气")
        .expect("合法 id/title")
        .description("NMC 天气")
        .permission("http-out:www.nmc.cn")
        .expect("http-out 权限")
        .permission("kv")
        .expect("kv 权限")
        .implements("skill.weather")
        .expect("扩展点")
        .official(true)
        .build()
        .expect("build");
    assert_eq!(m.id, "weather.official");
    assert_eq!(m.api_version, xiaozhi_plugin_sdk::API_VERSION);
    assert_eq!(m.permissions.len(), 2);
    assert_eq!(m.implements, vec!["skill.weather".to_string()]);
    assert!(m.official);
}

#[test]
fn manifest_rejects_bad_shapes_guest_side() {
    // id 形状（大写/空段）
    assert!(ManifestBuilder::new("Bad ID", "标题").is_err());
    assert!(ManifestBuilder::new(".lead-dot", "标题").is_err());
    // 空 title
    assert!(ManifestBuilder::new("a.b", "").is_err());
    // 未知权限
    let e = ManifestBuilder::new("a.b", "t")
        .expect("ok")
        .permission("root")
        .unwrap_err();
    assert!(matches!(e, xiaozhi_plugin_sdk::Error::Invalid(_)), "{e:?}");
    // http-out 空 host
    assert!(ManifestBuilder::new("a.b", "t")
        .expect("ok")
        .permission("http-out:")
        .is_err());
    // 重复权限
    assert!(ManifestBuilder::new("a.b", "t")
        .expect("ok")
        .permission("kv")
        .expect("kv")
        .permission("kv")
        .is_err());
    // 扩展点无点号 + 重复
    assert!(ManifestBuilder::new("a.b", "t")
        .expect("ok")
        .implements("nodot")
        .is_err());
    assert!(ManifestBuilder::new("a.b", "t")
        .expect("ok")
        .implements("a.b")
        .expect("ok")
        .implements("a.b")
        .is_err());
}

// ============================================================
// registration 构建器
// ============================================================

#[test]
fn registration_happy_path() {
    let reg = RegistrationBuilder::new()
        .menu_entry("nav.main", "weather", "天气", "插件", 100, "settings")
        .expect("menu")
        .config_field(FieldSpec::str("city", "默认城市").hint("北京").build())
        .expect("field str")
        .config_field(
            FieldSpec::enum_("unit", "单位", &["摄氏", "华氏"])
                .hot(Hot::NextSession)
                .build(),
        )
        .expect("field enum")
        .action("refresh", "刷新缓存", false, "清空站点缓存")
        .expect("action")
        .build();
    assert_eq!(reg.menu.len(), 1);
    assert_eq!(reg.config_fields.len(), 2);
    assert_eq!(reg.actions.len(), 1);
    assert_eq!(reg.config_fields[1].enum_values.len(), 2);
}

#[test]
fn registration_rejects_duplicates_and_unknown_shapes() {
    // 未知槽位
    assert!(RegistrationBuilder::new()
        .menu_entry("sidebar.secret", "a", "标签", "插件", 1, "settings")
        .is_err());
    // 槽内重复 id
    let b = RegistrationBuilder::new()
        .menu_entry("nav.main", "a", "标签", "插件", 1, "settings")
        .expect("first");
    assert!(b
        .menu_entry("nav.main", "a", "标签2", "插件", 2, "settings")
        .is_err());
    // 同 id 不同槽位允许（槽内唯一，不是全局唯一）
    assert!(RegistrationBuilder::new()
        .menu_entry("nav.main", "a", "标签", "插件", 1, "settings")
        .expect("nav")
        .menu_entry("config.section", "a", "标签", "插件", 1, "settings")
        .is_ok());
    // 空 label
    assert!(RegistrationBuilder::new()
        .menu_entry("nav.main", "a", "", "插件", 1, "settings")
        .is_err());

    // 字段 key 形状 / 重复 / 非法 kind / 空 enum
    assert!(RegistrationBuilder::new()
        .config_field(FieldSpec::str("a.b", "标签").build())
        .is_err());
    let b = RegistrationBuilder::new()
        .config_field(FieldSpec::str("k", "标签").build())
        .expect("first");
    assert!(b
        .config_field(FieldSpec::str("k", "标签2").build())
        .is_err());
    // enum 无值
    assert!(RegistrationBuilder::new()
        .config_field(FieldSpec::enum_("unit", "单位", &[]).build())
        .is_err());

    // 动作空 label / 重复 id
    assert!(RegistrationBuilder::new()
        .action("x", "", false, "")
        .is_err());
    let b = RegistrationBuilder::new()
        .action("x", "标题", false, "")
        .expect("first");
    assert!(b.action("x", "标题2", true, "").is_err());
}

// ============================================================
// JSON UI 描述（A2UI 最小闭环；规则与宿主 validate_ui_json 同源）
// ============================================================

#[test]
fn ui_json_happy_path_carried_into_registration() {
    let descriptor = r#"{"a2ui":"0.1","pages":[{"id":"cfg","tree":{"type":"form"}}]}"#;
    let reg = RegistrationBuilder::new()
        .action("greet", "打招呼", false, "问候")
        .expect("action")
        .ui_json(descriptor)
        .expect("合法 UI 描述")
        .build();
    assert_eq!(reg.ui_json.as_deref(), Some(descriptor));
}

#[test]
fn ui_json_rejects_bad_shapes_guest_side() {
    // 非法 JSON
    let e = RegistrationBuilder::new()
        .ui_json("not json")
        .err()
        .expect("应拒绝");
    assert!(matches!(e, xiaozhi_plugin_sdk::Error::Invalid(_)), "{e:?}");
    // 顶层非对象
    assert!(RegistrationBuilder::new().ui_json("[]").is_err());
    assert!(RegistrationBuilder::new().ui_json("null").is_err());
    // 超大小上限
    let big = format!(r#"{{"pad":"{}"}}"#, " ".repeat(256 * 1024));
    assert!(RegistrationBuilder::new().ui_json(&big).is_err());
    // 未声明是合法默认（menu/fields 兼容面不受影响）
    assert_eq!(RegistrationBuilder::new().build().ui_json, None);
}

// ============================================================
// 可失败分配
// ============================================================

#[test]
fn fallible_alloc_helpers_roundtrip() {
    let s = fallible::try_string("你好，世界", "测试克隆").expect("小字符串必成");
    assert_eq!(s, "你好，世界");

    let mut buf = fallible::try_string_with_capacity(64, "测试容量").expect("容量预留");
    fallible::try_push_str(&mut buf, "第一段", "追加1").expect("容量内追加");
    fallible::try_push_str(&mut buf, "第二段", "追加2").expect("扩容追加");
    assert_eq!(buf, "第一段第二段");

    let mut v = fallible::try_vec_with_capacity::<u8>(4, "vec 容量").expect("vec 容量");
    for i in 0..10 {
        fallible::try_push(&mut v, i, "vec 追加").expect("vec 追加");
    }
    assert_eq!(v, (0..10).collect::<Vec<u8>>());
}
