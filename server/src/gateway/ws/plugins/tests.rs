//! WASM 插件端点契约（feature 门控）：真实回环 HTTP + 真实 wasm 夹具（产物入库）。
//! 鉴权不在本层（main.rs 全局 layer）；这里测路由注册、合并投影、白名单与扩展点解析回显。

#![cfg(feature = "wasm-plugins")]

use std::sync::Arc;

use crate::config::store::sqlite::SqliteConfigStore;
use crate::config::store::ConfigStore;
use crate::config::Config;
use crate::runtime::Engines;

struct Server {
    handle: tokio::task::JoinHandle<()>,
}

impl Drop for Server {
    fn drop(&mut self) {
        self.handle.abort();
    }
}

/// 产物不入库（用户方向）：按需本地构建（缺失/源更新自动重建）。
fn fixture_bytes(name: &str) -> Vec<u8> {
    crate::plugins::wasm::tests::ensure_fixture(name)
}

async fn fixture() -> (String, Server, Arc<Engines>) {
    let mut cfg = Config::default();
    cfg.memory.db_path = std::env::temp_dir()
        .join(format!("xz-wasm-api-{}", uuid::Uuid::new_v4()))
        .join("data.db")
        .display()
        .to_string();
    let store = SqliteConfigStore::open(":memory:").unwrap();
    store.persist(&cfg).unwrap();
    let engines = Engines::new_dev(&cfg, store).unwrap();
    let wasm = engines
        .wasm
        .as_ref()
        .expect("feature 编入时宿主必在（空注册表起步）");
    wasm.load("hello.official", &fixture_bytes("hello"))
        .expect("load hello");
    wasm.load("greeter.usermade", &fixture_bytes("greeter-user"))
        .expect("load usermade");
    serve(engines).await
}

/// 文件配置库版本（产物目录 = 库旁 plugins/）：上传/重启重装语义需要真实落盘点。
/// 不预装任何插件——上传测试自建状态。
async fn fixture_at(db_path: &std::path::Path) -> (String, Server, Arc<Engines>) {
    let mut cfg = Config::default();
    cfg.memory.db_path = db_path.with_file_name("memory.db").display().to_string();
    let store = SqliteConfigStore::open(db_path.to_str().unwrap()).unwrap();
    store.persist(&cfg).unwrap();
    let engines = Engines::new_dev(&cfg, store).unwrap();
    serve(engines).await
}

