//! weather.official 回放测试（P1c 试点：真实插件源 plugins-src/weather/ 经
//! 脚本化 NMC 应答走完整瀑布——省列表 → 全省城市 → 站点匹配 → 天气组装 → 缓存命中）。

use super::{fast_limits, fixture, host_with};
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
