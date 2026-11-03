//! 管理台通道：`GET /api/admin/ws`（**WebSocket**，双向）——实时推送 + RPC 复用。
//!
//! ## 用途
//!
//! `/api/ws` 已被小智设备协议占用，管理台 WS 用独立路径。本通道承载两类通讯：
//!
//! 1. **实时推送**（原有）：EchoMuse 设备面版本变化推送。设备上线/离线/批准/设置/
//!    删除都会自增设备面版本号（[`crate::device::echomuse::subscribe_devices_version`]
//!    的 watch 通道），变化即推给管理台。同一机制还承载**服务器状态**推送：出口 IP
//!    解析完成时自增状态版本号（[`crate::plugins::geo::subscribe_status_version`]），
//!    变化即推 `server_status_changed`；首帧 hello 内嵌状态快照，首连即有数据。
//! 2. **管理 API 的 RPC 复用**：管理台客户端与服务器之间的 API 通讯**始终走 WS**
//!    （web 与 macApp；设备连接协议与大文件传输除外）。客户端发
//!    `{"type":"rpc","id":<n>,"method":"GET","path":"/api/stats"}`，服务端把
//!    (method, path, body) 打进**既有 axum Router**（`tower::ServiceExt::oneshot`），
//!    复用同一批 handler（鉴权在升级时已完成，路由内开关/404 语义原样保留），
//!    回 `{"type":"rpc_result","id":<n>,"status":200,"body":{...}}`。
//!    —— REST 路由表因此无需为 WS 复制一份业务逻辑；静态托管与设备协议不经过它。
//!
//! ## 协议契约（客户端按此实现，不可偏离）
//!
//! - 连接建立后服务端**立刻**发 `{"type":"hello","version":<当前版本号>,
//!   "server_status":{...}}`——`server_status` 是服务器状态快照（首连自动下发，
//!   客户端不需要主动拉取就能渲染首页状态卡）；
//! - 版本变化时服务端推 `{"type":"echomuse_devices_changed","version":<新版本>}`；
//! - 服务器状态变化（当前：出口 IP 解析完成，含境外/失败落 note）推
//!   `{"type":"server_status_changed","status":{...}}`——`status` 形状与 hello 的
//!   `server_status` 完全一致（`schema_version` + `geo` 段），客户端用同一套解析；
//! - 客户端可发文本帧：`{"type":"ping"}` → 回 `{"type":"pong"}`（应用层心跳，
//!   浏览器 WS 没有协议层 ping API）；
//! - **RPC 帧**：`{"type":"rpc","id":<u64>,"method":"<GET|POST|PUT|DELETE>",
//!   "path":"/api/...","body":{...}?}`（body 仅写方法需要）→ 回
//!   `{"type":"rpc_result","id":<n>,"status":<HTTP 状态码>,"body":<JSON|null>}`；
//!   帧/参数非法或执行失败（超时等非 HTTP 错误）回
//!   `{"type":"rpc_result","id":<n>,"status":0,"error":"<原因>"}`。
//!   仅接受 `/api/` 前缀路径（静态资源与设备协议不在此通道）；`/api/admin/ws`
//!   自身被显式拒绝（防递归升级）。单请求默认超时 [`RPC_TIMEOUT_SECS`] 秒，
//!   帧可带可选 `"timeout"`（秒，钳制 1..=[`RPC_TIMEOUT_MAX_SECS`]）按请求延长——
//!   NLU 模型测试 / 节点测试等长端点用。
//! - 未知 type / 非 JSON / 非对象 / 二进制帧 → 回 `{"type":"error","error":"<原因>"}`
//!   且**连接保持不断开**；Close 帧结束循环。
//!
//! ## 实现分层
//!
//! 纯逻辑核心（[`hello_frame`]/[`event_frame`]/[`reply_for`]/[`admin_ws_loop`]）只
//! 见 channel 不见 socket，外置测试 mpsc/watch 直驱；axum handler 只做鉴权
//! （[`super::auth_ok`]，浏览器 WS 用 `?token=` 查询参数兜底）、升级与双向桥接。
//! `[echomuse]` disabled 时管理面 HTTP 全 404、版本号不动，但本通道照常可用
//! （它是管理台通用通道，不 404）——RPC 同理：打的是同一路由表，404 语义一致。

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use axum::body::Body;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Router;
use futures_util::{SinkExt, StreamExt};
use tokio::sync::{mpsc, watch};
use tower::util::ServiceExt;

