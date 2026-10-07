//! WebSocket 网关：HTTP 路由、握手与协商、鉴权，然后转入会话状态机
//! （[`crate::session::run_session`]）。
//!
//! ## 路由
//! - `GET /api/health`：健康检查，返回 JSON（status/name/version）。
//! - `GET /api/ws`：WebSocket 会话入口（先 [`auth_ok`] 鉴权，再 [`handle_handshake`] 协商）。
//! - `GET/PUT/POST /api/config`：管理页面读写当前配置（语义见下）。
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
//! - `PUT`/`POST`：写回启动加载的配置文件（TOML，**原注释会被覆盖丢失**）。
//!   内置默认启动、`config_path` 为 `None` 时返回 400（无法持久化）。
//! - ⚠️ 引擎相关参数（ASR/TTS/LLM）在启动时构建，改配置后**需重启 server** 才生效；
//!   仅 `[server]` 部分（监听 / token / 管理页目录）下次启动生效。

use anyhow::Context;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use axum::{
    extract::{ws::WebSocketUpgrade, ws::WebSocket, Json, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
use tower_http::services::{ServeDir, ServeFile};
use uuid::Uuid;

use crate::config::{Config, ServerConfig};
use crate::engine::Engines;
use crate::protocol::{AudioParams, BinVersion, ClientHello, ClientMessage, ServerMessage};
use crate::session::{run_session, send_text, SessionParams};
use crate::transport::{IncomingFrame, Transport, WsTransport};

/// 构造 Axum 路由：API（健康检查 / WebSocket）之外，其余路径静态托管
/// `config.server.admin_dir` 指向的管理页面（h5App 构建产物，含 index.html）。
/// 该目录不存在时，`/` 及以下返回友好提示而非崩溃。
pub fn router(engines: Arc<Engines>) -> Router {
    let admin_dir = engines.config.server.admin_dir.clone();

    let api = Router::new()
        .route("/api/health", get(health))
        .route("/api/ws", get(ws_handler))
        .route("/api/config", get(get_config).put(put_config).post(put_config))
        // 小智固件的 OTA 引导端点（带/不带尾斜杠都注册：固件配置的 URL 以 / 结尾）。
        // ⚠️ 所有接口必须在 /api 之下（接口规范），固件 OTA_URL 指到 /api/ota/ 即可——
        // 该地址完全可配置，无需对齐官方服务器的 /xiaozhi/ota/ 路径。
        .route("/api/ota", get(ota).post(ota))
        .route("/api/ota/", get(ota).post(ota))
        // 固件托管（上传/下载/当前版本查询）+ 发音人目录，并入同一 /api 前缀
        .merge(crate::firmware::router())
        .merge(crate::voices::router());

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
    (
        [(header::CONTENT_TYPE, "application/json; charset=utf-8")],
        serde_json::json!({
            "status": "ok",
            "name": "xiaozhi-server-rust",
            "version": env!("CARGO_PKG_VERSION"),
        })
        .to_string(),
    )
        .into_response()
}

/// 管理页面目录未配置 / 未构建时，给一个明确的提示而不是 404 空白。
async fn admin_missing() -> Response {
    (
        StatusCode::NOT_FOUND,
        "管理页面未找到：请先构建 h5App（web）并将 [server].admin_dir 指向其包含 index.html 的输出目录。",
    )
        .into_response()
}

/// OTA/版本检查端点（xiaozhi-esp32 固件开机约定，解析见固件 `main/ota.cc CheckVersion`）：
/// 设备携系统信息 POST 固件配置的 OTA_URL，响应引导其接入本服务的 WebSocket 并校时。
///
/// 响应契约（固件逐段解析，缺段仅告警不阻塞）：
/// - `websocket`：**扁平对象** `{url, token, version}`，固件原样写入 NVS，
///   `OpenAudioChannel` 读取——token 非空时固件自动加 `Bearer ` 前缀（对齐 /api/ws 鉴权）；
///   `version` = 二进制协议版本，**显式下发对齐固件**（不传则固件用内置默认，可能与
///   `[audio].binary_protocol_version` 错位，下行 Opus 会被解析成乱码）。
/// - `server_time`：`{timestamp: ms, timezone_offset: 分钟}`，固件 settimeofday 校时；
///   offset 固定 +480（UTC+8，部署面为中国局域网，不校时仅日志/显示时间漂移）。
/// - `firmware`：`{version, url}`——url 恒空 + 版本取服务端包版本 → 永不触发固件升级，
///   仅让固件日志打出 "Current is the latest version"。
/// - `activation`：不返回 → 设备跳过激活流程（那是 xiaozhi.me 官方服务器的机制）。
async fn ota(State(engines): State<Arc<Engines>>, headers: HeaderMap, body: String) -> Response {
    // 设备身份记录（Device-Id = MAC）：接入日志可核对是哪台设备
    let device_id = headers
        .get("Device-Id")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("unknown");
    let firmware = serde_json::from_str::<serde_json::Value>(&body)
        .ok()
        .and_then(|v| {
            let app = v.get("application")?;
            Some(format!(
                "{}/{}",
                app.get("name").and_then(|n| n.as_str()).unwrap_or("?"),
                app.get("version").and_then(|n| n.as_str()).unwrap_or("?")
            ))
        })
        .unwrap_or_else(|| "(无/解析失败)".to_string());
    tracing::info!("OTA 检查：device={device_id} firmware={firmware}");

    // WebSocket 地址跟随请求 Host：设备用哪个地址访问 OTA，就用哪个地址连 WS
    //（部署 IP/端口变化零维护）。HTTP/1.1 必带 Host；缺失兜底 localhost 并告警。
    let host = headers
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned)
        .unwrap_or_else(|| {
            tracing::warn!("OTA 请求缺 Host 头，websocket url 兜底 localhost（设备可能连不上）");
            format!("localhost:{}", engines.config.server.port)
        });

    let hosted = crate::firmware::latest().map(|m| m.version);
    let payload = ota_payload(
        &host,
        &engines.config.server.expected_token,
        engines.config.audio.binary_protocol_version,
        hosted.as_deref(),
    );
    (
        [(header::CONTENT_TYPE, "application/json; charset=utf-8")],
        payload.to_string(),
    )
        .into_response()
}

/// OTA 响应体（独立纯函数：契约对齐固件解析，单测锁定字段形状）。
fn ota_payload(host: &str, expected_token: &str, bin_ver: u8, hosted: Option<&str>) -> serde_json::Value {
    serde_json::json!({
        "websocket": {
            "url": format!("ws://{host}/api/ws"),
            "token": expected_token,
            "version": bin_ver,
        },
        "server_time": {
            "timestamp": std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0),
            "timezone_offset": 480,
        },
        "firmware": match hosted {
            // 已托管固件：下发版本 + 下载地址，设备 IsNewVersionAvailable 判定后自动升级
            Some(v) => serde_json::json!({
                "version": v,
                "url": format!("http://{host}/api/ota/firmware/latest"),
            }),
            // 未托管：url 空 → 固件只打 "Current is the latest version"，永不升级
            None => serde_json::json!({
                "version": env!("CARGO_PKG_VERSION"),
                "url": "",
            }),
        },
    })
}

