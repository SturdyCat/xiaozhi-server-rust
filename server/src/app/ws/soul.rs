//! 灵魂端点组：预设清单 + 最终提示词预览。
//!
//! 为什么需要这两个端点：人格是**看不见的**功能。用户把字段填完、保存成功，也无法确认
//! "最终发给模型的到底是什么"——尤其在内置预设与手填字段**合并**之后。没有反馈，就只会得到
//! "配了没用 / 不敢改"（与记忆的「测试召回」同一个道理）。
//!
//! 端点：
//! - `GET  /api/soul/presets`：内置预设清单（含 `profile`，客户端可**原样喂给表单**做"载入默认"）；
//! - `POST /api/soul/preview`：给定**可选的草稿 `[soul]`**，返回最终 `instructions`、
//!   分段构成，以及"哪些字段来自预设、哪些被你覆盖"。未知预设/非法变量在这里就报错
//!   （= 保存前的校验器，不必先写盘）。

use std::sync::Arc;

use axum::{
    extract::{Json, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
};

use crate::engine::Engines;
use crate::plugins::soul::{self, presets, SoulConfig};

/// `GET /api/soul/presets`。
pub(super) async fn soul_presets() -> Response {
    json_response(serde_json::json!({
        "ok": true,
        "presets": presets::json_list(),
        "default_id": presets::DEFAULT_ID,
        "none_id": presets::NONE_ID,
        "note": "preset 是**可覆盖的基线**：留空（空串/空数组）的字段由预设补，填了的字段覆盖它；\
                 \"none\" = 不用预设。等价于直接在 config.toml 里写 preset = \"…\"。",
    }))
}

/// `POST /api/soul/preview`：`{ "device_id": "…"?, "soul": { … }? }`。
///
/// `soul` 是**表单草稿**（尚未保存也能预览）；省略则用盘上当前配置。
pub(super) async fn soul_preview(
    State(engines): State<Arc<Engines>>,
    Json(raw): Json<serde_json::Value>,
) -> Response {
    match preview_body(&engines.live_config(), &raw) {
        Ok(body) => json_response(body),
        Err(msg) => bad_request(&msg),
    }
}

/// 纯逻辑（可单测）：活配置 + 可选草稿 → 预览响应体。
fn preview_body(
    live: &crate::config::Config,
    raw: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    let mut cfg = live.clone();
    let asked = raw
        .get("device_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    let device_id = if asked.is_empty() {
        // 预览没有会话上下文：占位符比空串更能说明「{{device_id}} 只在会话里展开」
        "<device_id>".to_string()
    } else {
        asked.clone()
    };
    if let Some(draft) = raw.get("soul").filter(|v| !v.is_null()) {
        cfg.soul = serde_json::from_value::<SoulConfig>(draft.clone()).map_err(|e| {
            format!(
                "soul 草稿格式不正确: {e}。列表字段（traits/values/boundaries/catchphrases/examples）\
                 必须是数组，不是多行字符串。"
            )
        })?;
    }
    // 规范化（`preset` 空串 → 默认预设），与 `POST /api/config` 的入口语义一致
    cfg.normalize();
    let p = soul::preview::build(&cfg, &device_id).map_err(|e| format!("{e:#}"))?;
    let mut body = p.to_json();
    body["ok"] = serde_json::json!(true);
    // token 估算复用记忆注入的同一函数（口径一致，不另造一个估算器）
    body["approx_tokens"] =
        serde_json::json!(crate::plugins::memory::assemble::estimate_tokens(&p.instructions));
    body["device_id"] = serde_json::json!(asked);
    body["note"] = serde_json::json!(if p.enabled {
        "这是最终发给模型的 instructions（平台约束 + 人格 + 表达风格 + [llm].system_prompt）。\
         from_preset = 由内置预设补上的字段；overridden = 你自己填了值的字段。"
    } else {
        "人格未启用：实际发给模型的是 [llm].system_prompt 原文（逐字节一致）。\
         打开「启用人格档案」后再预览。"
    });
    Ok(body)
}

fn json_response(body: serde_json::Value) -> Response {
    (
        [(header::CONTENT_TYPE, "application/json; charset=utf-8")],
        body.to_string(),
    )
        .into_response()
}

fn bad_request(msg: &str) -> Response {
    (
        StatusCode::BAD_REQUEST,
        json_response(serde_json::json!({ "ok": false, "message": msg })),
    )
        .into_response()
}

#[cfg(test)]
mod tests;
