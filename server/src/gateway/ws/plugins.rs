//! 能力清单与配置元数据端点（`docs/plugin-architecture-unification.md` §3.4）。
//!
//! - `GET /api/plugins`：能力 + 实现 + **实际状态**（`enabled` 与 `phase` 分离，
//!   并带 `last_error`）。此前管理页只能靠"保存后能不能用"猜后端，运行期失败全部静默。
//!   feature `wasm-plugins` 编入时追加 `wasm` 键（动态插件清单 + 扩展点解析投影，
//!   P2 起与静态能力清单合并为同一张清单）。
//! - `GET /api/config/schema`：字段元数据（标签/类型/默认值提示/热生效语义/是否密钥）。
//! - `POST /api/plugins/{id}/actions/{name}`：**统一动作端点**（管理台按钮与语音工具
//!   共用同一实现——"一次操作、双调用方"）。v1 只路由 wasm 插件；静态能力的动作
//!   仍走技能系统（MCP/plan），不在这里做第二套分发。
//! - `POST /api/plugins/extensions/{point}/preference`：扩展点首选实现（排他解析表；
//!   v1 进程内态，P3 接配置管线持久化）。
//! - `GET /api/plugins/{id}/ui`：单个已注册插件的 JSON UI 页面只读投影（A2UI 最小
//!   闭环：配置/测试页由插件 registration.ui 声明，宿主透传；未装载 404——未注册
//!   插件不返回业务 UI）。
//!
//! ⚠️ 只读/受控投影：不改变静态能力运行期行为，不做自动表单布局。

use std::sync::Arc;

use axum::{extract::State, response::Response};

#[cfg(test)]
mod tests;

use super::json_ok;
use crate::gateway::ws::config::config_is_persistent;
use crate::plugins::registry;
use crate::runtime::Engines;

/// `GET /api/plugins`：能力清单 + 状态（+ wasm 动态插件投影）。
pub(super) async fn list_plugins(State(engines): State<Arc<Engines>>) -> Response {
    let mut body = engines.host.capabilities_json();
    body["ok"] = serde_json::json!(true);
    // UI 只反映通用底座；业务菜单必须来自已装载的 WASM manifest。
    body["ui"] = serde_json::json!({
        "mode": "base",
        "static_business": false,
    });
    // 配置来源一并带上：状态解读常需知道"这些值来自哪个文件、重启会不会丢"
    let config_path = engines.config_path();
    body["config"] = serde_json::json!({
        "path": config_path,
        "persistent": config_path.map(config_is_persistent),
    });
    // P2：动态插件与静态能力合并为同一张清单（追加键，旧客户端忽略未知键）
    #[cfg(feature = "wasm-plugins")]
    if let Some(host) = engines.wasm.as_ref() {
        body["wasm"] = host.snapshot_json();
    }
    json_ok(body)
}

/// `GET /api/config/schema`：字段元数据（按能力聚合）。
pub(super) async fn config_schema(State(_engines): State<Arc<Engines>>) -> Response {
    let mut body = registry::schema_json();
    if cfg!(not(feature = "native-plugins")) {
        // 静态 schema 不是插件注册：纯底座只把核心数据面字段交给管理台，
        // 业务插件字段必须来自 WASM manifest/config_fields。
        if let Some(caps) = body["capabilities"].as_array_mut() {
            caps.retain(|cap| {
                matches!(
                    cap.get("id").and_then(serde_json::Value::as_str),
                    Some("asr") | Some("vad") | Some("tts") | Some("llm")
                )
            });
        }
    }
    body["ok"] = serde_json::json!(true);
    json_ok(body)
}

// ============================================================
// wasm 插件端点（feature 门控；未编入 = 路由不存在，客户端隐藏插件 UI）
// ============================================================

#[cfg(feature = "wasm-plugins")]
mod wasm_api {
    use std::sync::Arc;

    use axum::extract::{Path, State};
    use axum::http::StatusCode;
    use axum::response::IntoResponse;
    use axum::response::Response;
    use serde_json::Value;

    use crate::runtime::Engines;

    use super::json_ok;

