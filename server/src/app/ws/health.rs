//! `GET /api/health/engines`：引擎健康自检的只读投影（P1-6）。
//!
//! ## 定位
//!
//! 与 `GET /api/plugins` 的分工：那边是**实现级**清单（每个描述符一行），
//! 这边是**能力级**健康聚合（每个 capability 一行，附运行期探测结论）。
//! 两者的能力聚合**同源**——都出自 [`PluginHost::capability_summary`]，
//! 杜绝"总览页说正常、健康页说降级"的状态分叉（PRD P1-6 明示的最大风险）。
//!
//! ## 响应结构（见 `docs/p1-round1-design.md` §2.1）
//!
//! - `capabilities[]`：11 行，每行 capability/display/enabled/phase/deviation/
//!   active_impl/last_error/probe/config_section/implementations[]；
//! - `phase` 合并规则：host 快照 phase 与探测结论取**更坏**者——
//!   probe Fail 时必需能力 → `Failed`（对齐启动失败语义）、可选能力 → `Degraded`
//!   （坏掉只降级，语音服务照常）；host 已有更坏状态时保留 host 的（`last_error` 同理，
//!   host 的原因优先，避免覆盖"启动时构建失败"这类更根本的信息）；
//! - `disabled` 项不跑探测（`probe: null`、`last_error: null`）——
//!   显式关闭不是错误（PRD 验收 2）；
//! - `config_section`：前端跳转锚点（fullchain→"aiui"、firmware→"ota"、voices→null
//!   无对应 section，其余同 id）——由服务端统一下发，客户端不自维护第二份映射。
//!
//! ## 性能与幂等
//!
//! 探测是纯本地 fs 快速操作（<1ms 量级），handler 内联执行即可，无需 spawn_blocking；
//! 整个端点不构建引擎、不发网络请求，连续两次调用除 `generated_ms` 外逐字段一致
//! （PRD 验收 6）。

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::{extract::State, response::Response};

use super::json_ok;
use crate::engine::Engines;
use crate::plugins::health::{ProbeState, probe};
use crate::plugins::host::{Phase, PluginHost};
use crate::plugins::registry::{ALL_CAPABILITIES, Capability};

/// capability → 配置页 section id（前端跳转锚点；`None` = 无对应配置 section）。
///
/// 多数能力与其 section 同名；两处例外：fullchain 的配置段叫 `aiui`、
/// firmware 的配置段叫 `ota`（对齐 `ConfigSections` 的 id 命名）；
/// voices 是 app 层服务（无配置卡），返回 `None`。
fn config_section(cap: Capability) -> Option<&'static str> {
    match cap {
        Capability::FullChain => Some("aiui"),
        Capability::Firmware => Some("ota"),
        Capability::Voices => None,
        other => Some(other.id()),
    }
}

/// phase 严格程度序（Active < Degraded < Failed；Disabled 视为最低——
/// "探测出问题"永远比"没启用"更值得上报）。
fn rank(p: Phase) -> u8 {
    match p {
        Phase::Disabled => 0,
        Phase::Active => 1,
        Phase::Degraded => 2,
        Phase::Failed => 3,
    }
}

/// 两者取更坏（rank 高者胜）。
fn worse(a: Phase, b: Phase) -> Phase {
    if rank(b) > rank(a) { b } else { a }
}

/// `GET /api/health/engines`：全部能力的健康聚合（只读投影）。
pub(super) async fn engines_health(State(engines): State<Arc<Engines>>) -> Response {
    json_ok(engines_health_json(&engines.host))
}

/// 健康聚合的纯函数核心（与 HTTP 层解耦，便于单测直接构造 `PluginHost::plan` 断言）。
fn engines_health_json(host: &PluginHost) -> serde_json::Value {
    let live = host.config();
    let caps: Vec<serde_json::Value> = ALL_CAPABILITIES
        .iter()
        .map(|&cap| {
            let s = host.capability_summary(cap);
            // 未启用的能力不跑探测：显式关闭不是错误（PRD 验收 2）。
            let probe_report = if s.enabled { probe(&live, cap) } else { None };
            // phase 合并：probe Fail → 必需能力 Failed / 可选能力 Degraded，再与 host 取更坏。
            let (phase, last_error) = match &probe_report {
                Some(r) if r.state == ProbeState::Fail => {
                    let probe_phase = if s.capability.fatal_if_enabled() {
                        Phase::Failed
                    } else {
                        Phase::Degraded
                    };
                    let merged = worse(s.phase, probe_phase);
                    // 原因优先级：host 侧**严格更坏**（如启动期构建失败 vs 探测只降级）时
                    // 保留 host 的根因；否则用探测文案（含「怎么修」）——同坏/探测更坏
                    // 时探测描述的是"现在这台机器上"的问题，行动性更强。
                    let err = if rank(s.phase) > rank(probe_phase) {
                        s.last_error.clone()
                    } else {
                        r.detail.clone().or_else(|| s.last_error.clone())
                    };
                    (merged, err)
                }
                _ => (s.phase, s.last_error.clone()),
            };
            serde_json::json!({
                "capability": cap.id(),
                "display": cap.display(),
                "required": cap.required(),
                "enabled": s.enabled,
                "phase": phase.as_str(),
                "deviation": phase.is_deviation(),
                "active_impl": s.active_impl,
                "last_error": last_error,
                "config_section": config_section(cap),
                "probe": probe_report.as_ref().map(|r| r.to_json()),
                "implementations": s.impls.iter().map(|i| i.to_json()).collect::<Vec<_>>(),
            })
        })
        .collect();

    // 计数按**合并后终态**统计（横幅/健康卡显示的就是这些档位）。
    let phase_count = |p: Phase| {
        caps.iter()
            .filter(|c| c["phase"] == serde_json::json!(p.as_str()))
            .count()
    };
    let mut counts = serde_json::Map::new();
    counts.insert("active".into(), phase_count(Phase::Active).into());
    counts.insert("degraded".into(), phase_count(Phase::Degraded).into());
    counts.insert("failed".into(), phase_count(Phase::Failed).into());
    counts.insert("disabled".into(), phase_count(Phase::Disabled).into());
    counts.insert("total".into(), caps.len().into());

    serde_json::json!({
        "ok": true,
        // 服务端生成时刻（毫秒）：前端不依赖本地时钟
        "generated_ms": SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0),
        "counts": counts,
        "capabilities": caps,
    })
}

#[cfg(test)]
mod tests;
