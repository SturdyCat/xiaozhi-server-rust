//! `GET /api/metrics`：延迟/事件聚合的只读投影（数据源 [`crate::app::metrics::Metrics`]，
//! P0-5 事件总线的常驻订阅者写入的内存态）。
//!
//! 口径：**全内存态、进程重启清零**；整轮延迟样本窗口为最近 200 轮。响应自带 `note`，
//! 前端直接展示即可——与 `/api/stats`、`/api/devices` 同性质（进程内观测面，非持久统计）。

use std::sync::Arc;

use axum::{extract::State, response::Response};

use super::json_ok;
use crate::engine::Engines;

/// `GET /api/metrics`：会话/整轮延迟/TTS/异常聚合（内存态，重启清零）。
pub(super) async fn metrics_snapshot(State(engines): State<Arc<Engines>>) -> Response {
    json_ok(engines.metrics.snapshot_json())
}