use super::auth_ok;

// ---- RPC 路由表注入 ----

/// RPC 复用的 axum 路由表（`router()` 构建完成时注入；OnceLock 重复 set 静默忽略）。
/// 放模块级而非 State 的原因：handler 注册发生在路由表构建**过程中**，拿不到"包含
/// 自己的 Router"；构建完成后 set、连接建立后才读，时序天然安全。
static RPC_ROUTER: OnceLock<Arc<Router<()>>> = OnceLock::new();

/// 注入 RPC 复用的路由表（在 `router()` 返回前调用；多次调用仅首次生效）。
pub(crate) fn set_rpc_router(router: Router<()>) {
    let _ = RPC_ROUTER.set(Arc::new(router));
}

/// 单个 RPC 请求的默认超时（秒）。配置保存等写操作含插件重建，30s 足够宽裕。
const RPC_TIMEOUT_SECS: u64 = 30;

/// 逐请求超时的硬上限（秒）：rpc 帧可带 `timeout` 字段覆盖默认值（NLU 模型测试、
/// 节点测试等长端点需要 40-60s），但不能无界——防止客户端占住 handler 线程资源。
const RPC_TIMEOUT_MAX_SECS: u64 = 120;

/// RPC 允许的 HTTP 方法（与路由表上声明的写法对齐；OPTIONS/HEAD 等管理台用不到）。
const RPC_METHODS: &[&str] = &["GET", "POST", "PUT", "DELETE"];

/// RPC 帧解析结果：id + 逐请求超时（None=默认 [`RPC_TIMEOUT_SECS`]）+ 打包好的 HTTP 请求。
type RpcRequest = (u64, Option<u64>, axum::http::Request<Body>);

/// 纯函数：把一条 rpc 帧解析成 (id, 超时覆盖, Request)。失败返回原因字符串。
///
/// 校验：对象含 `type=="rpc"`、非负整数 `id`、[`RPC_METHODS`] 中的方法、`/api/`
/// 前缀路径（拒绝 `/api/admin/ws` 防递归升级）；`body` 仅写方法接受且必须是对象；
/// 可选 `timeout`（秒）钳制到 1..=[`RPC_TIMEOUT_MAX_SECS`]。
pub(super) fn parse_rpc_frame(v: &serde_json::Value) -> Result<RpcRequest, String> {
    if v.get("type").and_then(|t| t.as_str()) != Some("rpc") {
        return Err("不是 rpc 帧".into());
    }
    let id = v
        .get("id")
        .and_then(|i| i.as_u64())
        .ok_or("rpc 帧缺少非负整数 id")?;
    let timeout = match v.get("timeout") {
        None | Some(serde_json::Value::Null) => None,
        Some(t) => {
            let secs = t.as_u64().ok_or("timeout 必须是非负整数（秒）")?;
            Some(secs.clamp(1, RPC_TIMEOUT_MAX_SECS))
        }
    };
    let method = v
        .get("method")
        .and_then(|m| m.as_str())
        .ok_or("rpc 帧缺少 method")?
        .to_uppercase();
    if !RPC_METHODS.contains(&method.as_str()) {
        return Err(format!(
            "不支持的 method {method}（仅 GET/POST/PUT/DELETE）"
        ));
    }
    let path = v
        .get("path")
        .and_then(|p| p.as_str())
        .ok_or("rpc 帧缺少 path")?;
    if !path.starts_with("/api/") {
        return Err("path 必须以 /api/ 开头".into());
    }
    if path == "/api/admin/ws" {
        return Err("/api/admin/ws 不能经 RPC 复用".into());
    }
    let write = method != "GET";
    let body_value = v.get("body");
    if body_value.is_some() && !write {
        return Err("GET 请求不应携带 body".into());
    }
    if write && body_value.is_none_or(|b| !b.is_object()) {
        return Err("写方法必须携带 JSON 对象 body".into());
    }
    let payload = if write {
        // 上方已校验：写方法必带 JSON 对象 body。
        let body_value = body_value.expect("写方法 body 已校验为对象");
        serde_json::to_vec(body_value).map_err(|e| format!("body 序列化失败: {e}"))?
    } else {
        Vec::new()
    };
    let mut builder = axum::http::Request::builder()
        .method(method.as_str())
        .uri(path);
    if write {
        builder = builder.header(
            axum::http::header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        );
    }
    let req = builder
        .body(Body::from(payload))
        .map_err(|e| format!("构造请求失败: {e}"))?;
    Ok((id, timeout, req))
}

