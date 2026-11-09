//! weather.official 回放测试（P1c 试点：真实插件源 plugins-src/weather/ 经
//! 脚本化 NMC 应答走完整瀑布——省列表 → 全省城市 → 站点匹配 → 天气组装 → 缓存命中）。

use super::{ensure_fixture, fixture, host_with};
use crate::plugins::wasm::Resolution;
use std::sync::Arc;

/// NMC 真实接口形状的最小样本（字段路径对齐 parse.rs：real.weather/predict.detail/passedchart）。
const PROVINCES: &str = r#"[{"code":"33","name":"浙江省"},{"code":"32","name":"江苏省"}]"#;
const ZHEJIANG_CITIES: &str = r#"[
  {"city":"杭州","code":"58457","province":"浙江省"},
  {"city":"宁波","code":"58468","province":"浙江省"}]"#;
const JIANGSU_CITIES: &str = r#"[{"city":"南京","code":"58238","province":"江苏省"}]"#;
const HANGZHOU_WEATHER: &str = r#"{"code":0,"data":{
  "real":{
    "publish_time":"2026-11-04 08:00",
    "weather":{"temperature":"22","info":"晴","humidity":"45","feelslike":"21","airpressure":"1008"},
    "wind":{"direct":"东北风","power":"3级"}},
  "predict":{"detail":[
    {"date":"2026-11-04",
     "day":{"info":"晴","temperature":"24","wind":{"direct":"东风","power":"2级"}},
     "night":{"info":"多云","temperature":"15","wind":{"direct":"东风","power":"2级"}}}]},
  "passedchart":[{"time":"08:00","temperature":"21"},{"time":"09:00","temperature":"22"}]}}"#;

fn result_of(host: &crate::plugins::wasm::WasmHost, req_id: &str) -> serde_json::Value {
    let bytes = host
        .kv()
        .get("weather.official", &format!("result:{req_id}"))
        .unwrap_or_else(|| panic!("结果未落 kv（req_id={req_id}）"));
    serde_json::from_slice(&bytes).expect("结果应为合法 JSON")
}

#[test]
fn weather_full_cold_path_then_cache_hit() {
    let fetcher = Arc::new(super::ScriptedFetcher::new());
    // 脚本按效果执行顺序：省列表 → 浙江省 → 江苏省 → 杭州天气
    fetcher.push_ok(200, PROVINCES);
    fetcher.push_ok(200, ZHEJIANG_CITIES);
    fetcher.push_ok(200, JIANGSU_CITIES);
    fetcher.push_ok(200, HANGZHOU_WEATHER);
    let host = host_with(&fetcher);
    host.load("weather.official", &fixture("weather"))
        .expect("load");

    // 官方默认实现占住扩展点（skill.weather）
    assert!(matches!(
        host.resolve("skill.weather"),
        Resolution::Resolved(ref id) if id == "weather.official"
    ));

    // 冷路径：省列表 →（一步并发）全省城市 → 站点匹配 → 天气，3 次批量回灌内完成
    let errs = host
        .handle_event(
            "weather.official",
            "weather.forecast",
            r#"{"req_id":"q1","city":"杭州","province":"","range":""}"#,
        )
        .expect("冷路径不应有宿主级错误");
    assert!(errs.is_empty(), "{errs:?}");
    assert_eq!(fetcher.fetch_count(), 4, "省列表 + 2 省 + 天气");

    let result = result_of(&host, "q1");
    assert_eq!(result["ok"], serde_json::json!(true), "{result}");
    assert_eq!(result["location"]["code"], "58457");
    assert_eq!(result["range"], "实时");
    let speech = result["speech"].as_str().unwrap_or_default();
    assert!(
        speech.contains("晴") && speech.contains("22"),
        "话术：{speech}"
    );
    assert_eq!(result["realtime"]["temp_c"], "22");

    // 站点码与结果缓存均已落 kv（派生缓存，可随时清空重建）
    assert_eq!(
        host.kv().get("weather.official", "station:杭州").as_deref(),
        Some("58457".as_bytes())
    );

    // 热路径：同城市不同别名、不同 range → station code 原始数据缓存命中，不再出网。
    let errs = host
        .handle_event(
            "weather.official",
            "weather.forecast",
            r#"{"req_id":"q2","city":"杭州天气","province":"","range":"明天"}"#,
        )
        .expect("热路径");
    assert!(errs.is_empty(), "{errs:?}");
    assert_eq!(fetcher.fetch_count(), 4, "站点原始数据缓存命中不应出网");
    assert_eq!(result_of(&host, "q2")["ok"], serde_json::json!(true));
    assert_eq!(result_of(&host, "q2")["range"], "明天");
}

