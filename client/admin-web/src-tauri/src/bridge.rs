//! 本地音频桥（W14b-1 / 架构路径 X）：Tauri Rust 侧的 loopback WS 服务。
//!
//! 职责边界（架构文档 §1.1④）：**只承载「Tauri Rust 音频硬件 ↔ 前端」的 PCM/Opus
//! 数据与控制命令，绝不承载测试协议帧**——协议编排在前端，前端持唯一 `/api/ws`
//! 会话（双 `/api/ws` = `test_buf` 会话级缓冲分裂，绝对禁止）。
//!
//! ## 安全策略（取舍见模块尾注）
//! - **只绑 `127.0.0.1:0`**（OS 分配空闲端口，固定端口必冲突）；
//! - **Host 校验**：仅 127.0.0.1 / localhost / [::1]（防 DNS rebinding）；
//! - **Origin 校验**：必须落在「窗口实际可加载的 origin 白名单」内（dev=Vite devUrl、
//!   内嵌=tauri://localhost、生产=服务器页面 URL 的 origin），scheme 限 http/https/tauri；
//!   拒绝 `null`（file:// / 沙箱页）与其它任意站点——防 CSWSH；
//! - **每launch随机 bridge token**：与端口同走 `initialization_script` 注入（同一条
//!   信任通道），WS 升级必须 `?bridge_token=` 精确匹配。取舍说明：页面的 Origin 是
//!   部署相关的局域网地址（http://<LAN-IP>:8000），**无法枚举白名单**，故 CSWSH 的
//!   主防线是随机 token（劫持者拿不到注入常量），Origin 白名单 + Host loopback 是纵深；
//! - 本地桥不经过 server `dev::guard`，以上三层是它自己的全部防线。
//!
//! ## 端口传递（§1.1③）
//! `WebviewWindowBuilder::initialization_script` 注入
//! `window.__XIAOZHI_BRIDGE_PORT__` / `__XIAOZHI_BRIDGE_TOKEN__`——init script
//! **无条件注入**（官方文档要求自行 guard origin），脚本内置 origin 守卫，只落在
//! 窗口可加载的 origin 上，绝不泄露给其它导航。**不用 invoke**：生产服务器页面下
//! IPC 被 ACL 拒绝（capabilities/default.json）。
//!
//! ## 控制协议
//! 前端 → Rust 文本帧：[`audio::BridgeCommand`]（start_capture / stop_capture /
//! discard_capture / play_recorded / stop_playback / tts_begin / tts_end / play_tts）；
//! 前端 → Rust **二进制帧**（W14b-2）：server `/api/ws` 下行 Opus（tts_test 合成结果，
//! 前端中转）→ `AudioManager::feed_tts_frame` 解码播放；
//! Rust → 前端：文本帧 [`audio::BridgeEvent`] + 二进制裸 Opus 帧（上行，v1 无封装头，
//! 前端原样中转到 server `/api/ws`）。连接断开即停采集/播放（清理）。

use std::sync::{Arc, Mutex};

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;
use futures_util::{SinkExt, StreamExt};

use crate::audio::{AudioManager, BridgeCommand, BridgeOut};

/// 本地桥 WS 路径。
pub const BRIDGE_WS_PATH: &str = "/bridge";

/// app 生命周期内固定的桥凭据（端口 + 每 launch 随机 token）。
#[derive(Debug, Clone)]
pub struct BridgeSecret {
    pub port: u16,
    pub token: String,
}

/// 判断 HTTP Host 头是否为 loopback（防 DNS rebinding；剥离端口后比对）。
pub fn is_loopback_host(host: &str) -> bool {
    // 去端口：IPv6 形如 [::1]:8000，其余 host:port。
    let bare = if let Some(rest) = host.strip_prefix('[') {
        match rest.find(']') {
            Some(idx) => &rest[..idx],
            None => rest,
        }
    } else {
        host.split(':').next().unwrap_or(host)
    };
    matches!(bare, "127.0.0.1" | "localhost" | "::1" | "[::1]")
}