/// 把一条 rpc 文本帧（已是 JSON 对象）交给路由表执行，返回 rpc_result 帧。
/// 内部带 [`RPC_TIMEOUT_SECS`] 超时；路由表未注入/执行失败按 `status:0` 处理。
pub(super) async fn execute_rpc(router: Arc<Router<()>>, frame: &serde_json::Value) -> String {
    let (id, timeout, req) = match parse_rpc_frame(frame) {
        Ok(ok) => ok,
        Err(reason) => return rpc_error_frame(id_of(frame), &reason),
    };
    let exec = async {
        // Router 内部是 Arc，clone 便宜；oneshot 需要 by value。
        let resp = (*router)
            .clone()
            .oneshot(req)
            .await
            .map_err(|e| format!("路由执行失败: {e}"))?;
        let status = resp.status().as_u16();
        let bytes = axum::body::to_bytes(resp.into_body(), 16 * 1024 * 1024)
            .await
            .map_err(|e| format!("读取响应失败: {e}"))?;
        // status 始终透传（客户端按 HTTP 语义报错，如 404 的"未启用对应功能"）；
        // body 非 JSON（如静态 fallback 的 404 文本页）时置 null 并附原因，不吞状态码。
        let (body, note) = if bytes.is_empty() {
            (serde_json::Value::Null, None)
        } else {
            match serde_json::from_slice(&bytes) {
                Ok(v) => (v, None),
                Err(e) => (
                    serde_json::Value::Null,
                    Some(format!("响应不是合法 JSON: {e}")),
                ),
            }
        };
        Ok::<(u16, serde_json::Value, Option<String>), String>((status, body, note))
    };
    let timeout_secs = timeout.unwrap_or(RPC_TIMEOUT_SECS);
    match tokio::time::timeout(Duration::from_secs(timeout_secs), exec).await {
        Ok(Ok((status, body, note))) => {
            let mut frame = serde_json::json!({
                "type": "rpc_result", "id": id, "status": status, "body": body,
            });
            if let Some(note) = note {
                frame["error"] = serde_json::json!(note);
            }
            frame.to_string()
        }
        Ok(Err(reason)) => rpc_error_frame(id, &reason),
        Err(_) => rpc_error_frame(id, "请求超时"),
    }
}

/// 从原始帧里取 id（解析失败时也尽量回带，便于客户端丢弃对应的 pending）。
fn id_of(frame: &serde_json::Value) -> u64 {
    frame.get("id").and_then(|i| i.as_u64()).unwrap_or(0)
}

/// `status:0` 的 rpc_result 错误帧（非 HTTP 失败：帧非法/超时/路由未注入）。
fn rpc_error_frame(id: u64, error: &str) -> String {
    serde_json::json!({ "type": "rpc_result", "id": id, "status": 0, "error": error }).to_string()
}

// ---- 帧构造（纯函数，测试直驱） ----

/// 服务器状态推送的载荷：`schema_version` + `geo` 段（快照唯一来源
/// [`crate::plugins::geo::status_snapshot`]）。hello 的 `server_status` 字段与
/// `server_status_changed` 的 `status` 字段共用本函数——两处形状逐字一致。
pub(super) fn server_status_payload() -> serde_json::Value {
    serde_json::json!({
        "schema_version": 1,
        "geo": crate::plugins::geo::status_snapshot(),
    })
}

/// 连接建立后的第一帧：告知当前设备面版本号（客户端拿它对齐增量）+
/// 服务器状态快照（首连自动下发，首页状态卡无需手动刷新）。
pub(super) fn hello_frame(version: u64) -> String {
    serde_json::json!({
        "type": "hello",
        "version": version,
        "server_status": server_status_payload(),
    })
    .to_string()
}

