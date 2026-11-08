//! 生命周期端点回归：依赖前置 409、依赖者停用门、Active 禁移除、
//! 升级/降级判定、停用意图持久化与 Registered-not-active UI 404。
//! 挂真实产物目录（生命周期端点需要读盘产物），与 tests.rs（清单/动作/UI 契约）分离。

#![cfg(feature = "wasm-plugins")]

use std::sync::Arc;

use crate::config::store::sqlite::SqliteConfigStore;
use crate::config::store::ConfigStore;
use crate::config::Config;
use crate::runtime::Engines;

use super::{b64, fixture, fixture_at, fixture_bytes, get_json, Server};

/// 挂真实产物目录的回环（生命周期端点需要读盘产物）。
pub(super) async fn lifecycle_fixture(tag: &str) -> (String, Server, Arc<Engines>) {
    let dir = std::env::temp_dir().join(format!("xz-wasm-lc-{tag}-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let db = dir.join("config.db");
    fixture_at(&db).await
}

#[tokio::test]
async fn lifecycle_endpoints_full_flow_with_dependency_gate() {
    let (base, _server, engines) = lifecycle_fixture("flow").await;
    let client = reqwest::Client::new();
    async fn upload(base: &str, client: &reqwest::Client, id: &str, fx: &str) -> reqwest::Response {
        let bytes = fixture_bytes(fx);
        client
            .post(format!("{base}/api/plugins/upload"))
            .json(&serde_json::json!({"id": id, "wasm_base64": b64(&bytes)}))
            .send()
            .await
            .unwrap()
    }
    async fn post(base: &str, client: &reqwest::Client, path: &str) -> reqwest::Response {
        client.post(format!("{base}{path}")).send().await.unwrap()
    }

    // greeter-user 先行：注册可过（注册不判依赖），激活 409 列缺口（hello 缺失）
    assert_eq!(
        upload(&base, &client, "greeter.usermade", "greeter-user")
            .await
            .status(),
        200
    );
    assert_eq!(
        post(&base, &client, "/api/plugins/greeter.usermade/register")
            .await
            .status(),
        200
    );
    let resp = post(&base, &client, "/api/plugins/greeter.usermade/activate").await;
    assert_eq!(resp.status(), 409, "依赖缺失应 409");
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(
        body["missing"][0]["id"],
        serde_json::json!("hello.official"),
        "{body}"
    );
    assert_eq!(body["missing"][0]["reason"], serde_json::json!("missing"));

    // hello 激活后依赖满足 → greeter 激活成功
    assert_eq!(
        upload(&base, &client, "hello.official", "hello")
            .await
            .status(),
        200
    );
    assert_eq!(
        post(&base, &client, "/api/plugins/hello.official/register")
            .await
            .status(),
        200
    );
    assert_eq!(
        post(&base, &client, "/api/plugins/hello.official/activate")
            .await
            .status(),
        200
    );
    assert_eq!(
        post(&base, &client, "/api/plugins/greeter.usermade/activate")
            .await
            .status(),
        200
    );

    // 依赖者 Active 时停用 hello → 409 列依赖者
    let resp = post(&base, &client, "/api/plugins/hello.official/deactivate").await;
    assert_eq!(resp.status(), 409, "{:?}", resp.text().await.unwrap());
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(
        body["dependents"],
        serde_json::json!(["greeter.usermade"]),
        "{body}"
    );

    // Active 禁移除 → 409
    let resp = post(&base, &client, "/api/plugins/hello.official/remove").await;
    assert_eq!(resp.status(), 409);

    // 依赖者先停用 → hello 可停用；无依赖者停用 200
    assert_eq!(
        post(&base, &client, "/api/plugins/greeter.usermade/deactivate")
            .await
            .status(),
        200
    );
    assert_eq!(
        post(&base, &client, "/api/plugins/hello.official/deactivate")
            .await
            .status(),
        200
    );

    // 仅非激活可移除：Registered 可移除（产物+标记+kv 一并回收）
    let dir = engines.wasm_artifact_dir().unwrap();
    assert!(dir.path().join("greeter.usermade.wasm").exists());
    assert_eq!(
        post(&base, &client, "/api/plugins/greeter.usermade/remove")
            .await
            .status(),
        200
    );
    assert!(!dir.path().join("greeter.usermade.wasm").exists());
    assert!(
        dir.path().join("greeter.usermade.registered.json").exists() == false,
        "移除应删除注册标记"
    );
    // 再移除 → 404
    assert_eq!(
        post(&base, &client, "/api/plugins/greeter.usermade/remove")
            .await
            .status(),
        404
    );

    // 升级判定：同 id 同版本重传 → 409（新 ≤ 旧）
    let resp = upload(&base, &client, "hello.official", "hello").await;
    assert_eq!(resp.status(), 409, "同版本重传应 409");
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(
        body["error"]
            .as_str()
            .unwrap_or_default()
            .contains("版本不升级"),
        "{body}"
    );
}

#[tokio::test]
async fn deactivate_persists_disabled_intent_and_register_failure_keeps_staged() {
    let (base, _server, engines) = lifecycle_fixture("intent").await;
    let client = reqwest::Client::new();
    // weather 无依赖：上传→注册→激活→停用
    let resp = client
        .post(format!("{base}/api/plugins/upload"))
        .json(&serde_json::json!({
            "id": "weather.official",
            "wasm_base64": b64(&fixture_bytes("weather")),
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    // 未注册直接激活 → 409 WrongLifecycle
    let resp = client
        .post(format!("{base}/api/plugins/weather.official/activate"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 409);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["lifecycle"], serde_json::json!("staged"), "{body}");

    // 垃圾产物无法构造（上传已拦）：此处验证停用写 enabled=false
    assert_eq!(
        client
            .post(format!("{base}/api/plugins/weather.official/register"))
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
    assert_eq!(
        client
            .post(format!("{base}/api/plugins/weather.official/activate"))
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
    assert_eq!(
        client
            .post(format!("{base}/api/plugins/weather.official/deactivate"))
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
    let persisted = engines.config_store.load().unwrap();
    assert!(
        !persisted.plugins.entries["weather.official"].enabled,
        "停用应持久化 enabled=false"
    );
    // 停用后实例卸载：动作不可用；UI 404
    let ui = client
        .get(format!("{base}/api/plugins/weather.official/ui"))
        .send()
        .await
        .unwrap();
    assert_eq!(ui.status(), 404, "停用插件不返回业务 UI");
}

// ============================================================
// raw 直传：登记 Staged 语义 + id 探测（memory 库即可，无产物目录依赖）
// ============================================================

#[tokio::test]
async fn raw_upload_stages_component() {
    let (base, _server, _engines) = fixture().await;
    let client = reqwest::Client::new();
    let bytes = fixture_bytes("weather");
    let resp = client
        .post(format!("{base}/api/plugins/upload/raw"))
        .query(&[("id", "weather.official")])
        .header("content-type", "application/octet-stream")
        .body(bytes)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["ok"], serde_json::json!(true), "{body}");
    assert_eq!(body["staged"], serde_json::json!(true));
    assert_eq!(body["lifecycle"], serde_json::json!("staged"));
    // 登记（不装载）即可见于清单
    let list = get_json(&client, &base, "/api/plugins").await;
    let found = list["wasm"]["plugins"]
        .as_array()
        .unwrap()
        .iter()
        .any(|p| p["id"] == "weather.official");
    assert!(found, "登记后清单应含该插件");
}

#[tokio::test]
async fn raw_upload_gate_is_platform_level_and_per_item_enabled_allows_upload() {
    let mut cfg = Config::default();
    cfg.memory.db_path = std::env::temp_dir()
        .join(format!("xz-wasm-gate-{}", uuid::Uuid::new_v4()))
        .join("data.db")
        .display()
        .to_string();
    // 每插件 enabled=false 是激活意图，不再拦上传（禁用插件允许升级产物）
    cfg.plugins.entries.insert(
        "hello.official".into(),
        crate::config::PluginEntryConfig {
            enabled: false,
            ..Default::default()
        },
    );
    let store = SqliteConfigStore::open(":memory:").unwrap();
    store.persist(&cfg).unwrap();
    let engines = Engines::new_dev(&cfg, store).unwrap();
    let app = crate::gateway::ws::router(engines.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let _handle = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{base}/api/plugins/upload/raw"))
        .query(&[("id", "hello.official")])
        .header("content-type", "application/octet-stream")
        .body(fixture_bytes("hello"))
        .send()
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        200,
        "条目禁用只是停用意图：上传仍登记 Staged"
    );
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["lifecycle"], serde_json::json!("staged"), "{body}");

    // 平台总闸（[plugins].enabled=false）一票否决
    let mut off = Config::default();
    off.memory.db_path = cfg.memory.db_path.clone();
    off.plugins.enabled = false;
    let store = SqliteConfigStore::open(":memory:").unwrap();
    store.persist(&off).unwrap();
    let engines = Engines::new_dev(&off, store).unwrap();
    let app = crate::gateway::ws::router(engines.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let _handle = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{base}/api/plugins/upload/raw"))
        .query(&[("id", "hello.official")])
        .header("content-type", "application/octet-stream")
        .body(fixture_bytes("hello"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 403, "平台总闸关闭应拒绝上传");
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(
        body["error"].as_str().unwrap_or_default().contains("总闸"),
        "{body}"
    );
}

#[tokio::test]
async fn raw_upload_without_id_discovers_manifest_id() {
    let (base, _server, _engines) = fixture().await;
    let client = reqwest::Client::new();
    // id 缺省：服务端从组件 manifest 探测（"选文件即上传"——文件名无关）
    let resp = client
        .post(format!("{base}/api/plugins/upload/raw"))
        .header("content-type", "application/octet-stream")
        .body(fixture_bytes("weather"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["id"], serde_json::json!("weather.official"), "{body}");
    assert_eq!(body["staged"], serde_json::json!(true));
}
