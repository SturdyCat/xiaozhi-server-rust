//! wasm 后端事件派发（mod.rs 与 pure_base.rs 共用单源，`#[path]` 引入）。
//!
//! 经 `skill.weather` 扩展点查天气（官方默认 `weather.official` 或用户兼容实现）。
//! 与原生 `WeatherService::forecast` 同契约：永远返回成功形态的 `Value`。
//! skills 目录「不联网」铁律不变：网络发生在 wasm 插件效果（宿主 fetcher 执行）。
//! 派发四步：扩展点解析（替换语义对 skills 透明）→ geo 预填（宿主服务，事件源侧
//! 职责）→ 派发事件（宿主同步跑完整瀑布）→ 从插件 kv 结果通道取回 JSON。
//! 调用纪律：`handle_event` 内含阻塞 HTTP（生产 fetcher），调用方必须在
//! `spawn_blocking`（skills 计划执行本就如此，与原生 reqwest 同纪律）。
//! （阶段二收口：dispatch 从 wasm 宿主 crate 的 bridge 归位到本能力模块——宿主
//! 不认识 `skill.weather` 这样的业务扩展点名。）

use serde_json::{json, Value};

/// 经 `skill.weather` 扩展点派发一次天气查询并读回结果。
pub(crate) fn forecast_via_plugin(
    host: &crate::plugins::wasm::WasmHost,
    city: &str,
    province: &str,
    range: &str,
) -> Value {
    use crate::plugins::wasm::Resolution;

    // ① 扩展点解析：未装载/全部失效 → 明确报缺（管理台上传或启动引导构建）
    let plugin_id = match host.resolve("skill.weather") {
        Resolution::Resolved(id) => id,
        other => {
            return json!({
                "ok": false,
                "speech": "天气插件未装载（管理台上传 weather.official 或检查启动引导日志）",
                "resolution": format!("{other:?}"),
            })
        }
    };

    // ② geo 预填：空城市回落服务器出口 IP 定位（与原生同口径；属宿主服务，
    //    插件侧按缺参明确报错——预填是事件源侧职责，见设计文档）
    let (city_clean, province_eff) = if city.trim().is_empty() {
        match crate::plugins::geo::current() {
            Some(loc) => {
                let c = if loc.city.trim().is_empty() {
                    loc.province.clone()
                } else {
                    loc.city.clone()
                };
                (c, loc.province.clone())
            }
            None => {
                return json!({
                    "ok": false,
                    "speech": "未指定城市，且服务器未能定位默认地区（可能处于境外网络），请告诉我城市名称",
                })
            }
        }
    } else {
        (city.trim().to_string(), province.trim().to_string())
    };

    // ③ 派发：宿主同步驱动完整瀑布（缓存命中→单步；冷路径→3 次批量回灌）
    let req_id = uuid::Uuid::new_v4().to_string();
    let payload = json!({
        "req_id": req_id,
        "city": city_clean,
        "province": province_eff,
        "range": range,
    })
    .to_string();
    if let Err(e) = host.handle_event(&plugin_id, "weather.forecast", &payload) {
        tracing::warn!(plugin = %plugin_id, "天气插件事件失败：{e:#}");
        return json!({"ok": false, "speech": "天气服务暂时不可用，请稍后再试"});
    }

    // ④ 结果通道：插件 kv `result:<req_id>`（失败也是 ok:false + 话术，同原生口径）
    match host.kv().get(&plugin_id, &format!("result:{req_id}")) {
        Some(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|e| {
            tracing::warn!(plugin = %plugin_id, "天气结果损坏：{e}");
            json!({"ok": false, "speech": "天气服务暂时不可用，请稍后再试"})
        }),
        None => {
            tracing::warn!(plugin = %plugin_id, "天气插件未写入结果（流程中断）");
            json!({"ok": false, "speech": "天气服务暂时不可用，请稍后再试"})
        }
    }
}
