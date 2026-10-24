//! WebSocket 网关：HTTP 路由、握手与协商、鉴权，然后转入会话状态机
//! （[`crate::workflow::session::run_session`]）。
//!
//! ## 路由
//! - `GET /api/health`：健康检查，返回 JSON（status/name/version）。
//! - `GET /api/health/engines`：引擎健康自检（P1-6）——11 个 capability 的能力级
//!   聚合 + 运行期本地探测（模型文件存在 / 记忆目录可写），与 `/api/plugins` 同源。
//! - `GET /api/ws`：WebSocket 会话入口（先 [`auth_ok`] 鉴权，再 [`handle_handshake`] 协商）。
//! - `GET /api/admin/ws`：管理台实时推送通道（双向 WS：hello + echomuse_devices_changed
//!   + ping/pong/error，见 [`admin_ws`]）。
//! - `GET/PUT/POST /api/config`：管理页面读写当前配置（语义见下）。
//! - `GET /api/plugins`：能力清单 + 实际状态（`enabled`/`phase`/`last_error`）。
//! - `GET /api/config/schema`：各能力/实现的字段元数据（类型/默认值提示/热生效语义/密钥标记）。
//! - `GET /api/usage` / `GET /api/session/{id}/usage`：令牌用量（成本与上下文膨胀观测）。
//! - `GET /api/devices` / `GET /api/devices/{id}`：设备清单与单台详情（在线态、最后活跃、
//!   协商参数、板型/固件版本）——数据源 [`registry`]（进程内注册表）。
//! - `POST /api/devices/{id}/inject`：向该设备的活跃会话注入一段文本，作为用户话语走
//!   完整 LLM→TTS→下行管线（不经 ASR 的联调利器，见 `app/inject`）。
//! - `GET /api/sessions`：**此刻**活着的连接及其阶段（聆听/思考/说话）。
//! - `GET /api/conversations`：内存里保留的最近对话（环形，非完整历史；可按设备过滤）。
//! - `GET /api/stats`：看板聚合（在线数 / 轮次 / 令牌 / 能力状态徽标）。
//! - `GET /api/metrics`：延迟/事件聚合大盘（整轮延迟 p50/p95、会话、TTS 下行、异常；
//!   数据源 [`metrics`]——事件总线常驻订阅者写入的内存态，重启清零）。
//! - `GET /api/mcp/tools`：按设备列出会话开始时通过 MCP 探测到的工具清单
//!   （"这台 ESP 支持哪些工具、每个能干什么"——数据源见 [`mcp`]）。
//! - `GET /api/memory/status` + `POST /api/memory/{recall,maintain,clear}`：图记忆的状态、
//!   当场试召回（"记忆到底有没有在工作"的反馈）、立即维护与清空。
//! - `GET/POST /api/ota(/)`：小智固件 OTA 引导（返回 websocket 接入信息 + 校时 + 版本，
//!   契约见 [`ota`]；固件侧把 OTA_URL 指到本服务即自动接入，无需硬编码 WS 地址）。
//! - 其余路径：静态托管 `[server].admin_dir`（h5App 构建产物，含 `index.html`）；
//!   目录不存在时 [`admin_missing`] 返回友好提示而非崩溃；SPA 未知路径回退 `index.html`。
//!
//! ## 鉴权
//! `[server].expected_token` 为空则不校验；非空时设备走 `Authorization: Bearer <token>`，
//! 浏览器 WebSocket 无法自定义请求头，额外允许 `?token=<token>` 查询参数兜底
//! （管理页面通过 URL 带 token 联调；设备侧仍走 Authorization 头）。见 [`auth_ok`]。
//!
//! ## `/api/config` 读写语义
//! - `GET`：若启动指定了配置文件则实时读盘，否则返回内存中的内置默认配置（/data/models 生产路径）。
//! - `PUT`/`POST`：**部分更新**——只覆盖请求中出现的字段，未出现的保持库内现值
//!   （显式 `null` = 清除该字段，回落 serde 默认值）；写回 SQLite 配置库
//!   （`data.db`）。配置库不可用（`location()` 为 `None`）时返回 400。
//! - 生效时机：TTS 引擎参数在**新会话**开始时按盘上配置热切换
//!   （[`crate::runtime::Engines::refresh_from_disk`]，经 `spawn_blocking`）；
//!   ASR 模型、监听地址、token、管理页目录等仍需重启 server。

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use axum::{
    extract::{ws::WebSocket, ws::WebSocketUpgrade, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{delete, get},
    Router,
};
use tower_http::services::{ServeDir, ServeFile};
use uuid::Uuid;

use crate::config::ServerConfig;
use crate::runtime::Engines;
use crate::workflow::protocol::{
    AudioParams, BinVersion, ClientHello, ClientMessage, ServerMessage,
};
use crate::workflow::protocol::{IncomingFrame, Transport, WsTransport};
use crate::workflow::session::{run_session, send_text, SessionParams};

// 管理台实时推送通道（`GET /api/admin/ws`）：hello + echomuse_devices_changed +
// ping/pong/error 的双向 WS，替代已删除的 /api/echomuse/devices/wait 长挂轮询。
pub(crate) mod admin_ws;
#[cfg(test)]
mod admin_ws_tests;
mod bench_api;
mod config;
mod health;
// 服务器全局实时日志流（`GET /api/logs` WS）：tracing 事件 → 环形缓冲 + 广播。
// pub(crate)：main.rs 在初始化 tracing 时挂载采集层 layer()。
pub(crate) mod log_stream;
mod mcp;
mod memory;
mod nlu_build;
// 延迟/事件聚合（P1-1）：只读投影，数据源 observability::metrics（事件总线订阅者）
mod metrics;
mod ota;
mod plugins;
mod registry;
mod soul;
mod usage;
use config::device_overlay::{device_overlay_delete, device_overlay_get, device_overlay_put};
use config::presets::{preset_apply, preset_delete, preset_save, presets_list};
use config::{config_meta, db_backup, db_query, get_config, put_config};
use health::engines_health;
use mcp::mcp_tools;
use memory::{memory_clear, memory_maintain, memory_rag_reindex, memory_recall, memory_status};
use metrics::metrics_snapshot;
use nlu_build::{build_log_ws, build_status, start_build};
// 插件维护
#[cfg(test)]
pub(crate) use bench_api::service_router as test_bench_router;
pub(crate) use config::{config_is_persistent, config_revision};
use ota::ota;
use plugins::{config_schema, list_plugins};
use registry::{conversations, device_detail, device_inject, devices, sessions, stats};
use soul::{soul_presets, soul_preview};
use usage::{session_usage, usage};

/// 统一的 JSON 响应（全部 `/api` 端点共用）：
/// `Content-Type: application/json; charset=utf-8` + 指定状态码。
pub(super) fn json_response(code: StatusCode, body: serde_json::Value) -> Response {
    match serde_json::to_string(&body) {
        Ok(json) => (
            code,
            [(header::CONTENT_TYPE, "application/json; charset=utf-8")],
            json,
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("序列化失败: {e}"),
        )
            .into_response(),
    }
}

/// [`json_response`] 的单参便捷包装（`StatusCode::OK`）。
pub(super) fn json_ok(body: serde_json::Value) -> Response {
    json_response(StatusCode::OK, body)
}

/// 构造 Axum 路由：API（健康检查 / WebSocket）之外，其余路径静态托管
/// `config.server.admin_dir` 指向的管理页面（h5App 构建产物，含 index.html）。
/// 该目录不存在时，`/` 及以下返回友好提示而非崩溃。
pub fn router(engines: Arc<Engines>) -> Router {
    let admin_dir = engines.config.server.admin_dir.clone();

    let api = Router::new()
        .route("/api/health", get(health))
        .route("/api/ws", get(ws_handler))
        // 管理台实时推送通道（`/api/ws` 已被小智设备协议占用，管理台走独立路径）
        .route("/api/admin/ws", get(admin_ws::admin_ws_handler))
        .route(
            "/api/config",
            get(get_config).put(put_config).post(put_config),
        )
        // 引擎健康自检（P1-6）：11 个 capability × 运行期本地探测的聚合投影，
        // 能力聚合与 /api/plugins 同源（capability_summary），phase 含探测合并
        .route("/api/health/engines", get(engines_health))
        // 配置来源元信息（macApp 显示"配置文件路径 + 是否持久化"，排查配置丢失去向）
        .route("/api/config/meta", get(config_meta))
        // 配置预设（P1-5）：保存当前 / 列出 / 应用 / 删除。
        // `{name}/delete` 的 POST 别名：Kuikly NetworkModule 只支持 GET/POST（无 DELETE），
        // 与 OTA 固件删除的 `POST /api/ota/firmware/delete` 同一惯例。
        .route("/api/config/presets", get(presets_list).post(preset_save))
        .route("/api/config/presets/{name}", delete(preset_delete))
        .route(
            "/api/config/presets/{name}/apply",
            axum::routing::post(preset_apply),
        )
        .route(
            "/api/config/presets/{name}/delete",
            axum::routing::post(preset_delete),
        )
        // 能力清单 + 状态（enabled/phase/last_error）与字段元数据（配置页据此校验/提示）
        .route("/api/plugins", get(list_plugins))
        .route("/api/config/schema", get(config_schema))
        // 远程客户端访问 SQLite 配置库：只读 SQL 查询（需 token）+ 一致性备份（VACUUM INTO）
        .route("/api/config/db/query", axum::routing::post(db_query))
        .route("/api/config/db/backup", axum::routing::post(db_backup))
        // Unified snapshot-isolated node diagnostics; legacy paths retained and admin-authenticated.
        .merge(crate::gateway::ws::bench_api::routes(&engines))
        // 记忆：状态 / 测试召回 / 立即维护 / 清空（"记忆开了没用"的当场验证入口）
        .route("/api/memory/status", get(memory_status))
        .route("/api/memory/recall", axum::routing::post(memory_recall))
        .route("/api/memory/maintain", axum::routing::post(memory_maintain))
        .route("/api/memory/clear", axum::routing::post(memory_clear))
        // P1-9：RAG 文档索引后台重建（仅 engine = rag 有效）
        .route(
            "/api/memory/rag/reindex",
            axum::routing::post(memory_rag_reindex),
        )
        // 灵魂：内置预设清单 / 最终提示词预览（"人格配了之后到底发了什么"的当场验证入口）
        .route("/api/soul/presets", get(soul_presets))
        .route("/api/soul/preview", axum::routing::post(soul_preview))
        // 令牌用量：全局 + 单会话（回答"成本"与"上下文膨胀"）
        .route("/api/usage", get(usage))
        .route("/api/session/{id}/usage", get(session_usage))
        // 设备 / 会话 / 对话记录 / 看板：数据源是进程内注册表（`app/registry`），
        // 全是**只读投影**（`GET /api/devices`、`/api/sessions`、`/api/conversations`、`/api/stats`）
        .route("/api/devices", get(devices))
        .route("/api/devices/{device_id}", get(device_detail))
        // 设备消息注入（P1-2）：文本作为用户话语走完整管线（会话侧通道见 app/inject）
        .route(
            "/api/devices/{device_id}/inject",
            axum::routing::post(device_inject),
        )
        // 每设备 overlay（P1-5）：白名单字段增量；仅会话内参数可覆盖，结构型参数拒绝。
        // POST = PUT 的别名、`/delete` 的 POST = DELETE 的别名（Kuikly 客户端无 PUT/DELETE，
        // 同 OTA 固件删除惯例）。
        .route(
            "/api/devices/{device_id}/overlay",
            get(device_overlay_get)
                .put(device_overlay_put)
                .post(device_overlay_put)
                .delete(device_overlay_delete),
        )
        .route(
            "/api/devices/{device_id}/overlay/delete",
            axum::routing::post(device_overlay_delete),
        )
        .route("/api/sessions", get(sessions))
        .route("/api/conversations", get(conversations))
        .route("/api/stats", get(stats))
        // 延迟/事件聚合大盘（P1-1）：整轮延迟 p50/p95、会话、TTS、异常（内存态，重启清零）
        .route("/api/metrics", get(metrics_snapshot))
        // 设备 MCP 工具：按设备列出会话开始时探测到的工具清单（回答"这台设备能被 AI 指挥做什么"）
        .route("/api/mcp/tools", get(mcp_tools))
        .route("/api/nlu/model/build", get(build_status).post(start_build))
        // 单次训练的聚焦日志流（全局日志流见 /api/logs）：终态帧后服务端主动关闭。
        .route("/api/nlu/model/build/log", get(build_log_ws))
        // 服务器全局实时日志流：macApp 连上即可实时看到全部运行日志；
        // AI/脚本客户端也可连它，一边调 /api 测试端点一边读日志观察效果。
        .route("/api/logs", get(log_stream::logs_ws))
        // 小智固件的 OTA 引导端点（带/不带尾斜杠都注册：固件配置的 URL 以 / 结尾）。
        // ⚠️ 所有接口必须在 /api 之下（接口规范），固件 OTA_URL 指到 /api/ota/ 即可——
        // 该地址完全可配置，无需对齐官方服务器的 /xiaozhi/ota/ 路径。
        .route("/api/ota", get(ota).post(ota))
        .route("/api/ota/", get(ota).post(ota))
        // 固件托管（上传/下载/当前版本查询）+ 发音人目录，并入同一 /api 前缀
        .merge(crate::device::firmware::router())
        .merge(crate::device::voices::router())
        // 服务端数据目录文件管理（/api/files/*）：管理台文件页数据源 + EchoMuse
        // 自定义唤醒词模型上传通道（oww-models/）；通用存储管理，不受 [echomuse] 开关影响
        .merge(crate::gateway::files::router())
        // EchoMuse 设备入口（Echo Dot 2 / emOS）：`[echomuse].enabled=true` 才生效
        // （路由内部自检开关，关闭时返回 404；挂载本身零行为变化）。
        .merge(crate::device::echomuse::routes::em_routes())
        // EchoMuse 管理面（/api/echomuse/*，经 auth_ok 鉴权）：registry 全表 + 认领 + 下行指令
        .merge(crate::device::echomuse::admin::admin_routes())
        // EchoMuse 蓝牙代理管理面（/api/echomuse/devices/{id}/ble/*，同 gate 鉴权）：
        // 广播采集查询 + GATT 直驱；`[echomuse].ble_proxy=false` 时统一 404。
        .merge(crate::device::echomuse::ble::ble_routes());

    let app = if Path::new(&admin_dir).is_dir() {
        // SPA：未知路径回退到 index.html，交给前端路由处理。
        let admin = ServeDir::new(&admin_dir)
            .precompressed_gzip()
            .not_found_service(ServeFile::new(Path::new(&admin_dir).join("index.html")));
        api.fallback_service(admin)
    } else {
        api.fallback(admin_missing)
    };

    app.with_state(engines)
}

/// 健康检查：返回 JSON（HEALTHCHECK 仅校验 HTTP 200）。
async fn health() -> Response {
    json_ok(serde_json::json!({
        "status": "ok",
        "name": "xiaozhi-server-rust",
        "version": env!("CARGO_PKG_VERSION"),
    }))
}

/// 管理页面目录未配置 / 未构建时，给一个明确的提示而不是 404 空白。
async fn admin_missing() -> Response {
    (
        StatusCode::NOT_FOUND,
        "管理页面未找到：请先构建 h5App（web）并将 [server].admin_dir 指向其包含 index.html 的输出目录。",
    )
        .into_response()
}

async fn ws_handler(
    ws: WebSocketUpgrade,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
    State(engines): State<Arc<Engines>>,
) -> Response {
    if !auth_ok(&headers, &query, &engines.config.server) {
        return (StatusCode::UNAUTHORIZED, "unauthorized").into_response();
    }
    ws.on_upgrade(move |socket| async move {
        if let Err(e) = handle_handshake(socket, engines, &headers).await {
            tracing::warn!("会话异常: {e}");
        }
    })
}

pub(crate) fn auth_ok(
    headers: &HeaderMap,
    query: &HashMap<String, String>,
    server: &ServerConfig,
) -> bool {
    if server.expected_token.is_empty() {
        return true;
    }
    let expected = format!("Bearer {}", server.expected_token);
    if let Some(auth) = headers.get(header::AUTHORIZATION) {
        if auth.to_str().is_ok_and(|v| v == expected) {
            return true;
        }
    }
    // 兜底：浏览器 WebSocket 无法自定义请求头，允许 ?token= 查询参数
    // （管理页面通过 URL 带 token 联调；设备侧仍走 Authorization 头）
    query
        .get("token")
        .is_some_and(|t| t == &server.expected_token)
}

async fn handle_handshake(
    socket: WebSocket,
    engines: Arc<Engines>,
    headers: &HeaderMap,
) -> anyhow::Result<()> {
    // 设备身份（固件 WS 握手头；测试台 macApp 也带）：接入日志据此核对是哪台设备
    let device_id = headers
        .get("Device-Id")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("unknown");
    let client_id = headers
        .get("Client-Id")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("unknown");
    // 握手起即收口到传输抽象：hello 收发与会话主循环走同一条 Transport 路径
    let mut transport = WsTransport::new(socket);
    let hello = recv_hello(&mut transport).await?;

    // 协商：上行跟随设备 hello（默认 16k），下行由服务器配置决定（默认 24k）。
    //
    // ⚠️ **二进制协议版本跟随设备 `hello.version`**（上行与下行同版本）：设备按其 NVS
    // `websocket.version` 同时决定收/发帧格式（`websocket_protocol.cc` 的 `version_`），
    // 下行若用服务端配置的另一个版本，设备会把带头的帧当纯 Opus 解析（或反之）→ 解码乱码。
    // 服务端 `[audio].binary_protocol_version` 的作用是**经 OTA 写进设备 NVS**
    // （见 `ota_payload` 的 `websocket.version`），二者因此天然一致；此处以设备上报为准，
    // 兼容"设备 NVS 被手工改成其他版本"的场景。
    let uplink_bin_ver = BinVersion::from_u8(hello.version);
    let downlink_bin_ver = uplink_bin_ver;
    let uplink_sr = hello
        .audio_params
        .as_ref()
        .map(|a| a.sample_rate)
        .unwrap_or(16_000);
    let downlink_sr = engines.config.audio.downlink_sample_rate;
    let downlink_frame_ms = engines.config.audio.downlink_frame_duration_ms;
    let session_id = Uuid::new_v4().to_string();

    // 服务器 hello：写入下行音频参数（设备据此解码播放）。
    // session_id 与会话主循环共用同一个（对齐设备侧关联语义），故 clone 一份进 hello。
    let server_hello = ServerMessage::Hello {
        transport: "websocket",
        session_id: Some(session_id.clone()),
        audio_params: Some(AudioParams {
            sample_rate: downlink_sr,
            channels: engines.config.audio.channels,
            frame_duration: downlink_frame_ms,
            ..AudioParams::default()
        }),
    };
    send_text(&mut transport, &server_hello)
        .await
        .map_err(|e| anyhow::anyhow!("发送 server hello 失败: {e}"))?;

    let params = SessionParams {
        output_codec: crate::media::media::AudioCodec::Opus,
        uplink_bin_ver,
        downlink_bin_ver,
        uplink_sr,
        downlink_sr,
        downlink_frame_ms,
        downlink_lead_ms: engines.config.audio.downlink_lead_ms,
        // 服务器侧水库（内存换流畅）：容量 + 起跑闸，见 [audio].downlink_buffer_*
        downlink_buffer_ms: engines.config.audio.downlink_buffer_ms,
        // 下载缓冲（PCM，秒级预算）：由水库容量派生，控制 TTS 流的下载流速
        download_buffer_ms: engines.config.audio.download_budget_ms(),
        downlink_buffer_start_ms: engines.config.audio.downlink_buffer_start_ms,
        downlink_buffer_start_max_ms: engines.config.audio.downlink_buffer_start_max_ms,
    };
    // 设备是否支持 MCP（**设备是 MCP Server，服务器是 MCP Client**）。
    // 最新固件 `GetHelloMessage()` 硬编码 `features.mcp: true`；极老固件不带该字段 → false，
    // 本会话一个 MCP 报文都不发，指令闸门退回「断开本次会话」（向后兼容，见 app/session.rs）。
    let mcp = hello.features.as_ref().is_some_and(|f| f.mcp);
    if hello.test {
        tracing::info!(
            "session {session_id} 测试台连接（hello.test=true，受理 asr_test/tts_test/llm_test）"
        );
    } else {
        // 设备正式会话（ASR/TTS 走这条 WS；按需建立——唤醒/按键触发对话时才连接，开机只走 OTA）
        tracing::info!(
            "session {session_id} 设备接入：device={device_id} client={client_id} \
             上行={uplink_sr}Hz 下行={downlink_sr}Hz/{downlink_frame_ms}ms 上行二进制协议=v{} 下行协议=v{} mcp={}",
            hello.version,
            engines.config.audio.binary_protocol_version,
            if mcp { "on" } else { "off" },
        );
    }
    run_session(
        &mut transport,
        engines,
        params,
        session_id,
        device_id.to_string(),
        client_id.to_string(),
        hello.test,
        mcp,
    )
    .await
}

async fn recv_hello<T: Transport>(transport: &mut T) -> anyhow::Result<ClientHello> {
    let frame = transport.recv().await;
    let s = match frame {
        IncomingFrame::Text(s) => s,
        IncomingFrame::Binary(_)
        | IncomingFrame::Audio(_)
        | IncomingFrame::AudioBoundary(_, _)
        | IncomingFrame::EndOfUtterance(_) => {
            anyhow::bail!("首个消息不是文本 hello")
        }
        IncomingFrame::Closed => anyhow::bail!("连接在无 hello 时关闭或出错"),
    };
    let cm: ClientMessage =
        serde_json::from_str(&s).map_err(|e| anyhow::anyhow!("解析 hello 失败: {e}"))?;
    match cm {
        ClientMessage::Hello(h) => Ok(h),
        _ => anyhow::bail!("首个消息不是 hello"),
    }
}
