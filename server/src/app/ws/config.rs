//! `/api/config` 读写端点组（含配置来源元信息）：
//! - `GET`：启动指定了配置文件则实时读盘，否则返回内存内置默认配置。
//! - `PUT/POST`：写回启动加载的配置文件（TOML，原注释会被覆盖丢失）；内置默认启动时 400。
//! - `GET /api/config/meta`：配置来源（路径 / 是否挂载卷持久化），排查"配置丢失去向"。
//!
//! 拆分子模块（原 `ws.rs`）：路由表与 WS 网关见 [`super`]。

use std::sync::Arc;

use anyhow::Context;
use axum::{
    extract::{Json, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
};

use crate::config::Config;
use crate::engine::Engines;

/// 读取当前配置（供管理页面 UI 填充表单）。
/// 启动时指定了配置文件则实时从磁盘读取，否则返回内存中的内置默认配置（/data/models 生产路径）。
pub(super) async fn get_config(State(engines): State<Arc<Engines>>) -> Response {
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
pub(super) async fn put_config(
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
                json_msg(false, "server 以内置默认配置启动，未指定配置文件，无法持久化。请用 --config 指定 config.toml 后重启。", None),
            )
                .into_response()
        }
    };
    match persist_config(&path, &body) {
        Ok(()) => {
            // 保存成功即回报**实际写入路径**——配置"看不到/丢失"类问题可由此一眼定位
            //（例如容器内路径与宿主机挂载不一致时，Toast 会显示真实路径）。
            let mut msg = format!("配置已保存到 {path}；引擎相关参数需重启 server 生效。");
            if !config_is_persistent(&path) {
                msg.push_str(
                    " ⚠️ 该路径不在挂载卷上：重建容器会丢失配置。请按 docker-compose.yml 挂载 ./server-data:/data（XIAOZHI_CONFIG=/data/config.toml）。",
                );
            }
            (
                StatusCode::OK,
                json_msg(true, &msg, Some(&path)),
            )
                .into_response()
        }
        // 健壮性：失败原因同时记入服务端日志（客户端 UI 只能看到状态码）
        Err(e) => {
            tracing::error!("PUT /api/config：写入 {path} 失败: {e:#}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                json_msg(false, &format!("{e:#}"), Some(&path)),
            )
                .into_response()
        }
    }
}

/// 统一 JSON 响应体（macApp 读取 `message` 显示到 Toast）。
fn json_msg(ok: bool, message: &str, config_path: Option<&str>) -> String {
    serde_json::json!({ "ok": ok, "message": message, "config_path": config_path }).to_string()
}

/// 配置持久化判定：配置文件本身是挂载点（单文件 bind mount），或位于挂载目录之下
///（如 /data/config.toml 落在 ./server-data:/data 卷内）→ 容器重建不丢。
/// 非 Linux（macOS 本地开发）或 /proc 不可读时按"持久"处理（不误报）。
pub(crate) fn config_is_persistent(path: &str) -> bool {
    match std::fs::read_to_string("/proc/self/mountinfo") {
        Ok(mi) => is_persistent_in(&mi, path),
        Err(_) => true,
    }
}

/// mountinfo 解析（独立函数便于单测）：挂载点为目标文件本身，或目标路径位于某挂载目录之下。
fn is_persistent_in(mountinfo: &str, path: &str) -> bool {
    for line in mountinfo.lines() {
        let mut fields = line.split(' ');
        // 格式：id parent major:minor root mount_point options ...
        let _ = fields.next();
        let _ = fields.next();
        let _ = fields.next();
        let _ = fields.next();
        let Some(target) = fields.next() else { continue };
        if target == path || path.starts_with(&format!("{target}/")) {
            return true;
        }
    }
    false
}

/// GET /api/config/meta：配置来源元信息（macApp 配置页展示，排查"配置去哪了"）。
pub(super) async fn config_meta(State(engines): State<Arc<Engines>>) -> Response {
    let (path, loaded_from) = match &engines.config_path {
        Some(p) => (Some(p.clone()), "file"),
        None => (None, "default"),
    };
    let persistent = path.as_deref().map(config_is_persistent);
    let hint = match (&path, persistent) {
        (None, _) => "以内置默认配置运行，保存不会持久化（用 --config/XIAOZHI_CONFIG 指定文件后重启）",
        (Some(_), Some(false)) => "⚠️ 配置文件不在挂载卷上：重建容器会丢失（请挂载 ./server-data:/data 并设 XIAOZHI_CONFIG=/data/config.toml）",
        _ => "",
    };
    (
        [(header::CONTENT_TYPE, "application/json; charset=utf-8")],
        serde_json::json!({
            "config_path": path,
            "loaded_from": loaded_from,
            "persistent": persistent,
            "hint": hint,
        })
        .to_string(),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

        /// 持久化判定：目录挂载下的文件、文件级 bind mount 均算持久；容器层文件不算。

        #[test]

        fn persistence_detection() {

            let mi = concat!(
            "25 30 0:23 / /data rw,relatime - ext4 /dev/sda1 rw
",
            "31 30 0:24 / /etc/hosts rw,relatime - ext4 /dev/sda1 rw
",
            "40 30 0:26 / /etc/xiaozhi/config.toml rw,relatime - ext4 /dev/sda1 rw
",
        );

            assert!(is_persistent_in(mi, "/data/config.toml"), "目录挂载下的文件应持久");

            assert!(is_persistent_in(mi, "/etc/xiaozhi/config.toml"), "文件级 bind mount 应持久");

            assert!(!is_persistent_in(mi, "/etc/xiaozhi/other.toml"), "容器层文件不应判持久");

            assert!(!is_persistent_in(mi, "/datax/config.toml"), "前缀相似但非子路径不应误判");

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
