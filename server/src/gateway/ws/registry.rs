//! 设备 / 会话 / 对话记录 / 看板端点组——管理页「概览 / 设备 / 会话 / 对话记录」的数据源。
//!
//! | 端点 | 回答什么问题 |
//! |---|---|
//! | `GET /api/devices` | 有哪些设备连过、现在谁在线、最后一次连上是什么时候、用什么参数 |
//! | `GET /api/devices/{id}` | 这一台：当前连接 + 最近几轮对话 + 板型/固件版本（并入 MCP 探测结果） |
//! | `GET /api/sessions` | **此刻**有哪几条连接活着、各自处在哪个阶段（聆听/思考/说话） |
//! | `GET /api/conversations` | 最近聊了什么（可按 `device_id` 过滤；内存环形，非完整历史） |
//! | `GET /api/stats` | 看板聚合：在线数、轮次数、令牌、能力状态徽标 |
//!
//! ## 与既有端点的分工（刻意不重叠）
//!
//! - **令牌用量**归 [`crate::gateway::ws::usage`]（`/api/usage`，计量口径含"上下文压力"分解）；
//!   `/api/stats` 只把它**汇入看板**，不重复实现计量。
//! - **设备工具清单**归 [`crate::gateway::ws::mcp`]（`/api/mcp/tools`，完整工具列表）；
//!   本模块只在设备行上**带上**板型/固件版本这几个标量（那正是设备管理页要显示的）。
//! - **配置**归 `/api/config`；本模块一个字段都不写。
//!
//! ## 口径必须在响应里说清楚（`note` / `scope` 字段）
//!
//! 这些数字全部来自**进程内**注册表（见 [`crate::device::registry`]）：重启清零、`online`
//! 是会话生命周期派生值而非心跳。若不在响应里写明，用户会把它当成"今日统计"或"权威在线表"——
//! 那是本模块最容易产生的误解，因此每个响应都自带口径说明，前端直接展示即可。

use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::Response,
};
use serde::Deserialize;
use serde_json::{json, Value};

use super::json_response;
use crate::device::registry::{self, ConversationRecord, DeviceRecord, LiveSession};
use crate::runtime::Engines;

/// `GET /api/conversations` 的查询参数。
#[derive(Debug, Default, Deserialize)]
pub(super) struct ConversationsQuery {
    /// 只看这台设备（省略 = 全部）。
    pub device_id: Option<String>,
    /// 最多返回几条**会话**（默认 [`DEFAULT_CONVERSATIONS_LIMIT`]，上限 [`MAX_CONVERSATIONS_LIMIT`]）。
    pub limit: Option<usize>,
}

/// `GET /api/conversations` 默认返回的会话数。
const DEFAULT_CONVERSATIONS_LIMIT: usize = 20;
/// 上限：防止一个 `?limit=100000` 把整份环形缓冲序列化出去。
const MAX_CONVERSATIONS_LIMIT: usize = 200;

/// `GET /api/devices`：设备清单（最近活跃在前）。
pub(super) async fn devices(State(engines): State<Arc<Engines>>) -> Response {
    let now = registry::now_ms();
    let rows = engines.registry.devices();
    let online = rows.iter().filter(|d| d.online).count();
    let list: Vec<Value> = rows
        .iter()
        .map(|d| {
            let live = engines.registry.sessions_of(&d.device_id).len();
            let mcp = McpFacts::of(&engines, &d.device_id);
            device_json(d, live, now, &mcp)
        })
        .collect();
    json_response(
        StatusCode::OK,
        json!({
            "ok": true,
            "devices": list,
            "count": list.len(),
            "online_count": online,
            "note": "口径 = **本进程启动以来**见过的设备（注册表是内存态，重启清零，设备重连后自动回填）。\
                     `online` 由会话生命周期派生，不是心跳：进程被强杀时不会翻转，因此请结合 `age_ms` 一起看。\
                     板型 / 固件版本来自会话开始时的 MCP 探测，设备从未以 MCP 连接过时为 null。\
                     多台不带 `Device-Id` 头的设备会挤进同一条 `unknown` 记录。",
        }),
    )
}

