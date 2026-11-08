//! skills ↔ weather 事件桥测试（P2c 生产接线）：后端开关、geo 预填、未装载显式报错、
//! 双跑缓存语义。NMC 应答脚本化（零真实网络），与 tests/weather.rs 同源样本。

use super::{ensure_fixture, host_with};
use crate::plugins::wasm::bridge;
use crate::plugins::weather::WeatherConfig;
use std::sync::Arc;

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
  "passedchart":[{"time":"08:00","temperature":"21"}]}}"#;

fn weather_host(scripted: bool) -> (Arc<super::ScriptedFetcher>, crate::plugins::wasm::WasmHost) {
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
fn bridge_full_flow_with_cache_reuse() {
    let (fetcher, host) = weather_host(true);

    let result = bridge::forecast(&host, "杭州", "", "");
    assert_eq!(result["ok"], serde_json::json!(true), "{result}");
    assert_eq!(result["location"]["code"], "58457");
    assert!(
        result["speech"].as_str().unwrap_or_default().contains("晴"),
        "{}",
        result
    );
    assert_eq!(fetcher.fetch_count(), 4, "省列表 + 2 省 + 天气");

    // 插件 kv 缓存：同城市同时段第二次查询零出网（桥对缓存透明）。
    // 注意粒度差异：插件按「城市+原始 range」缓存；原生按站点码缓存（不同 range
    // 变体共享一次拉取）——P7 收口时统一，见 weather README 边界一节。
    let again = bridge::forecast(&host, "杭州", "", "");
    assert_eq!(again["ok"], serde_json::json!(true));
    assert_eq!(fetcher.fetch_count(), 4, "缓存命中不应出网");
}

#[test]
fn bridge_geo_prefill_for_empty_city() {
    // 测试进程未注入定位（assemble 只设 RwLock(None)）→ 空城市走 geo 缺省报错
    //（与原生 forecast 同口径；有定位时由事件源侧预填，见 bridge 文档）
    let (_fetcher, host) = weather_host(false);
    let result = bridge::forecast(&host, "  ", "", "");
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
fn bridge_reports_unloaded_plugin_explicitly() {
    let fetcher = Arc::new(super::ScriptedFetcher::new());
    let host = host_with(&fetcher); // 空注册表（内核零插件知识 = 完整可启动）

    let result = bridge::forecast(&host, "杭州", "", "");
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
    // [weather].backend = "wasm"：WeatherService 分发进事件桥（真实服务级开关）
    let (fetcher, host) = weather_host(true);
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
fn echomuse_prepare_roundtrip_keeps_native_route() {
    let fetcher = Arc::new(super::ScriptedFetcher::new());
    let host = host_with(&fetcher);
    host.load("hello.official", &ensure_fixture("hello"))
        .expect("load hello");
    let welcome = bridge::echomuse_prepare(&host, "echo-1", "emos", "你好")
        .expect("echomuse prepare")
        .expect("fixture preserves welcome");
    assert_eq!(welcome, "你好");
}

#[test]
fn echomuse_prepare_fails_closed_without_plugin() {
    let fetcher = Arc::new(super::ScriptedFetcher::new());
    let host = host_with(&fetcher);
    let err = bridge::echomuse_prepare(&host, "echo-1", "emos", "你好")
        .expect_err("missing EchoMuse strategy must fail closed");
    assert!(err.contains("无可用实现"), "{err}");
}

#[test]
fn soul_prepare_roundtrip_returns_only_bounded_native_policy() {
    let fetcher = Arc::new(super::ScriptedFetcher::new());
    let host = host_with(&fetcher);
    host.load("hello.official", &ensure_fixture("hello"))
        .expect("load hello");
    let result =
        bridge::soul_prepare(&host, "device-1", "小智", "你", "", 2, 40).expect("soul prepare");
    assert_eq!(result.route.as_deref(), Some("native"));
    assert_eq!(result.tone.as_deref(), Some("温和、简洁"));
    assert_eq!(result.max_sentences, Some(2));
    assert_eq!(result.max_chars, Some(40));
}

#[test]
fn soul_prepare_fails_closed_without_plugin() {
    let fetcher = Arc::new(super::ScriptedFetcher::new());
    let host = host_with(&fetcher);
    let err = bridge::soul_prepare(&host, "device-1", "小智", "你", "", 2, 40)
        .expect_err("missing soul strategy must fail closed");
    assert!(err.contains("无可用实现"), "{err}");
}

#[test]
fn soul_prepare_rejects_unbounded_request_before_dispatch() {
    let fetcher = Arc::new(super::ScriptedFetcher::new());
    let host = host_with(&fetcher);
    let err = bridge::soul_prepare(
        &host,
        "device-1",
        "小智",
        "你",
        "x".repeat(257).as_str(),
        2,
        40,
    )
    .expect_err("oversized tone must be rejected");
    assert!(err.contains("tone 超长"), "{err}");
}

#[test]
fn tuya_connector_prepare_roundtrip_uses_bounded_decision() {
    let fetcher = Arc::new(super::ScriptedFetcher::new());
    let host = host_with(&fetcher);
    host.load("hello.official", &ensure_fixture("hello"))
        .expect("load hello");
    let candidates = vec![serde_json::json!({
        "index": 0,
        "label": "客厅插座",
        "online": true,
        "model": "outlet",
        "has_switch_dp": true,
    })];
    let result = bridge::tuya_prepare(&host, "客厅插座", "off", &candidates).expect("tuya prepare");
    assert_eq!(result.candidate_index, 0);
    assert_eq!(result.action, "off");
    assert_eq!(result.route.as_deref(), Some("native"));
}

#[test]
fn tuya_connector_prepare_fails_closed_without_plugin() {
    let fetcher = Arc::new(super::ScriptedFetcher::new());
    let host = host_with(&fetcher);
    let err = bridge::tuya_prepare(
        &host,
        "客厅插座",
        "on",
        &[serde_json::json!({"index": 0, "label": "客厅插座"})],
    )
    .expect_err("missing connector must fail closed");
    assert!(err.contains("无可用实现"), "{err}");
}

#[test]
fn wakeup_strategy_prepare_roundtrip_selects_native_catalog_entry() {
    let fetcher = Arc::new(super::ScriptedFetcher::new());
    let host = host_with(&fetcher);
    host.load("hello.official", &ensure_fixture("hello"))
        .expect("load hello");
    let catalog = vec!["我在".to_string(), "在呢".to_string()];
    let index = bridge::wakeup_prepare(&host, &catalog, true).expect("wakeup prepare");
    assert_eq!(index, 0);
}

#[test]
fn wakeup_strategy_prepare_rejects_missing_plugin() {
    let fetcher = Arc::new(super::ScriptedFetcher::new());
    let host = host_with(&fetcher);
    let err = bridge::wakeup_prepare(&host, &["我在".to_string()], true)
        .expect_err("missing wakeup strategy must fail closed");
    assert!(err.contains("无可用实现"), "{err}");
}

#[test]
fn wakeup_strategy_prepare_rejects_out_of_range_catalog() {
    let fetcher = Arc::new(super::ScriptedFetcher::new());
    let host = host_with(&fetcher);
    host.load("hello.official", &ensure_fixture("hello"))
        .expect("load hello");
    let err = bridge::wakeup_prepare(&host, &[], true)
        .expect_err("empty native catalog must be rejected");
    assert!(err.contains("目录为空"), "{err}");
}

#[test]
fn tuya_connector_prepare_rejects_out_of_range_decision() {
    let fetcher = Arc::new(super::ScriptedFetcher::new());
    let host = host_with(&fetcher);
    host.load("hello.official", &ensure_fixture("hello"))
        .expect("load hello");
    let err = bridge::tuya_prepare(&host, "客厅插座", "on", &[])
        .expect_err("empty candidate list must reject fixture decision");
    assert!(err.contains("超出") || err.contains("越界"), "{err}");
}

#[test]
fn debug_prepare_roundtrip_keeps_bounded_native_controls() {
    let fetcher = Arc::new(super::ScriptedFetcher::new());
    let host = host_with(&fetcher);
    host.load("hello.official", &ensure_fixture("hello"))
        .expect("load hello");
    let result = bridge::debug_prepare(&host, "log-stream", "session-events", Some("s1"), 64)
        .expect("debug prepare");
    assert_eq!(result.max_items, 32);
    assert!(result.redact);
    assert_eq!(result.route.as_deref(), Some("native"));
}

#[test]
fn debug_prepare_rejects_unknown_kind_before_dispatch() {
    let fetcher = Arc::new(super::ScriptedFetcher::new());
    let host = host_with(&fetcher);
    let err = bridge::debug_prepare(&host, "raw-socket", "target", None, 32)
        .expect_err("unknown debug kind must fail closed");
    assert!(err.contains("不受支持"), "{err}");
}

#[test]
fn debug_prepare_rejects_oversized_request_before_dispatch() {
    let fetcher = Arc::new(super::ScriptedFetcher::new());
    let host = host_with(&fetcher);
    let err = bridge::debug_prepare(&host, "test-bench", "target", None, 257)
        .expect_err("oversized debug request must fail closed");
    assert!(err.contains("requested_items"), "{err}");
}

#[test]
fn skill_prepare_roundtrip_keeps_bounded_native_admission() {
    let fetcher = Arc::new(super::ScriptedFetcher::new());
    let host = host_with(&fetcher);
    host.load("hello.official", &ensure_fixture("hello"))
        .expect("load hello");
    let slots = vec![("device".to_string(), "客厅插座".to_string())];
    let result = bridge::skill_prepare(&host, "tuya.control", &slots, 8).expect("skill prepare");
    assert!(result.allow);
    assert_eq!(result.max_steps, 2);
    assert_eq!(result.route.as_deref(), Some("native"));
}

#[test]
fn skill_prepare_rejects_unbounded_slots_before_dispatch() {
    let fetcher = Arc::new(super::ScriptedFetcher::new());
    let host = host_with(&fetcher);
    let slots = (0..17)
        .map(|index| (format!("slot-{index}"), "value".to_string()))
        .collect::<Vec<_>>();
    let err = bridge::skill_prepare(&host, "tuya.control", &slots, 8)
        .expect_err("too many slots must fail closed");
    assert!(err.contains("slots 超限"), "{err}");
}

#[test]
fn skill_prepare_rejects_missing_plugin() {
    let fetcher = Arc::new(super::ScriptedFetcher::new());
    let host = host_with(&fetcher);
    let err = bridge::skill_prepare(&host, "tuya.control", &[], 1)
        .expect_err("missing skill policy must fail closed");
    assert!(err.contains("无可用实现"), "{err}");
}

#[test]
fn memory_prepare_roundtrip_keeps_bounded_native_budget() {
    let fetcher = Arc::new(super::ScriptedFetcher::new());
    let host = host_with(&fetcher);
    host.load("hello.official", &ensure_fixture("hello"))
        .expect("load hello");
    let result = bridge::memory_prepare(&host, "device-1", "我昨天说了什么", 8, 1024)
        .expect("memory prepare");
    assert!(result.allow);
    assert_eq!(result.max_items, 4);
    assert_eq!(result.max_chars, 512);
    assert_eq!(result.route.as_deref(), Some("native"));
}

#[test]
fn memory_prepare_rejects_unbounded_query_before_dispatch() {
    let fetcher = Arc::new(super::ScriptedFetcher::new());
    let host = host_with(&fetcher);
    let err = bridge::memory_prepare(&host, "device-1", &"x".repeat(513), 8, 1024)
        .expect_err("oversized memory query must fail closed");
    assert!(err.contains("query 为空或超长"), "{err}");
}

#[test]
fn memory_prepare_rejects_missing_plugin() {
    let fetcher = Arc::new(super::ScriptedFetcher::new());
    let host = host_with(&fetcher);
    let err = bridge::memory_prepare(&host, "device-1", "记忆", 1, 128)
        .expect_err("missing memory policy must fail closed");
    assert!(err.contains("无可用实现"), "{err}");
}

#[test]
fn nlu_catalog_prepare_roundtrip_filters_bounded_native_catalog() {
    let fetcher = Arc::new(super::ScriptedFetcher::new());
    let host = host_with(&fetcher);
    host.load("hello.official", &ensure_fixture("hello"))
        .expect("load hello");
    let candidates = vec![
        ("weather".to_string(), "天气".to_string()),
        ("end_session".to_string(), "结束对话".to_string()),
    ];
    let result =
        bridge::nlu_catalog_prepare(&host, "zh-CN", &candidates, 2).expect("nlu catalog prepare");
    assert_eq!(result.indices, vec![1, 0]);
    assert_eq!(result.route.as_deref(), Some("native"));
}

#[test]
fn nlu_catalog_prepare_rejects_unbounded_candidates_before_dispatch() {
    let fetcher = Arc::new(super::ScriptedFetcher::new());
    let host = host_with(&fetcher);
    let candidates = (0..33)
        .map(|index| (format!("skill-{index}"), "技能".to_string()))
        .collect::<Vec<_>>();
    let err = bridge::nlu_catalog_prepare(&host, "zh-CN", &candidates, 8)
        .expect_err("too many catalog candidates must fail closed");
    assert!(err.contains("candidates 为空或超限"), "{err}");
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

// ============================================================
// P2d：观察者挂钩（try_dispatch / spawn_deferred / observe_hook）
// ============================================================

#[test]
fn try_dispatch_returns_effects_without_executing() {
    let (fetcher, _host) = weather_host(false);
    let _ = fetcher;
    // hello 才有 hook 事件；装载 hello 做派发语义测试
    let host2 = host_with(&Arc::new(super::ScriptedFetcher::new()));
    host2
        .load("hello.official", &ensure_fixture("hello"))
        .expect("load");

    let effects = host2
        .try_dispatch("hello.official", "hook.start", r#"{"session_id":"s1"}"#)
        .expect("派发")
        .expect("应有 kv-put 效果");
    assert_eq!(effects.len(), 1);
    // 关键语义：效果未执行——kv 无值（spawn_deferred 才落）
    assert!(host2.kv().get("hello.official", "hook-last").is_none());

    // 业务性拒绝（未知事件）= Ok(None)，不算错误、不降级
    assert!(host2
        .try_dispatch("hello.official", "nope", "")
        .expect("业务拒绝不是错误")
        .is_none());
    assert_eq!(
        host2.snapshot_json()["plugins"][0]["phase"]["state"],
        "active"
    );
}

#[tokio::test]
async fn spawn_deferred_executes_effects_off_turn_path() {
    let fetcher = Arc::new(super::ScriptedFetcher::new());
    let host = host_with(&fetcher);
    host.load("hello.official", &ensure_fixture("hello"))
        .expect("load");

    let effects = host
        .try_dispatch("hello.official", "hook.link.nlu.llm", r#"{"text":"杭州"}"#)
        .expect("派发")
        .expect("效果");
    host.spawn_deferred("hello.official", effects);

    // fire-and-forget：有界等待异步落 kv（不 sleep 死等）
    for _ in 0..100 {
        if host.kv().get("hello.official", "hook-last").is_some() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    let stored = host
        .kv()
        .get("hello.official", "hook-last")
        .expect("异步执行应落 kv");
    assert!(String::from_utf8_lossy(&stored).contains("杭州"));
}

#[tokio::test]
async fn observe_hook_full_path_never_panics_on_unloaded() {
    let fetcher = Arc::new(super::ScriptedFetcher::new());
    let host = host_with(&fetcher);
    // 未装载插件：bridge::observe_hook 只告警不 panic（旁路永不打断）
    bridge::observe_hook(&host, "ghost.plugin", "start", "{}");
    // trap 插件装载后派发超预算 → 降级告警，同样不 panic
    host.load("trap.hostile", &ensure_fixture("trap"))
        .expect("load");
    // trap 夹具只对 spin 作恶（死循环→fuel 陷阱）——观察者路径同样触发降级
    bridge::observe_hook(&host, "trap.hostile", "spin", "{}");
    assert_eq!(
        host.snapshot_json()["plugins"][0]["phase"]["state"],
        "degraded"
    );
}

// ============================================================
// P3：[plugins] 配置门（装载期过滤 + 引导跳过）
// ============================================================

#[test]
fn assemble_skips_artifact_with_mismatched_persisted_pin() {
    use crate::config::store::sqlite::SqliteConfigStore;
    use crate::config::store::ConfigStore;
    use crate::config::{Config, PluginEntryConfig};

    let dir = std::env::temp_dir().join(format!("xz-plugin-pin-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).expect("tmp dir");
    let db = dir.join("data.db");
    let store = SqliteConfigStore::open(db.to_str().unwrap()).expect("store");
    let bytes = ensure_fixture("hello");
    let mut cfg = Config::default();
    cfg.plugins.entries.insert(
        "hello.official".into(),
        PluginEntryConfig {
            enabled: true,
            artifact: Some("00".repeat(32)),
            attestation: None,
        },
    );
    store.persist(&cfg).expect("persist");
    let artifacts = crate::plugins::wasm::artifacts::ArtifactDir::for_location(store.location())
        .expect("location 有值");
    artifacts
        .save("hello.official", &bytes)
        .expect("save hello");

    let engines = crate::runtime::Engines::new_dev(&cfg, store).expect("dev engines");
    let host = engines.wasm.as_ref().expect("feature 编入宿主必在");
    assert!(
        host.snapshot_json()["plugins"]
            .as_array()
            .unwrap()
            .iter()
            .all(|p| p["id"] != "hello.official"),
        "摘要不匹配的产物不得在启动时激活"
    );
}

#[test]
fn assemble_skips_config_disabled_artifacts() {
    use crate::config::store::sqlite::SqliteConfigStore;
    use crate::config::store::ConfigStore;
    use crate::config::{Config, PluginsConfig};

    // 落盘配置库（有 location → ArtifactDir 生效）+ 两份产物：启用=hello 真件、禁用=垃圾件
    let dir = std::env::temp_dir().join(format!("xz-plugin-gate-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).expect("tmp dir");
    let db = dir.join("data.db");
    let store = SqliteConfigStore::open(db.to_str().unwrap()).expect("store");
    let mut cfg = Config::default();
    let mut off = PluginsConfig::default();
    off.entries.insert(
        "junk.garbage".into(),
        crate::config::PluginEntryConfig {
            enabled: false,
            ..Default::default()
        },
    );
    cfg.plugins = off;
    store.persist(&cfg).expect("persist");

    let artifacts = crate::plugins::wasm::artifacts::ArtifactDir::for_location(store.location())
        .expect("location 有值 → 产物目录生效");
    artifacts
        .save("hello.official", &ensure_fixture("hello"))
        .expect("save hello");
    artifacts
        .save("junk.garbage", &[0u8; 16])
        .expect("save junk");

    let engines = crate::runtime::Engines::new_dev(&cfg, store).expect("dev engines");
    let host = engines.wasm.as_ref().expect("feature 编入宿主必在");
    let snap = host.snapshot_json();
    let ids: Vec<&str> = snap["plugins"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|p| p["id"].as_str())
        .collect();
    assert!(ids.contains(&"hello.official"), "启用产物应装载：{ids:?}");
    assert!(
        !ids.contains(&"junk.garbage"),
        "禁用产物不得装载（即使落盘存在）：{ids:?}"
    );
}
