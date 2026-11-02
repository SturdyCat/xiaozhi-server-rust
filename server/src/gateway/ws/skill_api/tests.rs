//! Real loopback HTTP + SQLite management contracts. No remote training, inference or audio.
use super::*;
use crate::config::store::{sqlite::SqliteConfigStore, ConfigStore};

mod clocks;

struct Server {
    handle: tokio::task::JoinHandle<()>,
    _slot: tokio::sync::OwnedSemaphorePermit,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.handle.abort();
    }
}
async fn fixture(cfg: Config) -> (String, Server, Arc<Engines>) {
    // HTTP management intentionally has only four worker slots process-wide. Keep unrelated
    // parallel test servers from turning normal CRUD assertions into admission-race tests.
    static SERVERS: OnceLock<Arc<tokio::sync::Semaphore>> = OnceLock::new();
    let slot = SERVERS
        .get_or_init(|| Arc::new(tokio::sync::Semaphore::new(2)))
        .clone()
        .acquire_owned()
        .await
        .unwrap();
    let store = SqliteConfigStore::open(":memory:").unwrap();
    store.persist(&cfg).unwrap();
    let engines = Engines::new_dev(&cfg, store).unwrap();
    let app = routes().with_state(engines.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let handle = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (
        base,
        Server {
            handle,
            _slot: slot,
        },
        engines,
    )
}
fn config() -> Config {
    let mut cfg = Config::default();
    cfg.server.expected_token = "skill-admin".into();
    cfg.memory.db_path = std::env::temp_dir()
        .join(format!("xz-skill-api-{}", uuid::Uuid::new_v4()))
        .join("data.db")
        .display()
        .to_string();
    cfg
}
async fn get(client: &reqwest::Client, base: &str, path: &str) -> reqwest::Response {
    client
        .get(format!("{base}{path}"))
        .bearer_auth("skill-admin")
        .send()
        .await
        .unwrap()
}
async fn post(client: &reqwest::Client, base: &str, path: &str, body: Value) -> reqwest::Response {
    client
        .post(format!("{base}{path}"))
        .bearer_auth("skill-admin")
        .json(&body)
        .send()
        .await
        .unwrap()
}

#[tokio::test]
async fn catalog_is_saved_effective_seed_and_command_not_a_demo_response() {
    let cfg = config();
    let before = serde_json::to_value(&cfg).unwrap();
    let (base, _server, engines) = fixture(cfg).await;
    let client = reqwest::Client::new();
    assert_eq!(
        client
            .get(format!("{base}/api/skills/catalog"))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    let result: Value = get(&client, &base, "/api/skills/catalog")
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(result["state"], "ok");
    assert_eq!(result["recognition"], "model_only");
    assert_eq!(
        result["revision"],
        super::super::config_revision(&engines.config_store.load().unwrap())
    );
    let intents = result["intents"].as_array().unwrap();
    let saved = engines.config_store.load().unwrap();
    let expected = crate::plugins::skill::effective_catalog(&saved.command, &saved.nlu).unwrap();
    assert_eq!(
        intents
            .iter()
            .map(|i| i["name"].as_str().unwrap())
            .collect::<Vec<_>>(),
        expected
            .intents
            .iter()
            .map(|i| i.name.as_str())
            .collect::<Vec<_>>()
    );
    for seed in &crate::plugins::nlu::NluConfig::default().intents {
        let mut returned = intents
            .iter()
            .find(|i| i["name"] == seed.name)
            .unwrap()
            .clone();
        assert_eq!(returned["source"], "default");
        returned.as_object_mut().unwrap().remove("source");
        assert_eq!(returned, serde_json::to_value(seed).unwrap());
    }
    assert!(intents.iter().any(|i| i["source"] == "default"));
    assert!(intents
        .iter()
        .any(|i| i["name"] == "end_session" && i["source"] == "command"));
    assert!(!intents.iter().any(|i| i["name"] == "help_guide_demo"));
    assert_eq!(result["executed"], false);
    let wire = result.to_string();
    assert!(!wire.contains("bert_model"));
    assert!(!wire.contains("skill-admin"));
    assert_eq!(
        serde_json::to_value(engines.config_store.load().unwrap()).unwrap(),
        before
    );
    let caps: Value = get(&client, &base, "/api/skills/capabilities")
        .await
        .json()
        .await
        .unwrap();
    assert!(caps["capabilities"]
        .as_array()
        .unwrap()
        .iter()
        .any(|c| c["api"] == "tasks.create" && c["args_schema"].is_object()));
}

#[tokio::test]
async fn explicit_empty_is_preserved_and_custom_saved_definition_is_not_replaced() {
    let mut cfg = config();
    cfg.command.enabled = false;
    cfg.nlu.intents.clear();
    let (base, _server, engines) = fixture(cfg).await;
    let client = reqwest::Client::new();
    let empty: Value = get(&client, &base, "/api/skills/catalog")
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(empty["intents"], json!([]));
    assert_eq!(empty["effective"]["enabled"], false);
    let mut saved = engines.config_store.load().unwrap();
    saved.nlu.intents = serde_json::from_value(json!([{"name":"help_guide_demo","templates":["显示使用帮助"],"action":{"kind":"reply","reply":"用户自己的帮助"}}])).unwrap();
    engines.config_store.persist(&saved).unwrap();
    let custom: Value = get(&client, &base, "/api/skills/catalog")
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(custom["intents"].as_array().unwrap().len(), 1);
    assert_eq!(custom["intents"][0]["source"], "custom");
    assert_eq!(custom["intents"][0]["action"]["reply"], "用户自己的帮助");
    assert_ne!(empty["revision"], custom["revision"]);
}

#[tokio::test]
async fn task_crud_is_authenticated_persistent_device_bound_and_does_not_claim_playback() {
    let (base, _server, engines) = fixture(config()).await;
    let client = reqwest::Client::new();
    let input = json!({"device_id":"device-A","request_id":"http-retry-key","args":{"text":"喝水","after":"十分钟"}});
    assert_eq!(
        client
            .post(format!("{base}/api/skill-tasks"))
            .json(&input)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    let response = post(&client, &base, "/api/skill-tasks", input.clone()).await;
    assert_eq!(response.status(), StatusCode::OK);
    let created: Value = response.json().await.unwrap();
    assert_eq!(created["state"], "ok");
    assert_eq!(created["task"]["device_id"], "device-A");
    assert_eq!(created["task"]["state"], "active");
    assert_eq!(created["played"], false);
    assert_eq!(created["socket_written"], false);
    let id = created["task"]["id"].as_str().unwrap();
    let retry: Value = post(&client, &base, "/api/skill-tasks", input)
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(retry["deduplicated"], true);
    assert_eq!(retry["task"]["id"], id);
    assert_eq!(post(&client, &base, "/api/skill-tasks", json!({"device_id":"device-A","request_id":"http-retry-key","args":{"text":"不同请求","after":"十分钟"}})).await.status(), StatusCode::CONFLICT);
    let all: Value = get(&client, &base, "/api/skill-tasks?limit=10&offset=0")
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(all["total"], 1);
    let other: Value = get(&client, &base, "/api/skill-tasks?device_id=device-B")
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(other["total"], 0);
    assert_eq!(
        get(
            &client,
            &base,
            &format!("/api/skill-tasks/{id}?device_id=device-B")
        )
        .await
        .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        post(
            &client,
            &base,
            &format!("/api/skill-tasks/{id}/cancel"),
            json!({"device_id":"device-B"})
        )
        .await
        .status(),
        StatusCode::NOT_FOUND
    );
    let cancelled: Value = post(
        &client,
        &base,
        &format!("/api/skill-tasks/{id}/cancel"),
        json!({"device_id":"device-A"}),
    )
    .await
    .json()
    .await
    .unwrap();
    assert_eq!(cancelled["state"], "ok");
    assert_eq!(cancelled["task_state"], "cancelled");
    let loaded: Value = get(&client, &base, &format!("/api/skill-tasks/{id}"))
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(loaded["task"]["state"], "cancelled");
    assert!(engines.dev_mode);
    // A separate service object opens the same SQLite path: not an in-memory demo task.
    assert_eq!(
        engines
            .skill_system
            .get_task(id, Some("device-A"))
            .unwrap()
            .unwrap()["state"],
        "cancelled"
    );
}

#[tokio::test]
async fn task_bounds_and_extra_fields_are_rejected_without_dispatch() {
    let (base, _server, _) = fixture(config()).await;
    let client = reqwest::Client::new();
    for query in ["limit=0", "limit=101", "offset=10001", "limit=bad"] {
        assert_eq!(
            get(&client, &base, &format!("/api/skill-tasks?{query}"))
                .await
                .status(),
            StatusCode::BAD_REQUEST
        );
    }
    for input in [
        json!({"device_id":"device-A","args":{"text":"x","after":"一分钟","device_id":"device-B"}}),
        json!({"device_id":"device-A","args":{"text":"x","after":"一分钟"},"api":"messages.send"}),
        json!({"device_id":"device-A","args":{"text":"x","after":"一分钟","daily":"08:00"}}),
        json!({"device_id":"device-A","args":{"text":"x","after":"一分钟","timezone":"External/Zone"}}),
    ] {
        assert_eq!(
            post(&client, &base, "/api/skill-tasks", input)
                .await
                .status(),
            StatusCode::BAD_REQUEST
        );
    }
    let result: Value = get(&client, &base, "/api/skill-tasks")
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(result["total"], 0);
    assert_eq!(
        get(&client, &base, "/api/skill-tasks/not-a-uuid")
            .await
            .status(),
        StatusCode::BAD_REQUEST
    );
}