/// 版本变化事件帧：管理台收到后刷新设备列表。
pub(super) fn event_frame(version: u64) -> String {
    serde_json::json!({ "type": "echomuse_devices_changed", "version": version }).to_string()
}

/// 服务器状态变化事件帧：与 hello 内嵌快照同一形状，客户端复用同一套解析。
pub(super) fn status_event_frame() -> String {
    serde_json::json!({
        "type": "server_status_changed",
        "status": server_status_payload(),
    })
    .to_string()
}

/// 应用层心跳应答（浏览器 WS 没有协议层 ping API）。
fn pong_frame() -> String {
    serde_json::json!({ "type": "pong" }).to_string()
}

/// 错误帧：只说明原因，连接保持。
fn error_frame(error: &str) -> String {
    serde_json::json!({ "type": "error", "error": error }).to_string()
}

/// 已定义但**无需回执**的消息类型（收下即忽略，连接保持）：未来扩展协议用，
/// 当前为空——除 ping/rpc 外的任何输入都按非法/未知回 error 帧。
const SILENT_TYPES: &[&str] = &[];

/// 客户端文本帧的回执决策（纯逻辑，测试直驱）：`ping` → pong；未知 type /
/// 非 JSON / 非对象 → error 帧；`SILENT_TYPES` 里的约定类型收下不回（None）。
/// `rpc` 帧不在此处应答——它由 [`admin_ws_loop`] 拦下交给 [`execute_rpc`]。
pub(super) fn reply_for(text: &str) -> Option<String> {
    let kind = serde_json::from_str::<serde_json::Value>(text)
        .ok()
        .filter(|v| v.is_object())
        .and_then(|v| v.get("type").and_then(|t| t.as_str()).map(str::to_string));
    match kind.as_deref() {
        Some("ping") => Some(pong_frame()),
        Some(t) if SILENT_TYPES.contains(&t) => None,
        // 有 type 但不认识 / 压根不是含 type 的 JSON 对象，分别说明原因。
        _ => Some(error_frame(if kind.is_some() {
            "未知的消息类型"
        } else {
            "消息必须是含 type 的 JSON 对象"
        })),
    }
}

// ---- 推送核心（channel 直驱，不碰 socket） ----

/// socket → 核心循环的上行事件。
#[derive(Debug)]
pub(crate) enum WsIn {
    /// 客户端发来的一条文本帧。
    Text(String),
    /// 客户端连接已结束（Close 帧 / 流关闭 / 读错误）。
    Closed,
}

/// 推送核心循环：连接后立刻发 hello（当前版本 + 服务器状态快照），随后
/// `tokio::select!` 挂上行事件、设备面版本 watch 与状态版本 watch——变化即发
/// 对应事件帧，`ping` 即回 pong，收到 Closed（或上行发送端全被丢弃）结束循环；
/// out 发送失败（订阅方没了）同样结束。
///
/// 两个 watch 首帧都用 `borrow_and_update` 取值并标记"已见"：订阅前已发生的 bump
/// 不重放（hello 的快照/版本号已覆盖），订阅后的 bump 一条不丢。
///
/// RPC：`type=="rpc"` 的文本帧在此拦下，`tokio::spawn` 并发执行（不阻塞事件
/// 推送），结果经 `out` 回给客户端；路由表未注入（`rpc=None`）回 `status:0`。
pub(super) async fn admin_ws_loop(
    mut incoming: mpsc::Receiver<WsIn>,
    out: mpsc::Sender<String>,
    mut ver: watch::Receiver<u64>,
    mut status_ver: watch::Receiver<u64>,
    rpc: Option<Arc<Router<()>>>,
) {
    let current = *ver.borrow_and_update();
    let _ = status_ver.borrow_and_update();
    if out.send(hello_frame(current)).await.is_err() {
        return;
    }
    loop {
        tokio::select! {
            item = incoming.recv() => match item {
                Some(WsIn::Text(text)) => {
                    // rpc 帧优先拦下（reply_for 会把它当"未知类型"回 error）。
                    let frame = serde_json::from_str::<serde_json::Value>(&text)
                        .ok()
                        .filter(|v| v.is_object())
                        .filter(|v| v.get("type").and_then(|t| t.as_str()) == Some("rpc"));
                    if let Some(frame) = frame {
                        match &rpc {
                            Some(router) => {
                                let router = router.clone();
                                let out = out.clone();
                                tokio::spawn(async move {
                                    let result = execute_rpc(router, &frame).await;
                                    let _ = out.send(result).await;
                                });
                            }
                            None => {
                                let _ = out
                                    .send(rpc_error_frame(id_of(&frame), "RPC 服务未就绪"))
                                    .await;
                            }
                        }
                        continue;
                    }
                    if let Some(reply) = reply_for(&text) {
                        if out.send(reply).await.is_err() {
                            return; // 订阅方没了
                        }
                    }
                }
                Some(WsIn::Closed) | None => return,
            },
            changed = ver.changed() => {
                // 发送端是进程级单例，Err 只在理论路径出现，按会话结束处理。
                if changed.is_err() {
                    return;
                }
                let version = *ver.borrow_and_update();
                if out.send(event_frame(version)).await.is_err() {
                    return;
                }
            }
            changed = status_ver.changed() => {
                if changed.is_err() {
                    return;
                }
                let _ = status_ver.borrow_and_update();
                if out.send(status_event_frame()).await.is_err() {
                    return;
                }
            }
        }
    }
}