/// 读取当前配置（供管理页面 UI 填充表单）。
/// 启动时指定了配置文件则实时从磁盘读取，否则返回内存中的内置默认配置（/data/models 生产路径）。
async fn get_config(State(engines): State<Arc<Engines>>) -> Response {
    // 健壮性：磁盘配置损坏/旧 schema 解析失败时回退内存配置（200），
    // 让管理页能正常打开——一次成功保存即用合法 TOML 覆盖修复坏文件，
    // 避免陷入"打不开 → 永远修不好"的死局。原因记入服务端日志。
    let cfg = match &engines.config_path {
        Some(p) => match Config::load(p) {
            Ok(c) => c,
            Err(e) => {
                tracing::error!(
                    "GET /api/config：配置文件 {p} 读取/解析失败，回退内置默认配置（可在管理页重新保存修复）: {e}"
                );
                (*engines.config).clone()
            }
        },
        None => (*engines.config).clone(),
    };
    match serde_json::to_string(&cfg) {
        Ok(json) => (
            [(header::CONTENT_TYPE, "application/json; charset=utf-8")],
            json,
        )
            .into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("序列化配置失败: {e}")).into_response(),
    }
}

/// 配置持久化（PUT /api/config 共用）：序列化为 TOML 写回。
///
/// 写入策略两级：
/// 1. **原子写**（首选）：写同目录临时文件 → `rename` 覆盖，避免写一半被杀导致配置损坏；
/// 2. **原地写回退**：⚠️ compose 把 `config.toml` 以**单文件 bind mount** 挂进容器时，
///    `rename` 跨挂载点必然失败（`EBUSY`，os error 16，实测）——此时回退直接覆写原文件
///    （非原子，但挂载单文件的场景下这是唯一可行路径），并清理残留 tmp。
pub(crate) fn persist_config(path: &str, cfg: &Config) -> anyhow::Result<()> {
    let toml_str = toml::to_string_pretty(cfg).context("序列化配置失败")?;
    let tmp = format!("{}.{}.tmp", path, uuid::Uuid::new_v4());
    let atomic = std::fs::write(&tmp, &toml_str).and_then(|()| std::fs::rename(&tmp, path));
    if atomic.is_ok() {
        return Ok(());
    }
    let atomic_err = atomic.unwrap_err();
    let fallback = std::fs::write(path, &toml_str);
    let _ = std::fs::remove_file(&tmp); // 清理 rename 失败残留的临时文件
    if let Err(e) = fallback {
        let hint = if e.kind() == std::io::ErrorKind::PermissionDenied {
            "（配置文件或所在目录为只读/属主不符。请检查宿主挂载是否误加 :ro，或修正属主与权限后重启 server。）"
        } else {
            ""
        };
        anyhow::bail!("写入配置失败: {e}{hint}（原子写失败原因: {atomic_err}）");
    }
    tracing::warn!(
        "配置已原地写入 {path}（挂载为单文件 bind mount，rename 原子替换不可用: {atomic_err}）"
    );
    Ok(())
}

