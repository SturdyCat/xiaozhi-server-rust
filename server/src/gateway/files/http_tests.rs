//! 文件管理 **真实 loopback HTTP** 回归：`TestContext`（自带 Config 的 Context 桩）
//! 起 axum 服务，不起真实 Engines。鉴权门禁、端到端 上传→列表→下载→删除 回路、
//! 覆盖矩阵、受保护名、穿越路径与各 4xx 状态码都在此覆盖。
//! 共享夹具（`temp_root`/`canonical`/`json`）复用 `super::tests::`（模式同 echomuse）。

use super::*;

use super::tests::{canonical, json, temp_root};
use std::time::Duration;

/// 测试用 Context：只带 Config（存储根由 `memory.db_path` 父目录推导），
/// 不构造真实 Engines。生产入口 `impl Context for Arc<Engines>` 另行覆盖。
#[derive(Clone)]
struct TestContext {
    config: Arc<Config>,
}
impl Context for TestContext {
    fn config(&self) -> &Config {
        &self.config
    }
}

fn context(root: &Path) -> TestContext {
    let mut config = Config::default();
    // db 放在 root 下 → 存储根 = root（registry::data_dir 的推导）
    config.memory.db_path = root.join("data.db").display().to_string();
    config.server.expected_token = "admin-secret".into();
    TestContext {
        config: Arc::new(config),
    }
}

struct TestServer {
    url: String,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for TestServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn server(ctx: TestContext) -> TestServer {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        axum::serve(listener, http::router::<TestContext>().with_state(ctx))
            .await
            .unwrap();
    });
    TestServer { url, task }
}

/// 发出请求并取（状态码，字节，Content-Type，Content-Disposition）。
async fn send(req: reqwest::RequestBuilder) -> (u16, Vec<u8>, String, String) {
    let resp = req.timeout(Duration::from_secs(5)).send().await.unwrap();
    let status = resp.status().as_u16();
    let header = |name: reqwest::header::HeaderName| -> String {
        resp.headers()
            .get(name)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string()
    };
    let ct = header(reqwest::header::CONTENT_TYPE);
    let disp = header(reqwest::header::CONTENT_DISPOSITION);
    (status, resp.bytes().await.unwrap().to_vec(), ct, disp)
}