/// `GET /api/devices/{device_id}`：单台设备详情（当前连接 + 最近对话）。
pub(super) async fn device_detail(
    State(engines): State<Arc<Engines>>,
    Path(device_id): Path<String>,
) -> Response {
    let now = registry::now_ms();
    let Some(device) = engines.registry.device(&device_id) else {
        return json_response(
            StatusCode::NOT_FOUND,
            json!({
                "ok": false,
                "device_id": device_id,
                "message": "没有这台设备的记录：设备号拼写错误，或它自本次服务启动以来还没连过\
                            （注册表是内存态，重启即清零）。",
            }),
        );
    };
    let live = engines.registry.sessions_of(&device_id);
    let convs = engines
        .registry
        .conversations_of(&device_id, DEFAULT_CONVERSATIONS_LIMIT);
    let mcp = McpFacts::of(&engines, &device_id);
    let mut body = device_json(&device, live.len(), now, &mcp);
    body["ok"] = json!(true);
    body["sessions"] = json!(live
        .iter()
        .map(|s| session_json(s, now))
        .collect::<Vec<_>>());
    body["conversations"] = json!(convs
        .iter()
        .map(|c| conversation_json(c))
        .collect::<Vec<_>>());
    body["note"] = json!(
        "`conversations` 是内存里保留的**最近几轮**（每会话最多 40 条、最多 32 个会话），\
         不是完整历史——要长期留存请开启 `[memory]`（走 `gm_messages`）。"
    );
    json_response(StatusCode::OK, body)
}

/// `POST /api/devices/{device_id}/inject`：向该设备的活跃会话注入一段文本（P1-2）。
///
/// 注入文本**作为用户话语**走与语音完全相同的管线（记录 → 指令闸门 → LLM → TTS → 下行），
/// 用于不经 ASR 直接验证 LLM/TTS 链路（联调利器）。会话侧通道见 `app/inject`：
/// 仅"设备 + 级联流水线"会话注册——测试台（独立请求-响应）与 AIUI 全链路（输入是 PCM）
/// 查无通道，报 409。
///
/// 状态码口径：400 文本非法；404 设备不在线；409 会话不支持/刚结束/忙；200 成功。
#[derive(serde::Deserialize)]
pub(super) struct InjectBody {
    /// 要注入的文本（trim 后非空，≤2000 字符）。
    pub text: String,
}

pub(super) async fn device_inject(
    State(engines): State<Arc<Engines>>,
    Path(device_id): Path<String>,
    axum::Json(body): axum::Json<InjectBody>,
) -> Response {
    let text = body.text.trim().to_string();
    if text.is_empty() {
        return json_response(
            StatusCode::BAD_REQUEST,
            json!({ "ok": false, "message": "text 不能为空" }),
        );
    }
    if text.chars().count() > 2_000 {
        return json_response(
            StatusCode::BAD_REQUEST,
            json!({ "ok": false, "message": "text 超长（上限 2000 字符）" }),
        );
    }
    // 找该设备的活跃**设备**会话（排除测试台），最近活跃优先——同设备多连接时注入最新的
    let mut candidates: Vec<_> = engines
        .registry
        .sessions_of(&device_id)
        .into_iter()
        .filter(|s| !s.is_test)
        .collect();
    candidates.sort_by_key(|s| std::cmp::Reverse(s.last_activity_ms));
    if candidates.is_empty() {
        return json_response(
            StatusCode::NOT_FOUND,
            json!({
                "ok": false,
                "device_id": device_id,
                "message": "设备当前不在线（没有活跃会话）。设备只在唤醒/按键触发对话时才建立连接\
                            （开机只走 OTA），请先唤醒设备再注入。",
            }),
        );
    }
    let mut last_err = None;
    for s in &candidates {
        match engines.inject.send(&s.session_id, text.clone()) {
            Ok(()) => {
                return json_response(
                    StatusCode::OK,
                    json!({
                        "ok": true,
                        "device_id": device_id,
                        "session_id": s.session_id,
                        "note": "文本已作为用户话语注入，走完整 LLM→TTS→下行管线；\
                                 对话记录与打断语义与正常语音轮次一致。",
                    }),
                );
            }
            Err(e) => last_err = Some((s.session_id.clone(), e)),
        }
    }
    // 全部候选都失败：按最后一个错误给口径（NoChannel=不支持，Closed=刚结束，Busy=忙）
    let (session_id, e) = last_err.expect("candidates 非空，循环至少执行一次");
    let message = match e {
        crate::workflow::pipeline::inject::InjectError::NoChannel => {
            "该会话不支持文本注入：测试台会话与 AIUI 全链路（识别在云端闭环）没有文本入口。"
        }
        crate::workflow::pipeline::inject::InjectError::Closed => "会话刚刚结束（通道已关闭），请刷新后重试。",
        crate::workflow::pipeline::inject::InjectError::Busy => {
            "会话正忙（正在处理上一轮对话，注入队列已满），请稍后重试。"
        }
    };
    json_response(
        StatusCode::CONFLICT,
        json!({ "ok": false, "device_id": device_id, "session_id": session_id, "message": message }),
    )
}