#[test]
fn weather_unknown_city_fails_gracefully() {
    let fetcher = Arc::new(super::ScriptedFetcher::new());
    fetcher.push_ok(200, PROVINCES);
    fetcher.push_ok(200, ZHEJIANG_CITIES);
    fetcher.push_ok(200, JIANGSU_CITIES);
    let host = host_with(&fetcher);
    host.load("weather.official", &fixture("weather"))
        .expect("load");

    host.handle_event(
        "weather.official",
        "weather.forecast",
        r#"{"req_id":"qE","city":"不存在的城市","province":"","range":""}"#,
    )
    .expect("未找到城市不是宿主错误");

    // 同原生口径：失败也走结果通道（ok:false + 话术），不 panic、不降级
    let result = result_of(&host, "qE");
    assert_eq!(result["ok"], serde_json::json!(false), "{result}");
    assert!(
        result["speech"]
            .as_str()
            .unwrap_or_default()
            .contains("未找到城市"),
        "{}",
        result
    );
    assert_eq!(
        host.snapshot_json()["plugins"][0]["phase"]["state"],
        "active"
    );
}

#[test]
fn weather_self_test_proves_shared_parse_alive_in_wasm() {
    let fetcher = Arc::new(super::ScriptedFetcher::new());
    let host = host_with(&fetcher);
    let results = host
        .validate("weather.official", &fixture("weather"))
        .expect("validate");
    assert!(
        results.iter().all(|r| r.ok),
        "自检应全过（共享 parse.rs 在 wasm 内可用）：{results:?}"
    );
}

// ============================================================
// 服务级事件派发（阶段二收口：dispatch 归位 plugins/weather，桥已删除）
// ============================================================

use crate::plugins::weather::dispatch::forecast_via_plugin;
use crate::plugins::weather::WeatherConfig;

fn dispatch_host(scripted: bool) -> (Arc<super::ScriptedFetcher>, crate::plugins::wasm::WasmHost) {
    let fetcher = Arc::new(super::ScriptedFetcher::new());
    if scripted {
        fetcher.push_ok(200, PROVINCES);
        fetcher.push_ok(200, ZHEJIANG_CITIES);
        fetcher.push_ok(200, JIANGSU_CITIES);
        fetcher.push_ok(200, HANGZHOU_WEATHER);
    }
    let host = host_with(&fetcher);
    host.load("weather.official", &ensure_fixture("weather"))
        .expect("load weather");
    (fetcher, host)
}

#[test]
fn dispatch_full_flow_with_cache_reuse() {
    let (fetcher, host) = dispatch_host(true);

    let result = forecast_via_plugin(&host, "杭州", "", "");
    assert_eq!(result["ok"], serde_json::json!(true), "{result}");
    assert_eq!(result["location"]["code"], "58457");
    assert!(
        result["speech"].as_str().unwrap_or_default().contains("晴"),
        "{}",
        result
    );
    assert_eq!(fetcher.fetch_count(), 4, "省列表 + 2 省 + 天气");

    // 插件 kv 缓存：同城市同时段第二次查询零出网（派发对缓存透明）。
    // 注意粒度差异：插件按「城市+原始 range」缓存；原生按站点码缓存（不同 range
    // 变体共享一次拉取）——P7 收口时统一，见 weather README 边界一节。
    let again = forecast_via_plugin(&host, "杭州", "", "");
    assert_eq!(again["ok"], serde_json::json!(true));
    assert_eq!(fetcher.fetch_count(), 4, "缓存命中不应出网");
}

