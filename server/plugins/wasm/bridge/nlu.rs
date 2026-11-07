//! P7g bounded NLU catalog policy. Recognition remains native/model-only.

use serde::Deserialize;
use serde_json::json;

use super::super::registry::WasmHost;
use super::super::Resolution;

const MAX_NLU_LOCALE_BYTES: usize = 32;
const MAX_NLU_CANDIDATES: usize = 32;
const MAX_NLU_ID_BYTES: usize = 128;
const MAX_NLU_LABEL_BYTES: usize = 256;

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct NluCatalogPrepareResult {
    #[serde(default)]
    pub indices: Vec<u32>,
    #[serde(default)]
    pub route: Option<String>,
}

/// Ask a WASM policy for bounded catalog presentation/filtering. The native
/// trained model remains the only intent recognizer; plugins never see the
/// utterance and cannot select or execute a skill.
pub fn nlu_catalog_prepare(
    host: &WasmHost,
    locale: &str,
    candidates: &[(String, String)],
    max_items: u32,
) -> std::result::Result<NluCatalogPrepareResult, String> {
    if locale.len() > MAX_NLU_LOCALE_BYTES {
        return Err("nlu catalog locale 超长".to_string());
    }
    if candidates.is_empty() || candidates.len() > MAX_NLU_CANDIDATES {
        return Err("nlu catalog candidates 为空或超限".to_string());
    }
    if !(1..=MAX_NLU_CANDIDATES as u32).contains(&max_items) {
        return Err("nlu catalog max_items 非法".to_string());
    }
    for (id, label) in candidates {
        if id.trim().is_empty()
            || id.len() > MAX_NLU_ID_BYTES
            || label.len() > MAX_NLU_LABEL_BYTES
            || id.chars().any(char::is_control)
            || label.chars().any(char::is_control)
        {
            return Err("nlu catalog candidate 超长、为空或含控制字符".to_string());
        }
    }
    let plugin_id = match host.resolve("nlu.catalog") {
        Resolution::Resolved(id) => id,
        other => return Err(format!("nlu catalog 扩展点无可用实现：{other:?}")),
    };
    let req_id = uuid::Uuid::new_v4().to_string();
    let payload = json!({
        "req_id": req_id,
        "locale": locale,
        "candidates": candidates.iter().map(|(id, label)| json!({ "id": id, "label": label })).collect::<Vec<_>>(),
        "max_items": max_items,
    })
    .to_string();
    let errors = host
        .handle_event(&plugin_id, "nlu.catalog.prepare", &payload)
        .map_err(|e| format!("nlu catalog 插件事件失败：{e:#}"))?;
    if !errors.is_empty() {
        return Err(format!("nlu catalog 效果执行失败：{errors:?}"));
    }
    let bytes = host
        .kv()
        .get(&plugin_id, &format!("nlu-catalog-result:{req_id}"))
        .ok_or_else(|| "nlu catalog 插件未返回结果".to_string())?;
    let result: NluCatalogPrepareResult =
        serde_json::from_slice(&bytes).map_err(|e| format!("nlu catalog 结果损坏：{e}"))?;
    if result.indices.len() > max_items as usize {
        return Err("nlu catalog 返回条目超过请求上限".to_string());
    }
    let mut seen = std::collections::HashSet::new();
    if result
        .indices
        .iter()
        .any(|index| (*index as usize) >= candidates.len() || !seen.insert(*index))
    {
        return Err("nlu catalog 返回索引越界或重复".to_string());
    }
    if result
        .route
        .as_deref()
        .is_some_and(|route| route != "native")
    {
        return Err("nlu catalog 返回不支持的 route".to_string());
    }
    Ok(result)
}
