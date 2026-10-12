//! 内联配置测试 HTTP 端点（P0-2）：`POST /api/test/{llm,tts,vad}`。
//!
//! 与 WS 测试台（`session/bench.rs`）**并存**：端点按请求 ad-hoc 构建引擎（读磁盘最新
//! 配置），**不要求 `is_test` WS 会话**——管理页配置卡（Web 无麦克风）可直接调它测
//! LLM 连通性 / TTS 与 VAD 延迟。测量核心在 [`crate::app::bench_api`]（与 Session 解耦）。
//!
//! ⚠️ 密钥安全：请求里携带的 `api_key` / `api_base` 只用于本次 ad-hoc 构建，
//! 绝不写日志明文、绝不持久化。

use std::sync::Arc;

use axum::extract::State;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;

use crate::app::bench_api::{bench_llm, bench_tts, bench_vad};
use crate::engine::Engines;

#[derive(Deserialize)]
pub struct TestLlmReq {
    pub text: String,
    pub api_base: Option<String>,
    pub api_key: Option<String>,
}

#[derive(Deserialize)]
pub struct TestTtsReq {
    pub text: String,
    pub lang: Option<String>,
    pub vcn: Option<String>,
    pub speed: Option<f32>,
}

#[derive(Deserialize)]
pub struct TestVadReq {}

/// `POST /api/test/llm`：LLM 连通性 + 首 token 延迟（无历史单轮）。
pub(super) async fn test_llm(
    State(engines): State<Arc<Engines>>,
    axum::Json(req): axum::Json<TestLlmReq>,
) -> Response {
    let r = bench_llm(&engines, &req.text, req.api_base.as_deref(), req.api_key.as_deref()).await;
    axum::Json(r).into_response()
}

/// `POST /api/test/tts`：TTS 流式合成 + 首个音频 chunk 延迟。
pub(super) async fn test_tts(
    State(engines): State<Arc<Engines>>,
    axum::Json(req): axum::Json<TestTtsReq>,
) -> Response {
    let r = bench_tts(
        &engines,
        &req.text,
        req.lang.as_deref(),
        req.vcn.as_deref(),
        req.speed,
    )
    .await;
    axum::Json(r).into_response()
}

/// `POST /api/test/vad`：VAD 引擎构建（冷启动）+ 首次检测耗时。
pub(super) async fn test_vad(
    State(engines): State<Arc<Engines>>,
    axum::Json(_req): axum::Json<TestVadReq>,
) -> Response {
    let r = bench_vad(&engines).await;
    axum::Json(r).into_response()
}
