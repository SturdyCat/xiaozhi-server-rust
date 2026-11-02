use super::super::*;
use crate::config::store::{sqlite::SqliteConfigStore, ConfigStore};
use crate::config::Config;
use axum::{extract::State, http::StatusCode, Json};
use serde_json::{json, Value};
use std::sync::Arc;

#[test]
fn workflow_partial_patch_replaces_bindings_atomically() {
    let mut base = Config::default();
    base.workflow = json!({"version":1,"bindings":[{"id":"old","plugin_id":"wakeup.greeting","position_id":"start"}]});
    let patch = json!({"workflow":{"version":1,"bindings":[{"id":"new","plugin_id":"soul.profile","position_id":"llm"}]}});
    let merged = merge_client_patch(&base, &patch).unwrap();
    assert_eq!(merged.workflow["bindings"].as_array().unwrap().len(), 1);
    assert_eq!(merged.workflow["bindings"][0]["id"], "new");
}

/// 带真实配置库（可持久化）的 Engines：`put_config` 需要 `config_path()` 非空才走得到 persist。
fn engines_with_store(tag: &str) -> Arc<crate::runtime::Engines> {
    let dir = std::env::temp_dir().join(format!("xz-put-{tag}-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let db = dir.join("data.db");
    let mut cfg = Config::default();
    cfg.memory.db_path = db.display().to_string();
    let store = SqliteConfigStore::open(&db.display().to_string()).unwrap();
    store.persist(&cfg).unwrap();
    crate::runtime::Engines::new_dev(&cfg, store).unwrap()
}

/// `workflow` 校验失败必须仍然是**可解析的 JSON body**。
///
/// 回归背景（QA 用 `git show HEAD` 对出来的）：把 `save_patch` 抽出去时，两种不同来源的
/// 400 被塞进同一个 `SaveError::Invalid`，handler 只能二选一 → `workflow` 这条从
/// `json_msg` 降级成纯文本。客户端 `ConfigFormActions.kt:203` 是
/// `data?.optString("message", "")`，纯文本取不到 → 界面从"哪个节点不合法"退化成
/// "保存失败: HTTP 400"。全仓只有一处产出这句文案，**没有任何测试锁响应体**，
/// 所以 1108 个用例全绿也发现不了。
///
/// 断言的是**响应体形状**而不是 `Content-Type`：`HEAD` 上这一步同样是
/// `(StatusCode::BAD_REQUEST, json_msg(..))`，axum 的 tuple 实现给的是
/// `text/plain; charset=utf-8` 而 body 是 JSON 文本；断言 JSON Content-Type 反而会
/// 与 `HEAD` 不一致（那就不是"PUT 语义一字不变"了）。
///
/// 把 handler 里 `WorkflowInvalid` 分支改回 `(StatusCode::BAD_REQUEST, detail)`，
/// 这条会立刻变红（`serde_json::from_slice` 失败）。
#[tokio::test]
async fn workflow_validation_failure_still_returns_a_json_body() {
    let engines = engines_with_store("workflow");
    let response = put_config(
        State(engines.clone()),
        Json(json!({
            "workflow": {"version": 1, "bindings": [
                {"id": "b1", "plugin_id": "no.such.plugin", "position_id": "start"}
            ]}
        })),
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let bytes = axum::body::to_bytes(response.into_body(), 8192)
        .await
        .unwrap();
    let body: Value = serde_json::from_slice(&bytes)
        .expect("workflow 校验失败的 body 必须是 JSON（纯文本会让客户端取不到 message）");
    assert_eq!(body["ok"], false, "{body}");
    assert_eq!(body["config_path"], Value::Null, "{body}");
    assert!(
        body["message"]
            .as_str()
            .unwrap_or_default()
            .contains("workflow 校验失败"),
        "message 必须带具体原因，实际 {body}"
    );
    // 前缀只许出现一次（handler 不许再拼一遍）。
    let message = body["message"].as_str().unwrap();
    assert_eq!(message.matches("workflow 校验失败").count(), 1, "{message}");
}
