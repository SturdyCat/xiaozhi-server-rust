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
    // 设备插件注册投影：管理台「设备管理」菜单的门控（见 [device_plugins_json]）
    body["device_plugins"] = device_plugins_json(&engines.config);
    // UI 模式是管理台隐藏/显示静态业务入口的唯一底座信号。纯底座即使打开了
    // 普通构建的旧 SQLite，也不能让客户端误把内置业务页当成已安装插件。
    body["ui"] = serde_json::json!({
        "mode": if cfg!(feature = "pure-base") { "pure-base" } else { "full" },
        "static_business": !cfg!(feature = "pure-base"),
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

/// 设备插件（echomuse / tuya）的**注册投影**——它们不是语音引擎（无 implementations），
/// 不进 `capabilities` 折叠，单独成数组随同一端点下发。
///
/// 注册语义 = 配置段启用（两段默认 `enabled=true`，即默认注册）：
/// - `registered=false`（用户显式关闭）→ 管理台隐藏「设备管理」里该插件的设备分区与
///   齿轮配置块；两个插件都未注册时整个「设备管理」菜单隐藏（注册后菜单才出现）；
/// - `phase` 只报 active/disabled 两态（纯配置投影，不做运行探测）；
/// - `note` 写热生效语义（echomuse RestartOnly / tuya 全热生效），与注册表同源结论。
///
/// ⚠️ 只读投影：不改变任何运行期行为（与本文件其余端点同一契约）。
// pub(super)：plugins_tests（兄弟模块）直驱纯函数做契约回归（与 list_plugins 同可见性口径）
pub(super) fn device_plugins_json(cfg: &crate::config::Config) -> serde_json::Value {
    // Keep the projection empty in pure-base, even for a config loaded from a
    // normal-build database. This prevents clients from discovering device UI.
    if cfg!(feature = "pure-base") {
        return serde_json::json!([]);
    }
    let echomuse = cfg.echomuse.enabled;
    let tuya = cfg.tuya.enabled;
    let row = |id: &str, label: &str, registered: bool, section: &str, note: &str| {
        serde_json::json!({
            "id": id,
            "label": label,
            "registered": registered,
            "phase": if registered { "active" } else { "disabled" },
            "config_section": section,
            "note": note,
            "last_error": "",
        })
    };
    serde_json::json!([
        row(
            "echomuse",
            "EchoMuse 设备",
            echomuse,
            "echomuse",
            "RestartOnly：开关保存后需重启服务端生效",
        ),
        row(
            "tuya",
            "涂鸦设备",
            tuya,
            "tuya",
            "全部热生效：保存即生效，无需重启"
        ),
    ])
}

/// `GET /api/config/schema`：字段元数据（按能力聚合）。
pub(super) async fn config_schema(State(_engines): State<Arc<Engines>>) -> Response {
    let mut body = registry::schema_json();
    if cfg!(feature = "pure-base") {
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
                    .into_response()
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

    /// `POST /api/plugins/upload`：上传**已构建**的 wasm 组件，验证→采用→立即生效。
    ///
    /// 分发模型（用户方向）：插件不编译进主系统、服务端零本地构建；本端点是唯一的
    /// 生产装载入口。body：`{"id":"<插件id>","wasm_base64":"...","activate":true}`。
    /// 流程：base64 解码 → 大小上限 → 隔离验证（manifest/api-version/注册包/self-test）
    /// →（activate）写盘（临时文件+原子改名）+ 热装载（先建新后替换）。
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
        let activate = body
            .get("activate")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        if id.is_empty() || b64.is_empty() {
            return (
                StatusCode::BAD_REQUEST,
                "需要 id 与 wasm_base64 字段（activate 默认 true）",
            )
                .into_response();
        }
        let bytes = match base64::engine::general_purpose::STANDARD.decode(b64) {
            Ok(b) => b,
            Err(e) => {
                return (
                    StatusCode::BAD_REQUEST,
                    format!("wasm_base64 解码失败：{e}"),
                )
                    .into_response()
            }
        };
        // 共享采用管线（raw 端点与本端点同一实现：验证 → 采用 → 持久化）
        adopt_bytes(&engines, &host, id, bytes, activate).await
    }

    /// `POST /api/plugins/upload/raw[?id=<id>]`：**raw octet-stream 直传**（管理台上传桥
    /// `uploadPickedFile` 的原生通道，token 走查询串）。id 缺省时从组件 manifest
    /// **探测**（"选文件即上传"）；显式 id 则严格匹配校验。其余语义与 JSON 端点一致：
    /// 隔离验证 → 采用 → 热装载 → 产物落盘；`[plugins]` 配置禁用的 id 明确拒绝。
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
                        .into_response()
                }
                Err(e) => {
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        format!("探测任务失败：{e}"),
                    )
                        .into_response()
                }
            },
        };
        adopt_bytes(&engines, host, id, body.to_vec(), true).await
    }

    /// 共享采用管线：大小上限 → `[plugins]` 配置门 → 隔离验证 → 采用（写盘+热装载）。
    async fn adopt_bytes(
        engines: &Arc<Engines>,
        host: &Arc<crate::plugins::wasm::WasmHost>,
        id: String,
        bytes: Vec<u8>,
        activate: bool,
    ) -> Response {
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
        if !engines.config.plugins.allows(&id) {
            return (
                StatusCode::FORBIDDEN,
                axum::Json(serde_json::json!({
                    "ok": false,
                    "error": format!("插件 {id} 被 [plugins] 配置禁用（先在配置里启用再上传）"),
                })),
            )
                .into_response();
        }
        // 隔离验证（不落表、不影响现有实例）；self-test 失败 → 400 带完整结果
        let id_for_check = id.clone();
        let validation = match tokio::task::spawn_blocking({
            let bytes = bytes.clone();
            let host = host.clone();
            move || host.validate(&id_for_check, &bytes)
        })
        .await
        {
            Ok(Ok(results)) => results,
            Ok(Err(e)) => {
                return (
                    StatusCode::BAD_REQUEST,
                    axum::Json(serde_json::json!({
                        "ok": false, "activated": false,
                        "error": format!("隔离验证失败（未采用）：{e:#}")
                    })),
                )
                    .into_response()
            }
            Err(e) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("验证任务失败：{e}"),
                )
                    .into_response()
            }
        };

        let mut resp = serde_json::json!({
            "ok": true,
            "id": id,
            "validation": validation,
            "activated": false,
        });
        if !activate {
            return json_ok(resp);
        }
        // 采用：写盘（持久意图）+ 热装载（先建新后替换；失败保留旧实例）
        let id_for_load = id.clone();
        let load = match tokio::task::spawn_blocking({
            let bytes = bytes.clone();
            let host = host.clone();
            move || host.load(&id_for_load, &bytes)
        })
        .await
        {
            Ok(r) => r,
            Err(e) => Err(anyhow::anyhow!("装载任务失败：{e}")),
        };
        match load {
            Ok(()) => {
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
                resp["activated"] = Value::Bool(true);
                resp["sha256"] = Value::String(digest);
                resp["persisted"] = Value::Bool(persisted);
                resp["extensions"] = engines
                    .wasm
                    .as_ref()
                    .map(|h| h.snapshot_json()["extensions"].clone())
                    .unwrap_or(Value::Null);
                json_ok(resp)
            }
            Err(e) => (
                StatusCode::BAD_REQUEST,
                axum::Json(serde_json::json!({
                    "ok": false, "activated": false,
                    "validation": validation,
                    "error": format!("装载失败（未采用，旧实例保留）：{e:#}")
                })),
            )
                .into_response(),
        }
    }

    /// `POST /api/plugins/{id}/unload`：卸载（卸注册表 + 回收 kv 命名空间 + 删除产物文件）。
    pub(crate) async fn unload_plugin(
        State(engines): State<Arc<Engines>>,
        Path(plugin_id): Path<String>,
    ) -> Response {
        let Some(host) = engines.wasm.as_ref() else {
            return unavailable();
        };
        let removed = host.unload(&plugin_id);
        let file_removed = engines
            .wasm_artifact_dir()
            .map(|d| d.remove(&plugin_id))
            .unwrap_or(false);
        if let Err(e) = engines.clear_wasm_artifact_pin(&plugin_id) {
            tracing::warn!(plugin = %plugin_id, "插件摘要钉版清理失败：{e:#}");
        }
        if !removed && !file_removed {
            return (
                StatusCode::NOT_FOUND,
                format!("插件 {plugin_id} 未装载且无产物文件"),
            )
                .into_response();
        }
        json_ok(serde_json::json!({
            "ok": true, "id": plugin_id,
            "unloaded": removed, "artifact_removed": file_removed,
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
}

#[cfg(feature = "wasm-plugins")]
pub(super) use wasm_api::{
    plugin_action, plugin_ui, set_extension_preference, unload_plugin, upload_plugin,
    upload_plugin_raw,
};