/// Origin 校验：scheme 必须 http/https/tauri，且 origin 在窗口可加载白名单内。
pub fn is_origin_allowed(origin: Option<&str>, allowed: &[String]) -> bool {
    let Some(origin) = origin else {
        return false; // 浏览器 WS 一定带 Origin；缺失（非浏览器）按拒绝处理
    };
    let trimmed = origin.trim();
    if trimmed == "null" {
        return false; // file:// / 沙箱页
    }
    let scheme_ok = trimmed.starts_with("http://")
        || trimmed.starts_with("https://")
        || trimmed.starts_with("tauri://");
    scheme_ok && allowed.iter().any(|a| a == trimmed)
}

/// 由窗口地址推导「可注入 init script 的 origin 白名单」。
///
/// - `WebviewUrl::App`：dev 构建 = Vite devUrl（http://127.0.0.1:5173）；
///   生产内嵌 = tauri://localhost。两种都可能（devUrl 由 conf 的 devUrl 决定）。
/// - `WebviewUrl::External(url)`：该 URL 的 origin（scheme://host[:port]）。
pub fn allowed_origins_for_window(url: &tauri::WebviewUrl) -> Vec<String> {
    match url {
        tauri::WebviewUrl::App(_) => vec![
            "http://127.0.0.1:5173".to_string(),
            "tauri://localhost".to_string(),
        ],
        tauri::WebviewUrl::External(u) => vec![format!("{}://{}", u.scheme(), u.authority())],
        // tauri-utils 将 WebviewUrl 标记为 non_exhaustive；未来新增变体不注入（fail-closed）。
        _ => Vec::new(),
    }
}

/// 生成注入前端的 init script：origin 守卫 + 端口/token 常量（官方要求自行 guard）。
pub fn bridge_init_script(port: u16, token: &str, allowed_origins: &[String]) -> String {
    let list: Vec<String> = allowed_origins
        .iter()
        .map(|o| format!("'{}'", o.replace('\'', "")))
        .collect();
    format!(
        "(() => {{ try {{ const allowed = [{}]; if (allowed.indexOf(window.location.origin) !== -1) {{ window.__XIAOZHI_BRIDGE_PORT__ = {}; window.__XIAOZHI_BRIDGE_TOKEN__ = '{}'; }} }} catch (e) {{}} }})();",
        list.join(","),
        port,
        token.replace('\'', "")
    )
}

/// 共享桥状态：音频管理器 + 每 launch 随机 token。
pub struct BridgeState {
    pub audio: Mutex<AudioManager>,
    pub token: String,
}

/// 启动本地桥：`bind(127.0.0.1:0)` + tokio runtime + axum serve。
///
/// runtime 存 OnceLock 常驻（app 进程退出即回收端口）；返回凭据供 init script 注入。
/// 失败返回 Err（调用方诚实降级：不注入端口，前端按「桥不可用」呈现）。
pub fn start_bridge(allowed_origins: Vec<String>) -> Result<BridgeSecret, String> {
    static RUNTIME: std::sync::OnceLock<tokio::runtime::Runtime> = std::sync::OnceLock::new();
    let runtime = RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("tokio runtime 构建失败")
    });

    let listener = runtime
        .block_on(async { tokio::net::TcpListener::bind("127.0.0.1:0").await })
        .map_err(|e| format!("本地桥绑定 127.0.0.1:0 失败: {e}"))?;
    let port = listener
        .local_addr()
        .map_err(|e| format!("本地桥读取端口失败: {e}"))?
        .port();

    let token: String = {
        use std::time::{SystemTime, UNIX_EPOCH};
        // 每 launch 随机 token：进程 ID + 纳秒时钟双熵源十六进制（防脚本常量猜测）。
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        format!("{:016x}{:016x}", std::process::id(), nanos as u64)
    };

    let state = Arc::new(BridgeState {
        audio: Mutex::new(AudioManager::new()),
        token: token.clone(),
    });

    let app = Router::new()
        .route(BRIDGE_WS_PATH, get(ws_upgrade))
        .route("/health", get(health))
        .with_state(state);
    let allow = allowed_origins.clone();
    runtime.spawn(async move {
        if let Err(e) = axum::serve(listener, app).await {
            eprintln!("[bridge] 本地桥服务异常退出: {e}");
        }
        let _ = allow; // move 进闭包保活（当前校验用 Query/HeaderMap，白名单经 state 传递）
    });

    Ok(BridgeSecret { port, token })
}

