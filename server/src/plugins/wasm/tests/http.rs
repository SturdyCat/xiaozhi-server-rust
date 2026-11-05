//! P1b：http-get 两段式回灌（脚本化 fetcher，零真实网络）。

use super::{fast_limits, fixture, host_with};
use crate::plugins::wasm::WasmHost;

#[test]
fn http_get_roundtrip_feeds_back_and_persists() {
    let fetcher = std::sync::Arc::new(super::ScriptedFetcher::new());
    fetcher.push_ok(200, "春光明媚 22 度");
    let host = host_with(&fetcher);
    host.load("hello.official", &fixture("hello"))
        .expect("load");

    let errs = host
        .handle_event("hello.official", "http.probe", "https://example.com/api")
        .expect("probe 不应有宿主级错误");
    assert!(errs.is_empty(), "{errs:?}");

    // 回灌事件驱动 guest 把响应存 kv：真实跨 wasm 边界的请求-回灌-落库闭环
    let stored = host
        .kv()
        .get("hello.official", "last-http")
        .expect("应已回灌落库");
    let text = String::from_utf8(stored).expect("kv 值为 UTF-8");
    assert!(text.contains("春光明媚"), "响应体应落 kv：{text}");
    assert!(text.contains("\"status\":200"), "状态码应回灌：{text}");
    assert_eq!(fetcher.fetch_count(), 1);
    assert_eq!(
        host.snapshot_json()["plugins"][0]["phase"]["state"],
        "active"
    );
}

#[test]
fn http_fetch_error_feeds_back_gracefully() {
    let fetcher = std::sync::Arc::new(super::ScriptedFetcher::new());
    fetcher.push_err("connection refused");
    let host = host_with(&fetcher);
    host.load("hello.official", &fixture("hello"))
        .expect("load");

    // 网络失败不是宿主错误：status=0 + error 原样回灌，插件自行降级
    host.handle_event("hello.official", "http.probe", "https://example.com/api")
        .expect("网络失败不应是宿主级错误");
    let stored = host
        .kv()
        .get("hello.official", "last-http")
        .expect("错误也应回灌");
    let text = String::from_utf8(stored).expect("UTF-8");
    assert!(
        text.contains("connection refused"),
        "错误消息应回灌：{text}"
    );
    assert!(text.contains("\"status\":0"), "失败状态码：{text}");
    assert_eq!(
        host.snapshot_json()["plugins"][0]["phase"]["state"],
        "active"
    );
}

#[test]
fn http_loop_hits_depth_cap_and_stops_cleanly() {
    let fetcher = std::sync::Arc::new(super::ScriptedFetcher::new());
    for _ in 0..16 {
        fetcher.push_ok(200, "again");
    }
    let host = host_with(&fetcher);
    host.load("hello.official", &fixture("hello"))
        .expect("load");

    // guest 对 loop 请求的回灌会再发请求（自我循环）；宿主深度上限应干净截断：
    // 初始 1 次 + 4 次回灌 = 5 次请求，之后停止回灌并报错，插件不降级（宿主策略非插件过错）
    let errs = host
        .handle_event("hello.official", "http.loop", "https://example.com/start")
        .expect("循环截断不是宿主级致命错误");
    assert!(
        errs.iter().any(|e| e.to_string().contains("深度达上限")),
        "应报深度上限：{errs:?}"
    );
    assert_eq!(fetcher.fetch_count(), 5, "1 初始 + 4 回灌");
    assert_eq!(
        host.snapshot_json()["plugins"][0]["phase"]["state"],
        "active",
        "宿主策略截断不应降级插件"
    );
}

#[test]
fn http_effect_without_permission_degrades() {
    let fetcher = std::sync::Arc::new(super::ScriptedFetcher::new());
    let host = host_with(&fetcher);
    host.load("greeter.usermade", &fixture("greeter-user"))
        .expect("load");

    // 无 http-out 权限的效果：权限门拒绝 → Degraded（对照导入侧 deny-as-absent 温和形态）
    let err = host
        .handle_event("greeter.usermade", "user.fetch", "")
        .unwrap_err();
    assert!(err.to_string().contains("http-out"), "拒绝文案：{err}");
    assert_eq!(
        host.snapshot_json()["plugins"][0]["phase"]["state"],
        "degraded"
    );
    assert_eq!(fetcher.fetch_count(), 0, "越权请求不应出网");
}

#[test]
fn http_url_gate_parses_hosts_strictly() {
    use crate::plugins::wasm::effects::{enforce_permissions, Effect, Permissions};
    let perms = Permissions::parse(&["http-out:example.com".into()]).expect("perms");

    let ok = |url: &str| {
        enforce_permissions(
            &perms,
            vec![Effect::HttpGet {
                request_id: "t".into(),
                url: url.into(),
            }],
        )
        .is_ok()
    };
    assert!(ok("https://example.com/api"));
    assert!(ok("http://example.com:8443/api"), "端口应剥离后匹配");
    assert!(!ok("https://evil.com/api"), "白名单外 host 拒绝");
    assert!(!ok("https://notexample.com/"), "前缀巧合不算匹配");
    assert!(!ok("ftp://example.com/x"), "非 http(s) scheme 拒绝");
    assert!(!ok("/relative/path"), "相对地址拒绝");
    assert!(!ok("https:///x"), "空 host 拒绝");
}

// ============================================================