#[tokio::test]
async fn http_requires_token_and_accepts_header_or_query() {
    let root = temp_root("http-auth");
    std::fs::create_dir_all(&root).unwrap();
    let srv = server(context(&root)).await;
    let client = reqwest::Client::new();

    // 无 token / 错 token → 401（JSON error）
    let (status, body, ct, _) = send(client.get(format!("{}/api/files", srv.url))).await;
    assert_eq!(status, 401);
    assert!(ct.starts_with("application/json"));
    assert!(json(&body)["error"].is_string());
    let (status, _, _, _) = send(
        client
            .get(format!("{}/api/files", srv.url))
            .bearer_auth("wrong"),
    )
    .await;
    assert_eq!(status, 401);

    // 全端点都要鉴权（含 POST 别名）
    for req in [
        client
            .post(format!("{}/api/files/upload?path=a.txt", srv.url))
            .body(b"x".to_vec()),
        client
            .post(format!("{}/api/files/mkdir", srv.url))
            .body(b"{\"path\":\"a\"}".to_vec()),
        client
            .post(format!("{}/api/files/delete", srv.url))
            .body(b"{\"path\":\"a\"}".to_vec()),
        client.get(format!("{}/api/files/download?path=a.txt", srv.url)),
    ] {
        let (status, _, _, _) = send(req).await;
        assert_eq!(status, 401);
    }

    // ?token= 可用（web fetch / 浏览器拿不到自定义头），空目录列表形状完整
    let (status, body, ct, _) = send(
        client
            .get(format!("{}/api/files", srv.url))
            .query(&[("token", "admin-secret")]),
    )
    .await;
    assert_eq!(status, 200);
    assert!(ct.starts_with("application/json"));
    let v = json(&body);
    assert_eq!(v["root"], canonical(&root));
    assert_eq!(v["path"], "");
    assert_eq!(v["entries"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn http_roundtrip_upload_list_download_delete() {
    let root = temp_root("http-roundtrip");
    std::fs::create_dir_all(&root).unwrap();
    let srv = server(context(&root)).await;
    let client = reqwest::Client::new();
    let token = "admin-secret"; // reqwest bearer_auth 自带 Bearer 前缀

    // mkdir（body JSON）
    let (status, body, _, _) = send(
        client
            .post(format!("{}/api/files/mkdir", srv.url))
            .bearer_auth(token)
            .json(&serde_json::json!({"path": "oww-models"})),
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(
        json(&body),
        serde_json::json!({"ok": true, "path": "oww-models"})
    );

    // 上传裸字节 → 列表可见
    let (status, body, _, _) = send(
        client
            .post(format!(
                "{}/api/files/upload?path={}",
                srv.url, "oww-models/hey_biscuit.onnx"
            ))
            .bearer_auth(token)
            .header(reqwest::header::CONTENT_TYPE, "application/octet-stream")
            .body(b"ONNXDATA".to_vec()),
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(
        json(&body),
        serde_json::json!({"ok": true, "path": "oww-models/hey_biscuit.onnx", "size": 8})
    );
    let (status, body, _, _) = send(
        client
            .get(format!("{}/api/files", srv.url))
            .bearer_auth(token)
            .query(&[("path", "oww-models")]),
    )
    .await;
    assert_eq!(status, 200);
    let v = json(&body);
    assert_eq!(v["path"], "oww-models");
    assert_eq!(v["entries"][0]["name"], "hey_biscuit.onnx");
    assert_eq!(v["entries"][0]["dir"], false);
    assert_eq!(v["entries"][0]["size"], 8);
    assert_eq!(v["entries"][0]["protected"], false);
    assert!(v["entries"][0]["modified"].as_u64().unwrap() > 0);

    // 下载回读（Content-Disposition 提示文件名）
    let (status, bytes, ct, disp) = send(
        client
            .get(format!("{}/api/files/download", srv.url))
            .bearer_auth(token)
            .query(&[("path", "oww-models/hey_biscuit.onnx")]),
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(bytes, b"ONNXDATA");
    assert_eq!(ct, "application/octet-stream");
    assert!(disp.contains("attachment"), "{disp}");
    assert!(disp.contains("filename=\"hey_biscuit.onnx\""), "{disp}");

    // POST 别名删除文件 → 再下载 404，列表为空
    let (status, body, _, _) = send(
        client
            .post(format!("{}/api/files/delete", srv.url))
            .bearer_auth(token)
            .json(&serde_json::json!({"path": "oww-models/hey_biscuit.onnx"})),
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(json(&body), serde_json::json!({"ok": true}));
    let (status, _, _, _) = send(
        client
            .get(format!("{}/api/files/download", srv.url))
            .bearer_auth(token)
            .query(&[("path", "oww-models/hey_biscuit.onnx")]),
    )
    .await;
    assert_eq!(status, 404);
    let (_, body, _, _) = send(
        client
            .get(format!("{}/api/files", srv.url))
            .bearer_auth(token)
            .query(&[("path", "oww-models")]),
    )
    .await;
    assert_eq!(json(&body)["entries"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn http_upload_conflict_overwrite_and_bad_targets() {
    let root = temp_root("http-upload");
    std::fs::create_dir_all(&root).unwrap();
    let srv = server(context(&root)).await;
    let client = reqwest::Client::new();
    let token = "admin-secret";
    let upload = |path: &str, overwrite: Option<&str>| {
        let mut req = client
            .post(format!("{}/api/files/upload", srv.url))
            .query(&[("path", path)])
            .bearer_auth(token)
            .header(reqwest::header::CONTENT_TYPE, "application/octet-stream")
            .body(b"DATA".to_vec());
        if let Some(ow) = overwrite {
            req = req.query(&[("overwrite", ow)]);
        }
        req
    };

    assert_eq!(send(upload("x.txt", None)).await.0, 200);
    // 已存在未 overwrite → 409，overwrite=1 / true → 200
    let (status, body, _, _) = send(upload("x.txt", None)).await;
    assert_eq!(status, 409);
    assert!(json(&body)["error"].is_string());
    assert_eq!(send(upload("x.txt", Some("1"))).await.0, 200);
    assert_eq!(send(upload("x.txt", Some("true"))).await.0, 200);

    // 目录占位 → 409（带 overwrite 也拒绝）
    send(
        client
            .post(format!("{}/api/files/mkdir", srv.url))
            .bearer_auth(token)
            .json(&serde_json::json!({"path": "m"})),
    )
    .await;
    assert_eq!(send(upload("m", Some("1"))).await.0, 409);

    // 空 body / 缺 path / 穿越路径 / 受保护名 → 400/400/400/403
    let (status, _, _, _) = send(
        client
            .post(format!("{}/api/files/upload?path=y.txt", srv.url))
            .bearer_auth(token)
            .header(reqwest::header::CONTENT_TYPE, "application/octet-stream"),
    )
    .await;
    assert_eq!(status, 400);
    let (status, _, _, _) = send(
        client
            .post(format!("{}/api/files/upload", srv.url))
            .bearer_auth(token)
            .header(reqwest::header::CONTENT_TYPE, "application/octet-stream")
            .body(b"DATA".to_vec()),
    )
    .await;
    assert_eq!(status, 400, "缺 path 查询参数");
    assert_eq!(send(upload("../evil", None)).await.0, 400);
    assert_eq!(send(upload("a//b", None)).await.0, 400);
    assert_eq!(send(upload("data.db", None)).await.0, 403);
    // 错误 query token → 401，且文件不落盘
    let (status, _, _, _) = send(
        client
            .post(format!("{}/api/files/upload", srv.url))
            .query(&[("path", "y.txt"), ("token", "nope")])
            .header(reqwest::header::CONTENT_TYPE, "application/octet-stream")
            .body(b"DATA".to_vec()),
    )
    .await;
    assert_eq!(status, 401);
    assert!(!root.join("y.txt").exists(), "被拒绝的上传不能写文件");
}

#[tokio::test]
async fn http_delete_aliases_protected_traversal_and_missing() {
    let root = temp_root("http-delete");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("data.db"), b"db").unwrap();
    std::fs::write(root.join("data.db-wal"), b"wal").unwrap();
    let srv = server(context(&root)).await;
    let client = reqwest::Client::new();
    let token = "admin-secret";
    let del = |path: serde_json::Value| {
        client
            .post(format!("{}/api/files/delete", srv.url))
            .bearer_auth(token)
            .json(&path)
    };

    // 受保护（POST body 与 DELETE query 两个入口一致），文件完好
    let (status, body, _, _) = send(del(serde_json::json!({"path": "data.db"}))).await;
    assert_eq!(status, 403);
    assert!(json(&body)["error"]
        .as_str()
        .unwrap()
        .contains("配置/记忆库"));
    let (status, body, _, _) = send(
        client
            .delete(format!("{}/api/files", srv.url))
            .bearer_auth(token)
            .query(&[("path", "data.db-wal")]),
    )
    .await;
    assert_eq!(status, 403);
    assert!(json(&body)["error"]
        .as_str()
        .unwrap()
        .contains("配置/记忆库"));
    assert!(root.join("data.db").exists());

    // 递归删目录 + DELETE 别名删文件
    send(
        client
            .post(format!("{}/api/files/upload?path=a/b/c.txt", srv.url))
            .bearer_auth(token)
            .header(reqwest::header::CONTENT_TYPE, "application/octet-stream")
            .body(b"x".to_vec()),
    )
    .await;
    assert_eq!(send(del(serde_json::json!({"path": "a"}))).await.0, 200);
    let (status, _, _, _) = send(
        client
            .get(format!("{}/api/files", srv.url))
            .bearer_auth(token)
            .query(&[("path", "a")]),
    )
    .await;
    assert_eq!(status, 404, "目录已删");

    std::fs::write(root.join("q.txt"), b"q").unwrap();
    let (status, _, _, _) = send(
        client
            .delete(format!("{}/api/files", srv.url))
            .bearer_auth(token)
            .query(&[("path", "q.txt")]),
    )
    .await;
    assert_eq!(status, 200);
    assert!(!root.join("q.txt").exists());

    // 根 / 穿越路径 / 缺字段 / 非 JSON body / 缺失 → 400/400/400/400/404
    assert_eq!(send(del(serde_json::json!({"path": ""}))).await.0, 400);
    assert_eq!(send(del(serde_json::json!({"path": "../x"}))).await.0, 400);
    assert_eq!(send(del(serde_json::json!({}))).await.0, 400);
    let (status, body, _, _) = send(
        client
            .post(format!("{}/api/files/delete", srv.url))
            .bearer_auth(token)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(b"not-json".to_vec()),
    )
    .await;
    assert_eq!(status, 400);
    assert!(json(&body)["error"].is_string());
    assert_eq!(send(del(serde_json::json!({"path": "nope"}))).await.0, 404);
}

#[tokio::test]
async fn http_list_and_download_edge_statuses_and_path_normalization() {
    let root = temp_root("http-list");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("data.db"), b"db").unwrap();
    std::fs::create_dir_all(root.join("oww-models")).unwrap();
    let srv = server(context(&root)).await;
    let client = reqwest::Client::new();
    let token = "admin-secret";

    // 缺失目录 404；穿越 400；文件路径 400；空段 400
    for (path, want) in [
        ("missing", 404u16),
        ("../x", 400),
        ("data.db", 400),
        ("a//b", 400),
    ] {
        let (status, _, _, _) = send(
            client
                .get(format!("{}/api/files", srv.url))
                .bearer_auth(token)
                .query(&[("path", path)]),
        )
        .await;
        assert_eq!(status, want, "path={path}");
    }
    // 尾部斜杠规整
    let (status, body, _, _) = send(
        client
            .get(format!("{}/api/files", srv.url))
            .bearer_auth(token)
            .query(&[("path", "oww-models/")]),
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(json(&body)["path"], "oww-models");

    // 根列表能看到保护名标记与目录
    let (_, body, _, _) = send(
        client
            .get(format!("{}/api/files", srv.url))
            .bearer_auth(token),
    )
    .await;
    let entries = json(&body)["entries"].as_array().unwrap().clone();
    let db = entries.iter().find(|e| e["name"] == "data.db").unwrap();
    assert_eq!(db["protected"], true);
    assert_eq!(
        entries.iter().find(|e| e["name"] == "oww-models").unwrap()["dir"],
        true
    );

    // 下载：目录 400、缺失 404
    let (status, _, _, _) = send(
        client
            .get(format!("{}/api/files/download", srv.url))
            .bearer_auth(token)
            .query(&[("path", "oww-models")]),
    )
    .await;
    assert_eq!(status, 400);
    let (status, _, _, _) = send(
        client
            .get(format!("{}/api/files/download", srv.url))
            .bearer_auth(token)
            .query(&[("path", "missing.bin")]),
    )
    .await;
    assert_eq!(status, 404);
}