async fn health() -> impl IntoResponse {
    (StatusCode::OK, "ok")
}

/// WS 升级入口：Host / Origin / bridge_token 三层校验，全过才升级。
async fn ws_upgrade(
    State(state): State<Arc<BridgeState>>,
    Query(params): Query<std::collections::HashMap<String, String>>,
    headers: HeaderMap,
    upgrade: WebSocketUpgrade,
) -> impl IntoResponse {
    // ① Host 必须 loopback（防 DNS rebinding）。
    let host_ok = headers
        .get(axum::http::header::HOST)
        .and_then(|v| v.to_str().ok())
        .map(is_loopback_host)
        .unwrap_or(false);
    if !host_ok {
        return (StatusCode::FORBIDDEN, "bridge: host not allowed").into_response();
    }
    // ② Origin 必须在窗口可加载白名单内（防 CSWSH）。
    let origin = headers
        .get(axum::http::header::ORIGIN)
        .and_then(|v| v.to_str().ok());
    if !is_origin_allowed(origin, &bridge_origins()) {
        return (StatusCode::FORBIDDEN, "bridge: origin not allowed").into_response();
    }
    // ③ 每 launch 随机 token（主防线；劫持者拿不到 init script 注入的常量）。
    match params.get("bridge_token") {
        Some(t) if constant_time_eq(t, &state.token) => {}
        _ => return (StatusCode::FORBIDDEN, "bridge: token mismatch").into_response(),
    }
    upgrade.on_upgrade(move |socket| handle_socket(socket, state))
}

/// Origin 白名单的运行期传递（lib.rs setup 写入；单测可覆盖）。
static BRIDGE_ORIGINS: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();

/// 注册 origin 白名单（setup 期调用一次）。
pub fn set_bridge_origins(origins: Vec<String>) {
    let _ = BRIDGE_ORIGINS.set(origins);
}

fn bridge_origins() -> Vec<String> {
    BRIDGE_ORIGINS.get().cloned().unwrap_or_default()
}

