//! 管理台实时推送通道：`GET /api/admin/ws`（**WebSocket**，双向）。
//!
//! ## 用途
//!
//! `/api/ws` 已被小智设备协议占用，管理台 WS 用独立路径。此前 EchoMuse 设备
//! 列表靠 `GET /api/echomuse/devices/wait` 长挂轮询（事件保持，25s 一轮），
//! 该端点已删除，改由本通道推送：设备上线/离线/批准/设置/删除都会自增设备面
//! 版本号（[`crate::device::echomuse::subscribe_devices_version`] 的 watch 通道），
//! 本通道把版本变化实时推给管理台；web 浏览器与 macApp 各自平台的 WS 接入共享
//! 同一层 AdminRealtime，按下方协议接入。
//!
//! ## 协议契约（客户端按此实现，不可偏离）
//!
//! - 连接建立后服务端**立刻**发 `{"type":"hello","version":<当前版本号>}`；
//! - 版本变化时服务端推 `{"type":"echomuse_devices_changed","version":<新版本>}`；
//! - 客户端可发文本帧：`{"type":"ping"}` → 回 `{"type":"pong"}`（应用层心跳，
//!   浏览器 WS 没有协议层 ping API）；未知 type / 非 JSON / 非对象 / 二进制帧 →
//!   回 `{"type":"error","error":"<原因>"}` 且**连接保持不断开**；Close 帧结束循环。
//! - 未来可扩展更多事件类型；本次只推 `echomuse_devices_changed`。
//!
//! ## 实现分层
//!
//! 纯逻辑核心（[`hello_frame`]/[`event_frame`]/[`reply_for`]/[`admin_ws_loop`]）
//! 只见 channel 不见 socket，外置测试 mpsc/watch 直驱；axum handler 只做鉴权
//! （[`super::auth_ok`]，浏览器 WS 用 `?token=` 查询参数兜底）、升级与双向桥接。
//! `[echomuse]` disabled 时管理面 HTTP 全 404、版本号不动，但本通道照常可用
//! （它是管理台通用事件通道，不 404）。

use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use futures_util::{SinkExt, StreamExt};
use tokio::sync::{mpsc, watch};

use super::auth_ok;

// ---- 帧构造（纯函数，测试直驱） ----

/// 连接建立后的第一帧：告知当前设备面版本号（客户端拿它对齐增量）。
pub(super) fn hello_frame(version: u64) -> String {
    serde_json::json!({ "type": "hello", "version": version }).to_string()
}

/// 版本变化事件帧：管理台收到后刷新设备列表。
pub(super) fn event_frame(version: u64) -> String {
    serde_json::json!({ "type": "echomuse_devices_changed", "version": version }).to_string()
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
/// 当前为空——除 ping 外的任何输入都按非法/未知回 error 帧。
const SILENT_TYPES: &[&str] = &[];

/// 客户端文本帧的回执决策（纯逻辑，测试直驱）：`ping` → pong；未知 type /
/// 非 JSON / 非对象 → error 帧；`SILENT_TYPES` 里的约定类型收下不回（None）。
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

/// 推送核心循环：连接后立刻发 hello（当前版本），随后 `tokio::select!` 挂
/// 上行事件与版本 watch——变化即发事件帧，`ping` 即回 pong，收到 Closed
/// （或上行发送端全被丢弃）结束循环；out 发送失败（订阅方没了）同样结束。
///
/// 首帧用 `borrow_and_update` 取版本并标记"已见"：订阅前已发生的 bump 不重放
/// （客户端凭 hello 的版本号对齐），订阅后的 bump 一条不丢。
pub(super) async fn admin_ws_loop(
    mut incoming: mpsc::Receiver<WsIn>,
    out: mpsc::Sender<String>,
    mut ver: watch::Receiver<u64>,
) {
    let current = *ver.borrow_and_update();
    if out.send(hello_frame(current)).await.is_err() {
        return;
    }
    loop {
        tokio::select! {
            item = incoming.recv() => match item {
                Some(WsIn::Text(text)) => {
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
    let (out_tx, mut out_rx) = mpsc::channel::<String>(16);
    let ver = crate::device::echomuse::subscribe_devices_version();
    tracing::debug!("管理台实时通道已连接");

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

    // 推送核心（hello + 事件 + 回执）跑在自己的任务里。
    tokio::spawn(admin_ws_loop(in_rx, out_tx, ver));

    // 下行桥（核心循环 → socket）：循环退出后其发送端被丢弃，本循环随 out_rx
    // 取尽而结束；客户端提前消失（写失败）则主动撤上行桥。
    while let Some(frame) = out_rx.recv().await {
        if sink.send(Message::Text(frame.into())).await.is_err() {
            break;
        }
    }
    up.abort();
    tracing::debug!("管理台实时通道已断开");
}
