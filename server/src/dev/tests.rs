use super::*;

fn parse(args: &[&str]) -> Result<Options> {
    Options::parse(args.iter().map(|v| (*v).to_string()))
}

#[test]
fn development_is_explicit_and_isolated() {
    assert!(!parse(&[]).unwrap().enabled);
    let options = parse(&["--dev"]).unwrap();
    assert!(options.enabled);
    assert!(options.dev_db().ends_with(".dev/data.db"));
    let explicit = parse(&["--dev", "--config-db", "local.db", "--port", "8123"]).unwrap();
    assert_eq!(explicit.dev_db(), "local.db");
    let mut config = crate::config::Config::default();
    explicit.apply(&mut config);
    assert_eq!(config.server.port, 8123);
    assert_eq!(
        PathBuf::from(&config.memory.db_path),
        std::path::absolute("local.db").unwrap()
    );
    assert_eq!(
        crate::device::echomuse::registry::data_dir(&config),
        std::env::current_dir().unwrap()
    );
}

#[test]
fn invalid_development_options_do_not_fall_back_to_production_paths() {
    for args in [
        vec!["--dev", "--config-db"],
        vec!["--dev", "--config-db", ":memory:"],
        vec!["--port", "8001"],
        vec!["--dev", "--port", "0"],
        vec!["--dev", "--port", "65536"],
        vec!["--dev", "--unknown"],
    ] {
        assert!(parse(&args).is_err(), "{args:?}");
    }
}

#[test]
fn all_model_entry_aliases_are_blocked_but_admin_remains_real() {
    for path in [
        "/api/ws",
        "/api/test/run",
        "/api/test/capabilities",
        "/api/test/tts",
        "/api/test/asr",
        "/api/test/vad",
        "/api/test/llm",
        "/api/test/llm/models",
        "/api/test/nlu",
        "/api/test/pipeline",
        "/api/nlu/model/build",
        "/api/nlu/model/build/log",
        "/control",
        "/data",
        "/em/control",
        "/em/data",
        "/api/devices/a/inject",
    ] {
        assert!(blocked(path), "{path}");
    }
    for path in [
        "/",
        "/em/",
        "/api/config",
        "/api/config/schema",
        "/api/health",
        "/api/plugins",
        "/api/health/engines",
        "/api/admin/ws",
        "/api/logs",
        "/api/devices",
        "/api/echomuse/devices",
        "/api/files",
        "/api/soul/preview",
    ] {
        assert!(!blocked(path), "{path}");
    }
}

#[tokio::test]
async fn guard_returns_real_unavailability_and_preserves_authentication() {
    use axum::{body::Body, http::Request as HttpRequest, routing::get, Router};
    use tower::ServiceExt;
    let store = crate::config::store::sqlite::SqliteConfigStore::open(":memory:").unwrap();
    let mut config = crate::config::Config::default();
    config.server.expected_token = "dev-test-token".into();
    let engines = Engines::new_dev(&config, store).unwrap();
    let app = Router::new()
        .route(
            "/api/test/run",
            get(|| async { "provider dispatch must not run" }),
        )
        .layer(axum::middleware::from_fn_with_state(engines, guard));
    let unauth = app
        .clone()
        .oneshot(
            HttpRequest::builder()
                .uri("/api/test/run")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(unauth.status(), StatusCode::UNAUTHORIZED);
    let denied = app
        .oneshot(
            HttpRequest::builder()
                .uri("/api/test/run?token=dev-test-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(denied.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body = axum::body::to_bytes(denied.into_body(), 8192)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["error_code"], "dev_inference_disabled");
    assert_eq!(json["inference_enabled"], false);
}
