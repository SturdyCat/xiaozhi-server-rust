//! 用户可见日志的**唯一**时间出口：`YYYY-MM-DD HH:MM:SS`（本地时区）。
//!
//! 此前控制台 fmt 层用默认 RFC3339 UTC（`2026-10-19T15:35:02.220899Z`），
//! 部署者对着手表看 `docker logs` 排查问题时还得心算时区，故统一改为本地墙上时间。
//! 消费方两处：[`crate::app::ws::log_stream`]（`/api/logs` 日志流前缀）与
//! `main.rs` 的 fmt 控制台层（`with_timer`）。
//!
//! 时区口径：chrono `Local` 按 `TZ` 环境变量 → `/etc/localtime` 解析；
//! Docker 镜像默认 `TZ=Asia/Shanghai`（见 Dockerfile 运行阶段），宿主机直跑则
//! 跟随系统时区。时间戳是展示层装饰，不参与任何协议/存储语义。
//!
//! 依赖说明：日志流曾刻意"不引第三方时间库"（纯 epoch 算术只能出 UTC 且无日期）；
//! 引入 chrono 后两处共用同一实现，避免 stdout 与日志流两套口径。

use chrono::Local;
use tracing_subscriber::fmt::format::Writer;
use tracing_subscriber::fmt::time::FormatTime;

/// 控制台 fmt 层计时器：每条日志前缀 `YYYY-MM-DD HH:MM:SS`（本地时区）。
pub struct LocalDatetime;

impl FormatTime for LocalDatetime {
    fn format_time(&self, w: &mut Writer<'_>) -> std::fmt::Result {
        w.write_str(&now_text())
    }
}

/// 当前本地时间的 `YYYY-MM-DD HH:MM:SS` 文本（fmt 层与日志流共用）。
pub fn now_text() -> String {
    Local::now().format("%Y-%m-%d %H:%M:%S").to_string()
}

#[cfg(test)]
mod tests;