/// 保存配置（供管理页面 UI 提交）。写回启动时加载的配置文件（TOML，原注释会丢失）。
/// 注意：server 进程内引擎（ASR/TTS/LLM）在启动时构建，改配置后需重启 server 才对引擎生效；
/// 仅 [server] 部分（监听地址 / token / 管理页目录）可在下次启动时生效。
async fn put_config(
    State(engines): State<Arc<Engines>>,
    // 收原始 Value 手动解析：反序列化失败可返回**带精确原因**的 400
    //（Json<Config> 提取器的默认拒绝只有笼统状态码，客户端难定位）。
    Json(raw): Json<serde_json::Value>,
) -> Response {
    let body: Config = match serde_json::from_value(raw) {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("PUT /api/config：客户端配置 JSON 解析失败: {e}");
            return (
                StatusCode::BAD_REQUEST,
                format!("配置 JSON 解析失败: {e}"),
            )
                .into_response();
        }
    };
    let path = match &engines.config_path {
        Some(p) => p.clone(),
        None => {
            return (
                StatusCode::BAD_REQUEST,
                "server 以内置默认配置启动，未指定配置文件，无法持久化。请用 --config 指定 config.toml 后重启。",
            )
            .into_response()
        }
    };
    match persist_config(&path, &body) {
        Ok(()) => (
            StatusCode::OK,
            format!("配置已保存到 {path}；引擎相关参数需重启 server 生效。"),
        )
            .into_response(),
        // 健壮性：失败原因同时记入服务端日志（客户端 UI 只能看到状态码）
        Err(e) => {
            tracing::error!("PUT /api/config：写入 {path} 失败: {e:#}");
            (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")).into_response()
        }
    }
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

pub(crate) fn auth_ok(headers: &HeaderMap, query: &HashMap<String, String>, server: &ServerConfig) -> bool {
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
    query.get("token").is_some_and(|t| t == &server.expected_token)
}

async fn handle_handshake(
    socket: WebSocket,
    engines: Arc<Engines>,
    _headers: &HeaderMap,
) -> anyhow::Result<()> {
    // 握手起即收口到传输抽象：hello 收发与会话主循环走同一条 Transport 路径
    let mut transport = WsTransport::new(socket);
    let hello = recv_hello(&mut transport).await?;

    // 协商：上行跟随设备 hello（默认 16k），下行由服务器配置决定（默认 24k）。
    let uplink_bin_ver = BinVersion::from_u8(hello.version);
    let downlink_bin_ver = BinVersion::from_u8(engines.config.audio.binary_protocol_version);
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
        uplink_bin_ver,
        downlink_bin_ver,
        uplink_sr,
        downlink_sr,
        downlink_frame_ms,
    };
    if hello.test {
        tracing::info!("session {session_id} 测试台连接（hello.test=true，受理 asr_test/tts_test/llm_test）");
    }
    run_session(&mut transport, engines, params, session_id, hello.test).await
}

