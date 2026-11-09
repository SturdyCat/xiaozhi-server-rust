//! Tuya **DP 载荷解析**（纯函数单源，无宿主依赖）。
//!
//! 涂鸦报文两种形状都见过：裸 `{"1":true,"2":30}` 与包裹
//! `{"dps":{"1":true},"t":123}`。本模块只依赖 `serde_json` + std，
//! native LAN 数据面（`device/tuya/lan.rs`）与 guest 插件
//! （`plugins-src/tuya` 经 `#[path]` 共用同一份源码）同源——阶段三 DP
//! 业务语义迁移的单源证据；改语义只改这一处，两侧由各自测试钉住。

use std::collections::HashMap;

/// DP 载荷 → `dp_id → value` 映射（纯函数；非对象/坏 JSON → 空 map）。
///
/// 包裹形状取 `dps` 子对象；裸形状把顶层对象整体当 DP 表。
pub(crate) fn parse_dps(payload: &str) -> HashMap<String, serde_json::Value> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(payload) else {
        return HashMap::new();
    };
    let obj = match v.get("dps").filter(|d| d.is_object()) {
        Some(dps) => dps,
        None => &v,
    };
    obj.as_object()
        .map(|m| m.iter().map(|(k, val)| (k.clone(), val.clone())).collect())
        .unwrap_or_default()
}