/// `GET /api/sessions`：此刻活着的连接（最近活跃在前）。
pub(super) async fn sessions(State(engines): State<Arc<Engines>>) -> Response {
    let now = registry::now_ms();
    let rows = engines.registry.sessions();
    let list: Vec<Value> = rows.iter().map(|s| session_json(s, now)).collect();
    json_response(
        StatusCode::OK,
        json!({
            "ok": true,
            "sessions": list,
            "count": list.len(),
            "test_count": rows.iter().filter(|s| s.is_test).count(),
            "note": "一条会话 = 一条 WebSocket 连接。设备每次唤醒通常新建一条连接（除非指令闸门命中后\
                     保留了音频通道）。`phase` 由服务端在关键节点更新，反映的是**服务端视角**，\n\
                     与设备此刻的物理状态可能有百毫秒级差异。",
        }),
    )
}

/// `GET /api/conversations[?device_id=..&limit=..]`：保留的最近对话。
pub(super) async fn conversations(
    State(engines): State<Arc<Engines>>,
    Query(q): Query<ConversationsQuery>,
) -> Response {
    let limit = q
        .limit
        .unwrap_or(DEFAULT_CONVERSATIONS_LIMIT)
        .min(MAX_CONVERSATIONS_LIMIT);
    // 按设备过滤走 `conversations_of`（它内部按设备筛完再截断，语义与"只看这台"一致）
    let rows: Vec<ConversationRecord> = match &q.device_id {
        Some(id) => engines.registry.conversations_of(id, limit),
        None => {
            let mut all = engines.registry.conversations();
            all.truncate(limit);
            all
        }
    };
    let list: Vec<Value> = rows.iter().map(conversation_json).collect();
    json_response(
        StatusCode::OK,
        json!({
            "ok": true,
            "conversations": list,
            "count": list.len(),
            "device_id": q.device_id,
            "limit": limit,
            "note": "**内存环形缓冲**：每个会话最多保留最近 40 条（user/assistant 各算一条），\
                     最多保留 32 个会话，先进先出。进程重启即清空。\
                     `truncated=true` 表示这段对话前面还有内容被挤掉了，界面需如实提示。",
        }),
    )
}

