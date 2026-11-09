//! `connector.esp-home` 的实体目录策略（阶段三 ESPHome 第一切片）。
//!
//! WASM 只决定「哪些实体对外暴露 + 显示名覆盖」这一**有界控制面决策**；协议握手、
//! noise_psk 加密、状态采集、音量命令执行和 BLE 代理全部留 native。插件拿到的只有
//! 实体目录元数据（索引/object_id/名称/种类/可写性），拿不到设备 ID、令牌、状态值
//! 和任何报文。（阶段三收口：seam 从宿主 crate 归位到本设备模块——`connector.esp-home`
//! 是设备业务扩展点名，宿主不持有；宿主只按字符串通用路由。）

use serde::Deserialize;
use serde_json::{json, Value};

use super::state::{CatalogPlan, EntityKind, ENTITIES};
use crate::plugins::wasm::{Resolution, WasmHost};

const MAX_ESPHOME_ENTITIES: usize = 64;
const MAX_ESPHOME_NAME_BYTES: usize = 128;
const RESULT_PREFIX: &str = "esphome-result";

/// 插件返回的单实体目录决策。
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct CatalogEntityDecision {
    pub index: usize,
    pub exposed: bool,
    /// 显示名覆盖；`None` = 保留原生静态名。
    #[serde(default)]
    pub name: Option<String>,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct CatalogResult {
    /// 路由声明：目录策略只允许 `native`（数据面不出原生，见模块注释）。
    #[serde(default)]
    pub route: Option<String>,
    pub entities: Vec<CatalogEntityDecision>,
}

/// 有界实体目录元数据（不含设备标识、令牌、状态值或任何报文）。
fn entity_catalog() -> Vec<Value> {
    ENTITIES
        .iter()
        .enumerate()
        .map(|(index, spec)| {
            let (kind, writable) = match spec.kind {
                EntityKind::Binary { .. } => ("binary_sensor", false),
                EntityKind::Number { .. } => ("number", true),
            };
            json!({
                "index": index,
                "object_id": spec.object_id,
                "name": spec.name,
                "kind": kind,
                "writable": writable,
            })
        })
        .collect()
}

/// 向 `connector.esp-home` 派发实体目录，读取放行/改名决策并构建 [`CatalogPlan`]。
///
/// fail-closed：扩展点无实现、事件失败、结果缺失/损坏、route 非 `native`、索引越界、
/// 名称为空或超长，一律显式报错——桥**不静默降级**为全放行（调用方决定是否启动）。
pub fn esp_home_catalog(host: &WasmHost) -> std::result::Result<CatalogPlan, String> {
    let catalog = entity_catalog();
    if catalog.len() > MAX_ESPHOME_ENTITIES {
        return Err(format!(
            "ESPHome 实体目录超限（{} > {MAX_ESPHOME_ENTITIES}）",
            catalog.len()
        ));
    }
    let plugin_id = match host.resolve("connector.esp-home") {
        Resolution::Resolved(id) => id,
        other => return Err(format!("ESPHome 目录策略扩展点无可用实现：{other:?}")),
    };
    let req_id = uuid::Uuid::new_v4().to_string();
    let payload = json!({"req_id": req_id, "entities": catalog}).to_string();
    let errors = host
        .handle_event(&plugin_id, "connector.esp-home.prepare", &payload)
        .map_err(|e| format!("ESPHome 目录策略插件事件失败：{e:#}"))?;
    if !errors.is_empty() {
        return Err(format!("ESPHome 目录策略效果执行失败：{errors:?}"));
    }
    let bytes = host
        .kv()
        .get(&plugin_id, &format!("{RESULT_PREFIX}:{req_id}"))
        .ok_or_else(|| {
            "ESPHome 目录策略插件未返回结果（需要 kv 键 esphome-result:<req_id>）".to_string()
        })?;
    let result: CatalogResult = serde_json::from_slice(&bytes)
        .map_err(|e| format!("ESPHome 目录策略结果损坏（需要 route/entities）：{e}"))?;
    if let Some(route) = result.route.as_deref() {
        if route != "native" {
            return Err(format!("ESPHome 目录策略返回不支持的 route={route:?}"));
        }
    }
    let mut overrides = Vec::with_capacity(result.entities.len());
    for decision in result.entities {
        if decision.index >= ENTITIES.len() {
            return Err(format!(
                "ESPHome 目录策略索引越界：{}（实体数 {}）",
                decision.index,
                ENTITIES.len()
            ));
        }
        let name = match decision.name.as_deref() {
            None => None,
            Some(name) => {
                let trimmed = name.trim();
                if trimmed.is_empty() {
                    None
                } else if trimmed.len() > MAX_ESPHOME_NAME_BYTES {
                    return Err(format!(
                        "ESPHome 目录策略实体 {} 显示名超长（{} > {MAX_ESPHOME_NAME_BYTES} 字节）",
                        decision.index,
                        trimmed.len()
                    ));
                } else {
                    Some(trimmed.to_string())
                }
            }
        };
        overrides.push((decision.index, decision.exposed, name));
    }
    Ok(CatalogPlan::from_overrides(overrides))
}