async fn serve(engines: Arc<Engines>) -> (String, Server, Arc<Engines>) {
    let app = crate::gateway::ws::router(engines.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let handle = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (base, Server { handle }, engines)
}

fn b64(bytes: &[u8]) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

async fn get_json(client: &reqwest::Client, base: &str, path: &str) -> serde_json::Value {
    client
        .get(format!("{base}{path}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap()
}

#[tokio::test]
async fn plugins_list_merges_wasm_projection_with_static_capabilities() {
    let (base, _server, _engines) = fixture().await;
    let client = reqwest::Client::new();
    let body = get_json(&client, &base, "/api/plugins").await;

    assert_eq!(body["ok"], serde_json::json!(true));
    // 静态能力清单仍在（旧客户端零破坏：只追加 wasm 键）
    assert!(
        body.get("asr").is_some() || body.get("capabilities").is_some(),
        "{body}"
    );
    // wasm 投影：两个已装载插件 + 扩展点解析（官方默认解析 + 用户实现并存）
    let plugins = body["wasm"]["plugins"].as_array().expect("wasm.plugins");
    assert_eq!(plugins.len(), 2);
    assert!(plugins.iter().any(|p| p["id"] == "hello.official"));
    assert_eq!(
        body["wasm"]["extensions"]["demo.greeter"]["state"],
        "resolved"
    );
    assert_eq!(
        body["wasm"]["extensions"]["demo.greeter"]["implementation"],
        "hello.official"
    );
}

#[tokio::test]
async fn unified_action_endpoint_roundtrip_and_whitelist() {
    let (base, _server, _engines) = fixture().await;
    let client = reqwest::Client::new();

    // 声明过的动作：真实 wasm 调用，结果透传（raw 保留原文）
    let resp = client
        .post(format!("{base}/api/plugins/hello.official/actions/greet"))
        .json(&serde_json::json!({"name": "管理台"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["ok"], serde_json::json!(true), "{body}");
    assert!(
        body["raw"].as_str().unwrap_or_default().contains("你好"),
        "动作结果：{body}"
    );

    // 未声明的动作：白名单拒绝（模型/客户端不能任意指定动作名）
    let resp = client
        .post(format!(
            "{base}/api/plugins/hello.official/actions/format-disk"
        ))
        .json(&serde_json::json!(null))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(
        body["error"]
            .as_str()
            .unwrap_or_default()
            .contains("未声明动作"),
        "{body}"
    );

    // 未装载的插件：400（区别于路由不存在）
    let resp = client
        .post(format!("{base}/api/plugins/ghost.plugin/actions/greet"))
        .json(&serde_json::json!(null))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
}

#[tokio::test]
async fn extension_preference_roundtrip_with_reflection() {
    let (base, _server, _engines) = fixture().await;
    let client = reqwest::Client::new();

    // 首选用户实现 → 解析切换并即时回显
    let resp = client
        .post(format!(
            "{base}/api/plugins/extensions/demo.greeter/preference"
        ))
        .json(&serde_json::json!({"implementation": "greeter.usermade"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["ok"], serde_json::json!(true), "{body}");
    assert_eq!(body["resolution"]["state"], "resolved");
    assert_eq!(body["resolution"]["implementation"], "greeter.usermade");

    // 清单投影同步反映
    let list = get_json(&client, &base, "/api/plugins").await;
    assert_eq!(
        list["wasm"]["extensions"]["demo.greeter"]["implementation"],
        "greeter.usermade"
    );

    // 回官方默认（null）
    let resp = client
        .post(format!(
            "{base}/api/plugins/extensions/demo.greeter/preference"
        ))
        .json(&serde_json::json!({"implementation": null}))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["resolution"]["implementation"], "hello.official");

    // 非法形状拒绝
    let resp = client
        .post(format!(
            "{base}/api/plugins/extensions/demo.greeter/preference"
        ))
        .json(&serde_json::json!({"implementation": 42}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
}

// ============================================================
// 上传即启用（P2b）：验证 → 采用 → 热装载 → 重启重装 → 卸载
// ============================================================

#[tokio::test]
async fn upload_validates_activates_and_persists_across_restart() {
    let dir = std::env::temp_dir().join(format!("xz-wasm-up-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let db = dir.join("config.db");
    let (base, server, engines) = fixture_at(&db).await;
    let client = reqwest::Client::new();

    // 上传已构建的 weather 组件（服务端零本地构建）：隔离验证全过 → 采用 + 热装载
    let resp = client
        .post(format!("{base}/api/plugins/upload"))
        .json(&serde_json::json!({
            "id": "weather.official",
            "wasm_base64": b64(&fixture_bytes("weather")),
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200, "合法产物应被采用");
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["activated"], serde_json::json!(true), "{body}");
    assert_eq!(body["persisted"], serde_json::json!(true), "{body}");
    assert!(
        body["validation"]
            .as_array()
            .unwrap()
            .iter()
            .all(|r| r["ok"] == serde_json::json!(true)),
        "{body}"
    );

    // 立即提供功能：清单可见 + 扩展点解析 + 动作可调（无任何构建步骤）
    let list = get_json(&client, &base, "/api/plugins").await;
    assert_eq!(
        list["wasm"]["extensions"]["skill.weather"]["implementation"],
        "weather.official"
    );
    let act = client
        .post(format!(
            "{base}/api/plugins/weather.official/actions/forecast"
        ))
        .json(&serde_json::json!({"city": "杭州"}))
        .send()
        .await
        .unwrap();
    assert_eq!(act.status(), 200);

    // 重启重装：同数据卷重建 engines（旧 host 连同 kv/epoch 线程一起销毁）
    drop(server);
    drop(engines);
    let (base2, _server2, engines2) = fixture_at(&db).await;
    let list2 = get_json(&client, &base2, "/api/plugins").await;
    let ids: Vec<&str> = list2["wasm"]["plugins"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["id"].as_str().unwrap())
        .collect();
    assert!(
        ids.contains(&"weather.official"),
        "重启后应自动重装：{ids:?}"
    );
    assert_eq!(
        engines2
            .wasm
            .as_ref()
            .unwrap()
            .kv()
            .len_for("weather.official"),
        0,
        "kv 是派生缓存：重启清空重建"
    );
}

#[tokio::test]
async fn upload_rejects_invalid_artifact_without_side_effects() {
    let (base, _server, engines) = fixture().await;
    let client = reqwest::Client::new();

    // 垃圾字节：隔离验证失败 → 400，不装载、不落盘、不影响既有插件
    let resp = client
        .post(format!("{base}/api/plugins/upload"))
        .json(&serde_json::json!({
            "id": "junk.plugin",
            "wasm_base64": b64(b"this is not wasm"),
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);

    // 合法组件但 id 与 manifest 不一致：同样拒绝（manifest.id 是唯一身份）
    let resp = client
        .post(format!("{base}/api/plugins/upload"))
        .json(&serde_json::json!({
            "id": "imposter.plugin",
            "wasm_base64": b64(&fixture_bytes("hello")),
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
    let list = get_json(&client, &base, "/api/plugins").await;
    assert_eq!(
        list["wasm"]["plugins"].as_array().unwrap().len(),
        2,
        "原状不动"
    );
    assert!(engines.wasm.as_ref().unwrap().kv().len_for("junk.plugin") == 0);
}

#[tokio::test]
async fn unload_removes_plugin_and_artifact() {
    let dir = std::env::temp_dir().join(format!("xz-wasm-un-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let db = dir.join("config.db");
    let (base, _server, engines) = fixture_at(&db).await;
    let client = reqwest::Client::new();

    let upload = client
        .post(format!("{base}/api/plugins/upload"))
        .json(&serde_json::json!({
            "id": "greeter.usermade",
            "wasm_base64": b64(&fixture_bytes("greeter-user")),
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(upload.status(), 200);
    let upload_body: serde_json::Value = upload.json().await.unwrap();
    assert_eq!(
        upload_body["persisted"],
        serde_json::json!(true),
        "{upload_body}"
    );
    let persisted_cfg = engines.config_store.load().expect("config pin");
    assert_eq!(
        persisted_cfg.plugins.entries["greeter.usermade"].artifact,
        upload_body["sha256"].as_str().map(str::to_string)
    );
    // 写入 kv 便于断言注册-回收
    engines
        .wasm
        .as_ref()
        .unwrap()
        .handle_event("greeter.usermade", "user.fetch", "")
        .ok();
    let artifact_dir = engines.wasm_artifact_dir().unwrap();
    assert!(artifact_dir.path().join("greeter.usermade.wasm").exists());

    let resp = client
        .post(format!("{base}/api/plugins/greeter.usermade/unload"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["unloaded"], serde_json::json!(true), "{body}");
    assert_eq!(body["artifact_removed"], serde_json::json!(true), "{body}");
    // 注册-回收：kv 命名空间清空、产物文件删除
    assert_eq!(
        engines
            .wasm
            .as_ref()
            .unwrap()
            .kv()
            .len_for("greeter.usermade"),
        0
    );
    assert!(!artifact_dir.path().join("greeter.usermade.wasm").exists());
    let after_unload = engines.config_store.load().expect("config after unload");
    assert_eq!(
        after_unload.plugins.entries["greeter.usermade"].artifact, None,
        "卸载应清除摘要钉版"
    );

    // 再卸 → 404（幂等边界）
    let resp = client
        .post(format!("{base}/api/plugins/greeter.usermade/unload"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);
}

// ============================================================
// P3：raw 直传端点（管理台上传桥通道）+ 配置门
// ============================================================

#[tokio::test]
async fn raw_upload_activates_component() {
    let (base, _server, _engines) = fixture().await;
    let client = reqwest::Client::new();
    let bytes = fixture_bytes("greeter-user");
    let resp = client
        .post(format!("{base}/api/plugins/upload/raw"))
        .query(&[("id", "greeter.usermade")])
        .header("content-type", "application/octet-stream")
        .body(bytes)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["ok"], serde_json::json!(true), "{body}");
    assert_eq!(body["activated"], serde_json::json!(true));
    // 热装载生效：清单立即可见
    let list = get_json(&client, &base, "/api/plugins").await;
    let found = list["wasm"]["plugins"]
        .as_array()
        .unwrap()
        .iter()
        .any(|p| p["id"] == "greeter.usermade");
    assert!(found, "上传后清单应含该插件");
}

#[tokio::test]
async fn raw_upload_rejected_for_config_disabled_id() {
    let mut cfg = Config::default();
    cfg.memory.db_path = std::env::temp_dir()
        .join(format!("xz-wasm-gate-{}", uuid::Uuid::new_v4()))
        .join("data.db")
        .display()
        .to_string();
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
    assert_eq!(resp.status(), 403, "配置禁用的 id 应拒绝上传");
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(
        body["error"]
            .as_str()
            .unwrap_or_default()
            .contains("配置禁用"),
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
        .body(fixture_bytes("greeter-user"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["id"], serde_json::json!("greeter.usermade"), "{body}");
    assert_eq!(body["activated"], serde_json::json!(true));
}

// ============================================================
// A2UI 最小闭环：GET /api/plugins/{id}/ui 只读投影 + 未注册不返回业务 UI
// ============================================================

#[tokio::test]
async fn plugin_ui_endpoint_serves_active_descriptor_and_404_for_unregistered() {
    let (base, _server, _engines) = fixture().await;
    let client = reqwest::Client::new();

    // 已装载（Active）插件：整份 UI 描述对象透传（解析后，非转义字符串）
    let resp = client
        .get(format!("{base}/api/plugins/hello.official/ui"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["ok"], serde_json::json!(true), "{body}");
    assert_eq!(body["id"], serde_json::json!("hello.official"));
    assert_eq!(body["phase"], serde_json::json!("active"));
    assert_eq!(body["ui"]["a2ui"], serde_json::json!("0.1"), "{body}");
    assert_eq!(body["ui"]["pages"].as_array().map(Vec::len), Some(2));
    assert_eq!(
        body["ui"]["pages"][0]["bindings"]["config"][0],
        serde_json::json!("greeting")
    );
    // 嵌套组件树（0.1 超集）：config 页 card→column[field,hint]；
    // test 页 card→column[input(name), actions(id 数组), result]
    assert_eq!(
        body["ui"]["pages"][0]["tree"]["type"],
        serde_json::json!("card"),
        "{body}"
    );
    let children = &body["ui"]["pages"][1]["tree"]["child"]["children"];
    assert_eq!(children[0]["type"], serde_json::json!("input"), "{body}");
    assert_eq!(children[0]["id"], serde_json::json!("name"));
    assert_eq!(
        children[1]["actions"],
        serde_json::json!(["greet"]),
        "actions 是动作 id 字符串数组：{body}"
    );
    assert_eq!(children[2]["type"], serde_json::json!("result"));

    // 未装载的插件：404，绝不返回任何业务 UI（A2UI 消费方契约）
    let resp = client
        .get(format!("{base}/api/plugins/ghost.plugin/ui"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["ok"], serde_json::json!(false));
    assert!(
        body["error"]
            .as_str()
            .unwrap_or_default()
            .contains("未装载"),
        "{body}"
    );

    // 清单投影同源：registration.ui 只对 Active 插件携带解析后的描述
    let list = get_json(&client, &base, "/api/plugins").await;
    let hello = list["wasm"]["plugins"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["id"] == "hello.official")
        .expect("hello in list");
    assert!(hello["registration"]["ui"].is_object(), "{hello}");
    assert_eq!(
        hello["registration"]["ui"]["a2ui"],
        serde_json::json!("0.1")
    );
    // 未声明 UI 的插件：ui 键恒在（形状稳定），值为 null
    let usermade = list["wasm"]["plugins"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["id"] == "greeter.usermade")
        .expect("usermade in list");
    assert!(
        usermade["registration"]["ui"].is_null(),
        "未声明 UI 应投影 null：{usermade}"
    );
}