async fn recv_hello<T: Transport>(transport: &mut T) -> anyhow::Result<ClientHello> {
    let frame = transport.recv().await;
    let s = match frame {
        IncomingFrame::Text(s) => s,
        IncomingFrame::Binary(_) => anyhow::bail!("首个消息不是文本 hello"),
        IncomingFrame::Closed => anyhow::bail!("连接在无 hello 时关闭或出错"),
    };
    let cm: ClientMessage =
        serde_json::from_str(&s).map_err(|e| anyhow::anyhow!("解析 hello 失败: {e}"))?;
    match cm {
        ClientMessage::Hello(h) => Ok(h),
        _ => anyhow::bail!("首个消息不是 hello"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// OTA 响应契约（锁定固件 main/ota.cc 解析所需的字段形状）：
    /// websocket 为扁平对象 url/token/version；firmware.url 恒空（自建端不托管固件升级）。
    #[test]
    fn ota_payload_contract() {
        let v = ota_payload("192.168.99.250:8000", "tk123", 1, Some("2.0.4"));
        assert_eq!(v["websocket"]["url"], "ws://192.168.99.250:8000/api/ws");
        assert_eq!(v["websocket"]["token"], "tk123");
        assert_eq!(v["websocket"]["version"], 1);
        // 已托管固件：firmware 指向下载地址（设备版本比较通过即自动升级）
        assert_eq!(v["firmware"]["version"], "2.0.4");
        assert_eq!(v["firmware"]["url"], "http://192.168.99.250:8000/api/ota/firmware/latest");
        assert!(v["server_time"]["timestamp"].as_u64().unwrap() > 0);
        // 未托管：url 空 → 设备永不触发升级
        let none = ota_payload("h:1", "", 1, None);
        assert_eq!(none["firmware"]["url"], "");
        // 未配置鉴权（expected_token 空）：字段仍在，空 token 固件自动跳过 Authorization 头
        assert_eq!(none["websocket"]["token"], "");
    }

    /// PUT 管线回归：模拟 macApp 管理页「保存配置」发送的 JSON 形状
    /// （ConfigFormState.save 的字段与类型），走「解析 → TOML 序列化 → 再解析」
    /// 全链路，任何一步失败都会在服务端表现为 500。
    #[test]
    fn put_pipeline_client_json_roundtrip() {
        // 与 ConfigFormState.save 的 put(...) 一致（port/xfyun 为新 schema）
        let client_json = serde_json::json!({
            "server": {
                "port": 8000,
                "expected_token": "tk",
                "worker_threads": 2,
                "admin_dir": "/app/web"
            },
            "audio": {
                "downlink_sample_rate": 24000,
                "downlink_frame_duration_ms": 60,
                "channels": 1,
                "binary_protocol_version": 1
            },
            "asr": {
                "model": "/data/models/SenseVoiceSmall/model.int8.onnx",
                "tokens": "/data/models/SenseVoiceSmall/tokens.txt",
                "language": "auto",
                "use_itn": true,
                "num_threads": 2,
                "provider": "cpu"
            },
            "vad": {
                "model": "/data/models/silero_vad.onnx",
                "threshold": 0.5,
                "min_silence_duration": 0.25,
                "min_speech_duration": 0.25
            },
            "tts": {
                "backend": "xfyun",
                "model": "/data/models/Kokoro/model.int8.onnx",
                "voices": "/data/models/Kokoro/voices.bin",
                "tokens": "/data/models/Kokoro/tokens.txt",
                "data_dir": "/data/models/Kokoro/espeak-ng-data",
                "dict_dir": "/data/models/Kokoro/dict",
                "lexicon": "/data/models/Kokoro/lexicon-us-en.txt,/data/models/Kokoro/lexicon-zh.txt",
                "lang": "zh",
                "speaker": 0,
                "speed": 1.0,
                "num_threads": 1,
                "xfyun": {
                    "app_id": "app",
                    "api_key": "key",
                    "api_secret": "secret",
                    "voice": "xiaoyan"
                }
            },
            "llm": {
                "api_base": "https://api.example.com/v1/responses",
                "api_key": "sk",
                "model": "gpt-4o",
                "system_prompt": "你是一个助手。",
                "max_history": 10,
                "temperature": 0.7,
                "stream": true
            }
        });

        // 1) JSON → Config（axum Json<Config> 的反序列化语义）
        let cfg: Config = serde_json::from_value(client_json.clone())
            .expect("客户端 JSON 应能反序列化为 Config");
        // 2) Config → TOML（persist_config 的序列化步骤）
        let toml_str = toml::to_string_pretty(&cfg).expect("Config 应能序列化为 TOML");
        // 3) TOML → Config（下次启动 Config::load 的语义）
        let reparsed: Config = toml::from_str(&toml_str).expect("写出的 TOML 应能重新解析");
        assert_eq!(reparsed.server.port, 8000);
        assert_eq!(reparsed.tts.backend, "xfyun");
        assert_eq!(reparsed.tts.xfyun.voice, "xiaoyan");
        assert_eq!(reparsed.llm.stream, true);
    }

    /// persist_config：写入后可被 Config::load 读回（字段保真）。
    #[test]
    fn persist_config_roundtrip() {
        let dir = std::env::temp_dir().join(format!("xz_cfg_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        let cfg: Config = toml::from_str(
            r#"
[server]
port = 8000
[tts]
backend = "xfyun"
[tts.xfyun]
app_id = "a"
voice = "xiaoyan"
"#,
        )
        .unwrap();
        persist_config(path.to_str().unwrap(), &cfg).unwrap();
        let loaded = Config::load(path.to_str().unwrap()).unwrap();
        assert_eq!(loaded.server.port, 8000);
        assert_eq!(loaded.tts.xfyun.voice, "xiaoyan");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// 旧 schema 配置文件（含已废弃的 listen 键）应能静默解析（未知键忽略）。
    #[test]
    fn legacy_config_with_listen_key_parses() {
        let legacy = r#"
[server]
listen = "192.168.99.250:8000"
expected_token = ""

[tts]
backend = "sherpa"
model = "/data/models/Kokoro/model.int8.onnx"
"#;
        let cfg: Config = toml::from_str(legacy).expect("旧 schema 配置应能解析");
        assert_eq!(cfg.server.port, 8000); // 缺省回退
        assert_eq!(cfg.tts.backend_kind().as_str(), "sherpa");
    }
}