#[test]
fn dispatch_geo_prefill_for_empty_city() {
    // 测试进程未注入定位（assemble 只设 RwLock(None)）→ 空城市走 geo 缺省报错
    //（与原生 forecast 同口径；有定位时由事件源侧预填，见 weather 文档）
    let (_fetcher, host) = dispatch_host(false);
    let result = forecast_via_plugin(&host, "  ", "", "");
    assert_eq!(result["ok"], serde_json::json!(false), "{result}");
    assert!(
        result["speech"]
            .as_str()
            .unwrap_or_default()
            .contains("未指定城市"),
        "{}",
        result
    );
}

#[test]
fn dispatch_reports_unloaded_plugin_explicitly() {
    let fetcher = Arc::new(super::ScriptedFetcher::new());
    let host = host_with(&fetcher); // 空注册表（内核零插件知识 = 完整可启动）

    let result = forecast_via_plugin(&host, "杭州", "", "");
    assert_eq!(result["ok"], serde_json::json!(false), "{result}");
    assert!(
        result["speech"]
            .as_str()
            .unwrap_or_default()
            .contains("未装载"),
        "缺依赖明确报错，不静默回退：{result}"
    );
    assert_eq!(fetcher.fetch_count(), 0);
}

#[test]
fn backend_switch_routes_service_to_wasm() {
    // [weather].backend = "wasm"：WeatherService 分发进事件派发（真实服务级开关）
    let (fetcher, host) = dispatch_host(true);
    let mut cfg = WeatherConfig::default();
    // 默认后端随构建模式：full 默认 native（对存量配置零影响）；pure-base 未编译
    // 原生 NMC 客户端，唯一真实后端是已注册 WASM 插件，默认值即 wasm。
    #[cfg(not(feature = "native-plugins"))]
    assert_eq!(
        cfg.backend, "wasm",
        "pure-base 默认后端是 wasm（native 未编译）"
    );
    #[cfg(feature = "native-plugins")]
    assert_eq!(cfg.backend, "native", "默认后端是 native（零影响）");
    cfg.backend = "wasm".into();
    let svc = crate::plugins::weather::WeatherService::new(&cfg);
    svc.set_wasm_host(Arc::new(host));

    let result = svc.forecast("杭州", "", "");
    assert_eq!(result["ok"], serde_json::json!(true), "{result}");
    assert_eq!(result["location"]["code"], "58457");
    assert_eq!(
        fetcher.fetch_count(),
        4,
        "确实走了 wasm 瀑布而非原生 reqwest"
    );
}

#[test]
fn backend_switch_without_host_reports_explicitly() {
    // wasm 后端 + 宿主句柄缺失（初始化失败的装配形态）→ 明确报错，不回退 native。
    // 错误措辞随构建模式：full 报「未初始化」（宿主注入失败）；pure-base 报「未注册」
    //（底座视角缺的是插件本身）。
    let mut cfg = WeatherConfig::default();
    cfg.backend = "wasm".into();
    let svc = crate::plugins::weather::WeatherService::new(&cfg);
    let result = svc.forecast("杭州", "", "");
    assert_eq!(result["ok"], serde_json::json!(false), "{result}");
    #[cfg(not(feature = "native-plugins"))]
    let expected = "未注册";
    #[cfg(feature = "native-plugins")]
    let expected = "未初始化";
    assert!(
        result["speech"]
            .as_str()
            .unwrap_or_default()
            .contains(expected),
        "{}",
        result
    );
}

#[test]
fn backend_unknown_value_rejected() {
    let mut cfg = WeatherConfig::default();
    cfg.backend = "cloud".into();
    let svc = crate::plugins::weather::WeatherService::new(&cfg);
    let result = svc.forecast("杭州", "", "");
    assert_eq!(result["ok"], serde_json::json!(false), "{result}");
    // full 区分未知后端；pure-base 没有 native 分支，任何非 wasm 值统一显式拒绝。
    #[cfg(not(feature = "native-plugins"))]
    let expected = "只能使用已注册的 WASM 插件";
    #[cfg(feature = "native-plugins")]
    let expected = "未知天气后端";
    assert!(
        result["speech"]
            .as_str()
            .unwrap_or_default()
            .contains(expected),
        "{}",
        result
    );
}