/// `GET /api/stats`：看板聚合（在线设备 / 活会话 / 轮次 / 令牌 / 能力状态）。
pub(super) async fn stats(State(engines): State<Arc<Engines>>) -> Response {
    let now = registry::now_ms();
    let counts = engines.registry.counts();
    let convs = engines.registry.conversations();
    let user_turns: usize = convs
        .iter()
        .map(|c| c.turns.iter().filter(|t| t.role == "user").count())
        .sum();
    let assistant_turns: usize = convs
        .iter()
        .map(|c| c.turns.iter().filter(|t| t.role == "assistant").count())
        .sum();

    // 能力状态徽标：直接复用 `/api/plugins` 的投影，只摘出"总览"需要的几个字段，
    // 避免把整份能力清单（含每个实现的字段元数据）塞进看板响应。
    let plugins = engines.host.capabilities_json();
    let capabilities: Vec<Value> = plugins["capabilities"]
        .as_array()
        .map(|caps| {
            caps.iter()
                .map(|c| {
                    json!({
                        "id": c["id"],
                        "display": c["display"],
                        "required": c["required"],
                        "enabled": c["enabled"],
                        "phase": c["phase"],
                        "deviation": c["deviation"],
                        "active_impl": c["active_impl"],
                        "last_error": c["last_error"],
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    json_response(
        StatusCode::OK,
        json!({
            "ok": true,
            "generated_at_ms": now,
            "uptime_ms": engines.registry.uptime_ms(),
            "scope": "since_start",
            "scope_note": "所有累计值（设备 / 轮次 / 对话）都是**自本次服务启动以来**的，不是\"今日\"\
                           ——注册表是内存态，重启清零。令牌用量同理（随进程累计）。",
            "devices": { "total": counts.devices, "online": counts.online },
            "sessions": { "live": counts.sessions, "test": counts.test_sessions },
            "conversations": { "retained": counts.conversations },
            "turns": {
                "retained": user_turns + assistant_turns,
                "user": user_turns,
                "assistant": assistant_turns,
                "note": "只统计**仍在内存缓冲里**的轮次（受环形上界影响），因此它是下限而非总量；\
                         真正的累计轮次看 `usage.turns`。",
            },
            "usage": engines.usage.global().to_json(None),
            "capabilities": capabilities,
            "version": env!("CARGO_PKG_VERSION"),
        }),
    )
}

// ---------------------------------------------------------------
// JSON 投影（一处定义，保证 `/api/devices` 与 `/api/devices/{id}` 字段完全一致）
// ---------------------------------------------------------------

/// 设备行需要从 **MCP 注册表** join 进来的几个标量。
///
/// 拆成入参而不是在函数里直接读 `Engines`，是为了让 `device_json` 成为**纯函数**——
/// 这样 JSON 契约（前端依赖的字段名）能在单测里钉住，而构造 `Engines` 需要真实模型，
/// 单测付不起那个代价。handler 负责查表，投影只负责拼。
#[derive(Debug, Clone, Default)]
struct McpFacts {
    board: Option<String>,
    firmware_version: Option<String>,
    tool_count: Option<usize>,
    probed_at_ms: Option<u64>,
}

impl McpFacts {
    /// 从 MCP 注册表取一台设备的事实（`None` = 该设备从未以 MCP 连接过）。
    fn of(engines: &Engines, device_id: &str) -> Self {
        match engines.mcp.get(device_id) {
            Some(t) => Self {
                board: Some(t.board),
                firmware_version: Some(t.firmware_version),
                tool_count: Some(t.tools.len()),
                probed_at_ms: Some(t.probed_at_ms),
            },
            None => Self::default(),
        }
    }
}

fn device_json(d: &DeviceRecord, live_sessions: usize, now: u64, mcp: &McpFacts) -> Value {
    json!({
        "device_id": d.device_id,
        "client_id": d.client_id,
        "online": d.online,
        "first_seen_ms": d.first_seen_ms,
        "last_seen_ms": d.last_seen_ms,
        "age_ms": now.saturating_sub(d.last_seen_ms),
        "session_count": d.session_count,
        "turn_count": d.turn_count,
        "live_sessions": live_sessions,
        "last_session_id": d.last_session_id,
        "last_error": d.last_error,
        "is_test": d.is_test,
        "mcp": d.mcp,
        "audio": {
            "uplink_sample_rate": d.uplink_sr,
            "downlink_sample_rate": d.downlink_sr,
            "downlink_frame_duration_ms": d.downlink_frame_ms,
            "binary_protocol_version": d.bin_version,
        },
        "board": mcp.board,
        "firmware_version": mcp.firmware_version,
        "tool_count": mcp.tool_count,
        "tools_probed_at_ms": mcp.probed_at_ms,
    })
}

fn session_json(s: &LiveSession, now: u64) -> Value {
    json!({
        "session_id": s.session_id,
        "device_id": s.device_id,
        "client_id": s.client_id,
        "is_test": s.is_test,
        "mcp": s.mcp,
        "started_ms": s.started_ms,
        "last_activity_ms": s.last_activity_ms,
        "duration_ms": now.saturating_sub(s.started_ms),
        "idle_ms": now.saturating_sub(s.last_activity_ms),
        "phase": s.phase.as_str(),
        "phase_display": s.phase.display(),
        "turn_count": s.turn_count,
        "last_user_text": s.last_user_text,
        "last_error": s.last_error,
        "audio": {
            "uplink_sample_rate": s.uplink_sr,
            "downlink_sample_rate": s.downlink_sr,
            "downlink_frame_duration_ms": s.downlink_frame_ms,
            "binary_protocol_version": s.bin_version,
        },
    })
}

fn conversation_json(c: &ConversationRecord) -> Value {
    json!({
        "session_id": c.session_id,
        "device_id": c.device_id,
        "started_ms": c.started_ms,
        "last_ms": c.last_ms,
        "turn_count": c.turns.len(),
        "truncated": c.truncated(),
        "dropped": c.dropped,
        "turns": c.turns.iter().map(|t| json!({
            "role": t.role,
            "text": t.text,
            "at_ms": t.at_ms,
            "elapsed_ms": t.elapsed_ms,
        })).collect::<Vec<_>>(),
    })
}

#[cfg(test)]
mod tests;
