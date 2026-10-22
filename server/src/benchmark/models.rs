//! Existing /llm/models request/result fields retained; bounded, isolated saved-config proxy.
use std::time::{Duration, Instant};

#[derive(serde::Serialize)]
pub struct ModelsResult {
    pub state: &'static str,
    pub models: Vec<String>,
    pub elapsed_ms: u64,
    pub error: Option<String>,
    pub request_id: String,
    pub action_id: &'static str,
    pub source: &'static str,
    pub revision: Option<String>,
    pub error_code: Option<&'static str>,
    pub readiness: &'static str,
}
impl ModelsResult {
    fn error(code: &'static str, text: &str) -> Self {
        Self {
            state: "error",
            models: vec![],
            elapsed_ms: 0,
            error: Some(text.into()),
            request_id: uuid::Uuid::new_v4().to_string(),
            action_id: "llm.models",
            source: "saved",
            revision: None,
            error_code: Some(code),
            readiness: "configured",
        }
    }
}
fn models_url(api_base: &str) -> String {
    let b = api_base.trim().trim_end_matches('/');
    if let Some(v) = b.strip_suffix("/responses") {
        format!("{v}/models")
    } else if b.ends_with("/models") {
        b.into()
    } else {
        format!("{b}/models")
    }
}
pub(super) async fn from_service(
    service: &super::TestService,
    api_base: Option<&str>,
    api_key: Option<&str>,
) -> ModelsResult {
    let t = Instant::now();
    if api_base.is_some_and(|v| v.len() > 4096) || api_key.is_some_and(|v| v.len() > 4096) {
        return ModelsResult::error("invalid_input", "Override exceeds 4096 bytes");
    }
    let Ok(slot) = service.limit.clone().try_acquire_owned() else {
        return ModelsResult::error("busy", "Two diagnostics are already running");
    };
    let slot = std::sync::Arc::new(slot);
    let saved = match super::snapshot(
        service.store.clone(),
        service.read_limit.clone(),
        Some(slot.clone()),
    )
    .await
    {
        Ok(v) => v,
        Err(_) => {
            return ModelsResult::error("snapshot_unavailable", "Cannot load saved configuration")
        }
    };
    let mut cfg = saved.cfg.llm;
    if let Some(v) = api_base.filter(|v| !v.trim().is_empty()) {
        cfg.api_base = v.into();
    }
    if let Some(v) = api_key.filter(|v| !v.trim().is_empty()) {
        cfg.api_key = v.into();
    }
    let mut out = ModelsResult::error(
        "provider_error",
        "Model catalogue request failed; check upstream and saved credentials",
    );
    out.revision = Some(saved.revision);
    if api_base.is_some() || api_key.is_some() {
        out.source = "saved+request";
    }
    let result = async {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(15))
            .build()?;
        let mut req = client.get(models_url(&cfg.api_base));
        if !cfg.api_key.trim().is_empty() {
            req = req.bearer_auth(cfg.api_key.trim());
        }
        let mut response = req.send().await?;
        if !response.status().is_success() {
            anyhow::bail!("HTTP failure");
        }
        let mut bytes = vec![];
        while let Some(chunk) = response.chunk().await? {
            if bytes.len() + chunk.len() > 256 * 1024 {
                anyhow::bail!("Response too large");
            }
            bytes.extend_from_slice(&chunk);
        }
        let v: serde_json::Value = serde_json::from_slice(&bytes)?;
        let items = v
            .get("data")
            .or_else(|| v.get("models"))
            .and_then(|v| v.as_array())
            .ok_or_else(|| anyhow::anyhow!("Missing model array"))?;
        if items.len() > 1000 {
            anyhow::bail!("Too many models");
        }
        let mut models: Vec<String> = items
            .iter()
            .filter_map(|item| {
                item.as_str()
                    .or_else(|| item.get("id").and_then(|v| v.as_str()))
                    .or_else(|| item.get("name").and_then(|v| v.as_str()))
            })
            .filter(|v| !v.is_empty() && v.len() <= 256)
            .map(str::to_owned)
            .collect();
        models.sort();
        models.dedup();
        Ok::<_, anyhow::Error>(models)
    }
    .await;
    if let Ok(models) = result {
        out.state = "ok";
        out.models = models;
        out.error = None;
        out.error_code = None;
        out.readiness = "provider_verified";
    }
    out.elapsed_ms = t.elapsed().as_millis() as u64;
    out
}