/// 常量时间比较（防时序侧信道；token 32 hex 字符，开销可忽略）。
fn constant_time_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// 单连接生命周期：命令处理 + 事件/Opus 帧推送；断开即清理采集与播放。
async fn handle_socket(socket: WebSocket, state: Arc<BridgeState>) {
    let (mut sink, mut stream) = socket.split();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<BridgeOut>();

    {
        let mut audio = state.audio.lock().expect("audio lock");
        audio.reset(); // 新连接接管：清掉旧连接遗留的采集/播放
        audio.set_event_sender(tx);
    }

    // 推送任务：事件 / Opus 二进制帧 → 前端。
    let push_task = tokio::spawn(async move {
        while let Some(out) = rx.recv().await {
            let message = match out {
                BridgeOut::Event(event) => Message::Text(event.to_json().into()),
                BridgeOut::Opus(bytes) => Message::Binary(bytes.into()),
            };
            if sink.send(message).await.is_err() {
                break;
            }
        }
    });

    // 命令循环：前端文本帧 → AudioManager。
    while let Some(Ok(message)) = stream.next().await {
        match message {
            Message::Text(text) => {
                let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
                    continue; // 非 JSON 文本帧丢弃（fail-closed）
                };
                let Some(command) = BridgeCommand::parse(&value) else {
                    continue; // 未知命令丢弃
                };
                state.audio.lock().expect("audio lock").handle_command(command);
            }
            // W14b-2 下行中转：server /api/ws 的 Opus 二进制帧（tts_test 合成结果）→
            // 解码播放。协议帧（JSON 文本）不走这条通道——本地桥只承载音频数据。
            Message::Binary(data) => {
                state
                    .audio
                    .lock()
                    .expect("audio lock")
                    .feed_tts_frame(&data);
            }
            Message::Ping(_) | Message::Pong(_) => {}
            Message::Close(_) => break,
        }
    }

    // 断开清理：停采集/播放并切断事件出口。
    state.audio.lock().expect("audio lock").reset();
    push_task.abort();
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Host 校验：仅 loopback 放行。
    #[test]
    fn host_validation() {
        assert!(is_loopback_host("127.0.0.1:8000"));
        assert!(is_loopback_host("localhost:12345"));
        assert!(is_loopback_host("[::1]:9999"));
        assert!(is_loopback_host("127.0.0.1"));
        assert!(!is_loopback_host("192.168.1.5:8000"));
        assert!(!is_loopback_host("evil.example.com"));
        assert!(!is_loopback_host("0.0.0.0:0"));
    }

    /// Origin 校验：scheme 白名单 + null/file 拒绝 + 白名单匹配。
    #[test]
    fn origin_validation() {
        let allowed = vec![
            "http://127.0.0.1:5173".to_string(),
            "tauri://localhost".to_string(),
            "http://192.168.1.20:8000".to_string(),
        ];
        assert!(is_origin_allowed(Some("http://127.0.0.1:5173"), &allowed));
        assert!(is_origin_allowed(Some("tauri://localhost"), &allowed));
        assert!(is_origin_allowed(Some("http://192.168.1.20:8000"), &allowed));
        assert!(!is_origin_allowed(Some("http://evil.example.com"), &allowed));
        assert!(!is_origin_allowed(Some("null"), &allowed));
        assert!(!is_origin_allowed(Some("file:///etc/passwd"), &allowed));
        assert!(!is_origin_allowed(None, &allowed));
    }

    /// init script：包含端口、token 与 origin 守卫。
    #[test]
    fn init_script_format() {
        let script = bridge_init_script(
            51234,
            "abc123",
            &["http://127.0.0.1:5173".to_string(), "tauri://localhost".to_string()],
        );
        assert!(script.contains("__XIAOZHI_BRIDGE_PORT__ = 51234"));
        assert!(script.contains("__XIAOZHI_BRIDGE_TOKEN__ = 'abc123'"));
        assert!(script.contains("'http://127.0.0.1:5173'"));
        assert!(script.contains("'tauri://localhost'"));
        assert!(script.contains("window.location.origin"));
    }

    /// origin 白名单推导：App → devUrl + tauri；External → 其 origin。
    #[test]
    fn allowed_origins_for_window_mapping() {
        let app = allowed_origins_for_window(&tauri::WebviewUrl::App("index.html".into()));
        assert_eq!(
            app,
            vec!["http://127.0.0.1:5173".to_string(), "tauri://localhost".to_string()]
        );
        let url: url::Url = "https://admin.example.com:8443/ui".parse().expect("url");
        let external = allowed_origins_for_window(&tauri::WebviewUrl::External(url));
        assert_eq!(external, vec!["https://admin.example.com:8443".to_string()]);
    }

    /// 常量时间比较：等长不等值 false，不同长度 false，相等 true。
    #[test]
    fn constant_time_eq_semantics() {
        assert!(constant_time_eq("abc", "abc"));
        assert!(!constant_time_eq("abc", "abd"));
        assert!(!constant_time_eq("abc", "abcd"));
        assert!(!constant_time_eq("", "a"));
        assert!(constant_time_eq("", ""));
    }

    /// token 校验拒绝路径：Query 缺参 / 错值都不放行（配合 ws_upgrade；此处验证比较逻辑）。
    #[test]
    fn bridge_token_mismatch_rejected() {
        let secret = BridgeSecret {
            port: 1,
            token: "deadbeef".to_string(),
        };
        assert!(constant_time_eq("deadbeef", &secret.token));
        assert!(!constant_time_eq("cafebabf", &secret.token));
    }
}