    fn unavailable() -> Response {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            "WASM 插件平台不可用（初始化失败已降级关闭，见服务日志）",
        )
            .into_response()
    }

    /// `GET /api/plugins/{id}/ui`：单个已注册插件的 JSON UI 页面只读投影
    /// （配置/测试页等，由插件 registration.ui 声明，宿主透传不解释）。
    /// 未装载的插件 404——**未注册插件不返回业务 UI** 的唯一守门点（A2UI 消费方
    /// 以此为契约：只渲染 200 响应里的页面）。
    pub(crate) async fn plugin_ui(
        State(engines): State<Arc<Engines>>,
        Path(plugin_id): Path<String>,
    ) -> Response {
        let Some(host) = engines.wasm.as_ref() else {
            return unavailable();
        };
        // wasm 注册表快照持锁；投影是纯内存读，不必 spawn_blocking
        match host.plugin_ui_json(&plugin_id) {
            Some(mut body) => {
                body["ok"] = serde_json::json!(true);
                json_ok(body)
            }
            None => (
                StatusCode::NOT_FOUND,
                axum::Json(serde_json::json!({
                    "ok": false,
                    "error": format!("插件 {plugin_id} 未装载（未注册插件不返回业务 UI）"),
                })),
            )
                .into_response(),
        }
    }

    /// `POST /api/plugins/{id}/actions/{name}`：统一动作（白名单在宿主内：未声明动作拒绝）。
    /// 请求体可为任意 JSON（原样作为 payload 传给插件）；空体传空字符串。
    pub(crate) async fn plugin_action(
        State(engines): State<Arc<Engines>>,
        Path((plugin_id, action_id)): Path<(String, String)>,
        body: Option<axum::Json<Value>>,
    ) -> Response {
        // spawn_blocking 闭包要 'static：克隆 Arc 宿主而非借用 engines
        let Some(host) = engines.wasm.clone() else {
            return unavailable();
        };
        let payload = match body {
            Some(axum::Json(v)) => v.to_string(),
            None => String::new(),
        };
        // wasm 调用全部 spawn_blocking（仓库纪律；动作预算 5s 档由沙箱 epoch 兜底）
        let result = match tokio::task::spawn_blocking(move || {
            host.handle_action(&plugin_id, &action_id, &payload)
        })
        .await
        {
            Ok(joined) => joined,
            Err(e) => Err(anyhow::anyhow!("动作任务失败：{e}")),
        };
        match result {
            Ok(result) => {
                // 动作返回统一按 JSON 字符串透传；解析失败原样包一层
                let value = serde_json::from_str::<Value>(&result)
                    .unwrap_or_else(|_| Value::String(result.clone()));
                let mut body = serde_json::json!({ "ok": true, "result": value });
                body["raw"] = Value::String(result);
                json_ok(body)
            }
            Err(e) => (
                StatusCode::BAD_REQUEST,
                axum::Json(serde_json::json!({ "ok": false, "error": format!("{e:#}") })),
            )
                .into_response(),
        }
    }

    /// `POST /api/plugins/extensions/{point}/preference`：设置扩展点首选实现。
    /// `{"implementation": "<plugin-id>" | null}`（null = 回官方默认解析）。
    /// 进程内态；P3 接配置管线后持久化并热装载目标插件。
    pub(crate) async fn set_extension_preference(
        State(engines): State<Arc<Engines>>,
        Path(point): Path<String>,
        axum::Json(body): axum::Json<Value>,
    ) -> Response {
        let Some(host) = engines.wasm.as_ref() else {
            return unavailable();
        };
        let implementation = match body.get("implementation") {
            None | Some(Value::Null) => None,
            Some(Value::String(s)) if !s.is_empty() => Some(s.clone()),
            Some(_) => {
                return (
                    StatusCode::BAD_REQUEST,
                    "implementation 须为插件 id 字符串或 null",
                )
                    .into_response();
            }
        };
        host.set_extension_preference(&point, implementation);
        // 回当前解析结果，客户端保存后立即确认生效形态
        json_ok(serde_json::json!({
            "ok": true,
            "point": point,
            "resolution": resolution_json(&host.resolve(&point)),
        }))
    }

    /// `POST /api/plugins/upload`：上传**已构建**的 wasm 组件（生命周期版：**只登记
    /// Staged，不装载**；注册/激活走 `register`/`activate` 端点）。
    ///
    /// 分发模型（用户方向）：插件不编译进主系统、服务端零本地构建。body：
    /// `{"id":"<插件id>","wasm_base64":"..."}`。流程：base64 解码 → 大小上限 →
    /// 平台总闸 → 探测 manifest + 版本升级判定（同 id：新>旧才接受）→ 落盘
    /// （临时文件+原子改名）+ 钉版 → 登记 Staged；原 Active 条目随后原地换版本。
    pub(crate) async fn upload_plugin(
        State(engines): State<Arc<Engines>>,
        axum::Json(body): axum::Json<Value>,
    ) -> Response {
        use base64::Engine as _;

        let Some(host) = engines.wasm.clone() else {
            return unavailable();
        };
        // 立即转 owned：spawn_blocking 闭包要求 'static，不能借用 body
        let id = body
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let b64 = body
            .get("wasm_base64")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        if id.is_empty() || b64.is_empty() {
            return (StatusCode::BAD_REQUEST, "需要 id 与 wasm_base64 字段").into_response();
        }
        let bytes = match base64::engine::general_purpose::STANDARD.decode(b64) {
            Ok(b) => b,
            Err(e) => {
                return (
                    StatusCode::BAD_REQUEST,
                    format!("wasm_base64 解码失败：{e}"),
                )
                    .into_response();
            }
        };
        // 共享采用管线（raw 端点与本端点同一实现）
        adopt_bytes(&engines, &host, id, bytes).await
    }

    /// `POST /api/plugins/upload/raw[?id=<id>]`：**raw octet-stream 直传**（管理台上传桥
    /// `uploadPickedFile` 的原生通道，token 走查询串）。id 缺省时从组件 manifest
    /// **探测**（"选文件即上传"）；显式 id 则严格匹配校验。其余语义与 JSON 端点一致：
    /// 登记 Staged（不装载）；`[plugins]` 平台总闸关闭时明确拒绝。
    pub(crate) async fn upload_plugin_raw(
        State(engines): State<Arc<Engines>>,
        axum::extract::Query(query): axum::extract::Query<
            std::collections::HashMap<String, String>,
        >,
        body: axum::body::Bytes,
    ) -> Response {
        let Some(host) = engines.wasm.as_ref() else {
            return unavailable();
        };
        if body.is_empty() {
            return (
                StatusCode::BAD_REQUEST,
                axum::Json(
                    serde_json::json!({"ok": false, "error": "请求体为空（应为 wasm 组件字节）"}),
                ),
            )
                .into_response();
        }
        // id 缺省 → 从组件 manifest 探测（id 在组件里，文件名不可靠）；显式 id = 严格匹配校验
        let id = match query.get("id").filter(|s| !s.is_empty()) {
            Some(id) => id.clone(),
            None => match tokio::task::spawn_blocking({
                let host = host.clone();
                let bytes = body.clone();
                move || host.probe_id(&bytes)
            })
            .await
            {
                Ok(Ok(id)) => id,
                Ok(Err(e)) => {
                    return (
                        StatusCode::BAD_REQUEST,
                        axum::Json(serde_json::json!({
                            "ok": false,
                            "error": format!("id 探测失败：{e:#}"),
                        })),
                    )
                        .into_response();
                }
                Err(e) => {
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        format!("探测任务失败：{e}"),
                    )
                        .into_response();
                }
            },
        };
        adopt_bytes(&engines, host, id, body.to_vec()).await
    }

    /// 共享采用管线（生命周期版）：大小上限 → 平台总闸 → 探测 manifest + 版本升级
    /// 判定（同 id：新>旧才接受，保持原生命周期）→ 落盘+钉版 → 登记 Staged；
    /// 原 Active 条目随后原地换版本（注册包刷新）。**不再热装载新插件**。
    pub(super) async fn adopt_bytes(
        engines: &Arc<Engines>,
        host: &Arc<crate::plugins::wasm::WasmHost>,
        id: String,
        bytes: Vec<u8>,
    ) -> Response {
        use crate::plugins::wasm::Lifecycle;

        if bytes.len() > crate::plugins::wasm::artifacts::MAX_ARTIFACT_BYTES {
            return (
                StatusCode::PAYLOAD_TOO_LARGE,
                format!(
                    "产物超上限（{} 字节 > {}）",
                    bytes.len(),
                    crate::plugins::wasm::artifacts::MAX_ARTIFACT_BYTES
                ),
            )
                .into_response();
        }
        // 平台总闸（一票否决）；每插件 enabled 是激活意图（activate/deactivate 端点管），
        // 不再拦上传——禁用插件允许升级产物（重启后仍按激活意图决定是否运行）。
        if !engines.config.plugins.enabled {
            return (
                StatusCode::FORBIDDEN,
                axum::Json(serde_json::json!({
                    "ok": false,
                    "error": "WASM 插件平台已被 [plugins].enabled 总闸关闭",
                })),
            )
                .into_response();
        }
        // 探测 manifest + 版本升级判定（Staged：不注册、不装载、不跑 self-test）
        let staged = match tokio::task::spawn_blocking({
            let bytes = bytes.clone();
            let host = host.clone();
            let id = id.clone();
            move || host.stage(&id, &bytes)
        })
        .await
        {
            Ok(Ok(outcome)) => outcome,
            Ok(Err(crate::plugins::wasm::StageError::VersionNotNewer { new, current })) => {
                return (
                    StatusCode::CONFLICT,
                    axum::Json(serde_json::json!({
                        "ok": false, "staged": false,
                        "error": format!("版本不升级：新版本 {new} ≤ 现有 {current}（升级须更高版本）"),
                        "current_version": current,
                    })),
                )
                    .into_response()
            }
            Ok(Err(e)) => {
                return (
                    StatusCode::BAD_REQUEST,
                    axum::Json(serde_json::json!({
                        "ok": false, "staged": false,
                        "error": format!("登记失败（未采用）：{e}"),
                    })),
                )
                    .into_response()
            }
            Err(e) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("登记任务失败：{e}"),
                )
                    .into_response()
            }
        };

        // 采用：落盘（持久意图）+ 摘要钉版
        let digest = crate::plugins::wasm::artifacts::ArtifactDir::digest(&bytes);
        let artifact_saved = engines
            .wasm_artifact_dir()
            .map(|d| d.save(&id, &bytes).is_ok())
            .unwrap_or(false);
        let pin_saved = if artifact_saved {
            match engines.pin_wasm_artifact(&id, &digest) {
                Ok(()) => true,
                Err(e) => {
                    tracing::warn!(plugin = %id, "产物摘要钉版写入配置库失败：{e:#}");
                    false
                }
            }
        } else {
            false
        };
        let persisted = artifact_saved && pin_saved;
        if !persisted {
            tracing::warn!(
                plugin = %id,
                artifact_saved,
                pin_saved,
                "产物持久化不完整（本次运行有效，重启后可能跳过或丢失）"
            );
        }

        // 原 Active 条目升级：原地换版本（注册包刷新；失败保留旧实例并回显原因）
        let mut reload_error: Option<String> = None;
        if staged.preserved == Some(Lifecycle::Active) {
            match tokio::task::spawn_blocking({
                let bytes = bytes.clone();
                let host = host.clone();
                let id = id.clone();
                move || host.reload_active(&id, &bytes)
            })
            .await
            {
                Ok(Ok(())) => {}
                Ok(Err(e)) => {
                    reload_error = Some(format!("{e:#}"));
                    tracing::warn!(plugin = %id, "Active 升级原地换版本失败（旧实例保留）：{e:#}");
                }
                Err(e) => {
                    reload_error = Some(format!("换版本任务失败：{e}"));
                }
            }
        }
        let lifecycle = host
            .lifecycle_of(&id)
            .map(|(lc, _)| lc.as_str().to_string())
            .unwrap_or_else(|| "staged".to_string());
        json_ok(serde_json::json!({
            "ok": true,
            "id": id,
            "staged": true,
            "lifecycle": lifecycle,
            "version": staged.version,
            "upgraded": staged.upgraded,
            "previous_version": staged.previous_version,
            "sha256": digest,
            "persisted": persisted,
            "reload_error": reload_error,
            "extensions": engines
                .wasm
                .as_ref()
                .map(|h| h.snapshot_json()["extensions"].clone())
                .unwrap_or(Value::Null),
        }))
    }

    /// `POST /api/plugins/{id}/unload`：**兼容别名** = deactivate + remove（生命周期版）。
    /// Active 插件先停用（有 Active 依赖者 → 409），再移除条目/产物/标记/kv；
    /// 响应注明新语义。客户端应改用 `deactivate`/`remove` 端点。
    pub(crate) async fn unload_plugin(
        State(engines): State<Arc<Engines>>,
        Path(plugin_id): Path<String>,
    ) -> Response {
        use crate::plugins::wasm::{DeactivateError, RemoveError};

        let Some(host) = engines.wasm.as_ref() else {
            return unavailable();
        };
        // ① Active → 先停用（依赖者检查）；Registered/Staged 无需停用
        if let Err(e) = host.deactivate(&plugin_id) {
            if let DeactivateError::Dependents(list) = e {
                return (
                    StatusCode::CONFLICT,
                    axum::Json(serde_json::json!({
                        "ok": false,
                        "error": format!("存在 Active 依赖者，先停用：{}", list.join("、")),
                        "dependents": list,
                    })),
                )
                    .into_response();
            }
        }
        // ② 移除（条目 + kv）+ 磁盘回收（产物 + 注册标记 + 摘要钉版）
        let entry_removed = match host.remove(&plugin_id) {
            Ok(()) => true,
            Err(RemoveError::Active) => {
                return (
                    StatusCode::CONFLICT,
                    axum::Json(serde_json::json!({
                        "ok": false,
                        "error": "插件仍在运行（先停用再移除）",
                    })),
                )
                    .into_response();
            }
            Err(RemoveError::NotFound) => false,
        };
        let artifact_removed = engines
            .wasm_artifact_dir()
            .map(|d| d.remove(&plugin_id))
            .unwrap_or(false);
        let marker_removed = engines
            .wasm_artifact_dir()
            .map(|d| d.remove_registered_marker(&plugin_id))
            .unwrap_or(false);
        if let Err(e) = engines.clear_wasm_artifact_pin(&plugin_id) {
            tracing::warn!(plugin = %plugin_id, "插件摘要钉版清理失败：{e:#}");
        }
        if !entry_removed && !artifact_removed {
            return (
                StatusCode::NOT_FOUND,
                format!("插件 {plugin_id} 未登记且无产物文件"),
            )
                .into_response();
        }
        json_ok(serde_json::json!({
            "ok": true, "id": plugin_id,
            "removed": entry_removed || artifact_removed,
            "entry_removed": entry_removed,
            "artifact_removed": artifact_removed,
            "marker_removed": marker_removed,
            "alias": "deactivate+remove（生命周期版；请改用 deactivate/remove 端点）",
        }))
    }

    fn resolution_json(r: &crate::plugins::wasm::Resolution) -> Value {
        use crate::plugins::wasm::Resolution;
        match r {
            Resolution::Resolved(id) => {
                serde_json::json!({"state": "resolved", "implementation": id})
            }
            Resolution::Fallback {
                official,
                preferred,
                reason,
            } => serde_json::json!({
                "state": "fallback", "implementation": official,
                "preferred": preferred, "reason": reason,
            }),
            Resolution::Unresolved(reason) => {
                serde_json::json!({"state": "unresolved", "reason": reason})
            }
        }
    }

    /// 生命周期端点（register / activate / deactivate / remove）：见子模块文档。
    pub(super) mod lifecycle;
}

#[cfg(feature = "wasm-plugins")]
pub(super) use wasm_api::{
    lifecycle::{activate_plugin, deactivate_plugin, register_plugin, remove_plugin},
    plugin_action, plugin_ui, set_extension_preference, unload_plugin, upload_plugin,
    upload_plugin_raw,
};
