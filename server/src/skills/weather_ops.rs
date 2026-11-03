//! `weather.forecast` 技能能力：从进程级天气服务取数并组装结果。
//!
//! 本文件是 skills 目录内唯一会触发联网能力的入口，但它本身不直接发 HTTP——网络实际
//! 发生在 `crate::plugins::weather`（见 `server/src/skills/AGENTS.md`）。这里只取全局服务、
//! 抽取参数、调用其 `forecast` 并透传结果。

use super::SkillExecutionContext;
use anyhow::Result;
use serde_json::{json, Value};

/// 执行 `weather.forecast` 步骤。
///
/// 参数 `city` / `range` / `province` 均为可选；缺省时由天气服务回落到服务器出口 IP 定位。
/// 天气服务未初始化时返回友好的失败 JSON（不 panic）。
pub(super) fn forecast(args: &Value, _ctx: &SkillExecutionContext) -> Result<Value> {
    let svc = match crate::plugins::weather::global() {
        Some(s) => s,
        None => return Ok(json!({"ok": false, "speech": "天气服务未初始化"})),
    };
    let city = args
        .get("city")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    let province = args
        .get("province")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    let range = args
        .get("range")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    Ok(svc.forecast(&city, &province, &range))
}