// ---- axum handler（鉴权 → 升级 → 双向桥接） ----

/// `GET /api/admin/ws`：升级为 WebSocket 并挂上推送核心。
pub(super) async fn admin_ws_handler(
    ws: WebSocketUpgrade,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
    State(engines): State<Arc<crate::runtime::Engines>>,
) -> Response {
    if !auth_ok(&headers, &query, &engines.config.server) {
        return (StatusCode::UNAUTHORIZED, "unauthorized").into_response();
    }
    ws.on_upgrade(serve_admin_ws)
}

/// 双向桥接：socket 与 [`admin_ws_loop`] 之间各转一个方向。
/// - 上行：socket 文本帧 → `WsIn::Text`；二进制帧按契约直接回 error 帧（连接
///   保持）；Close/读错 → `WsIn::Closed`，核心循环随之结束。
/// - 下行：核心循环出帧 → socket；写失败（客户端已走）即收尾并撤上行桥。
async fn serve_admin_ws(socket: WebSocket) {
    let (mut sink, mut stream) = socket.split();
    let (in_tx, in_rx) = mpsc::channel::<WsIn>(16);
    let (out_tx, mut out_rx) = mpsc::channel::<String>(64);
    let ver = crate::device::echomuse::subscribe_devices_version();
    // 服务器状态 watch：出口 IP 解析完成时 bump → 推 server_status_changed。
    let status_ver = crate::plugins::geo::subscribe_status_version();
    let rpc = RPC_ROUTER.get().cloned();
    tracing::debug!("管理台通道已连接（rpc={})", rpc.is_some());

    // 上行桥（socket → 核心循环）。Ping/Pong 是协议层心跳，axum 侧自动应答，
    // 到不了应用层。
    let bridge_out = out_tx.clone();
    let up = tokio::spawn(async move {
        while let Some(msg) = stream.next().await {
            match msg {
                Ok(Message::Text(text)) => {
                    if in_tx.send(WsIn::Text(text.to_string())).await.is_err() {
                        break; // 核心循环已退出，连接该收尾了
                    }
                }
                Ok(Message::Binary(_)) => {
                    let _ = bridge_out.send(error_frame("不支持二进制帧")).await;
                }
                Ok(Message::Ping(_) | Message::Pong(_)) => {}
                Ok(Message::Close(_)) | Err(_) => break,
            }
        }
        let _ = in_tx.send(WsIn::Closed).await;
    });

    // 推送核心（hello + 事件 + RPC + 回执）跑在自己的任务里。
    tokio::spawn(admin_ws_loop(in_rx, out_tx, ver, status_ver, rpc));

    // 下行桥（核心循环 → socket）：循环退出后其发送端被丢弃，本循环随 out_rx
    // 取尽而结束；客户端提前消失（写失败）则主动撤上行桥。
    while let Some(frame) = out_rx.recv().await {
        if sink.send(Message::Text(frame.into())).await.is_err() {
            break;
        }
    }
    up.abort();
    tracing::debug!("管理台通道已断开");
}
