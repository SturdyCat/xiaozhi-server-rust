//! 远程 LLM 调用（OpenAI 兼容 `chat/completions`）。
//!
//! - [`LlmClient`]：真实 HTTP 调用（OpenAI 兼容接口）。
//! - [`MockLlm`]：无模型回显实现，用于**零配置本地联调**（无需真实 LLM 接口，
//!   默认 `llm.backend = "mock"` 时启用，保证 `cargo run` 即可端到端跑通）。
//!
//! 多轮历史由 [`crate::session`] 维护；本模块只负责单次 `chat` 请求。

use crate::config::LlmConfig;
use anyhow::{Context, Result};
use serde_json::{json, Value};

#[derive(Clone)]
pub struct LlmClient {
    client: reqwest::Client,
    cfg: LlmConfig,
}

impl LlmClient {
    pub fn new(cfg: &LlmConfig) -> Self {
        Self {
            client: reqwest::Client::new(),
            cfg: cfg.clone(),
        }
    }

    /// 发起一次多轮对话请求。
    ///
    /// - `history`：`(role, content)` 列表（不含 system，不含当前用户轮）。
    /// - `user_text`：当前轮用户文本。
    /// - 返回：助手回复文本。
    pub async fn chat(&self, history: &[(String, String)], user_text: &str) -> Result<String> {
        let mut messages = Vec::with_capacity(history.len() + 2);
        messages.push(json!({ "role": "system", "content": self.cfg.system_prompt }));
        for (role, content) in history {
            messages.push(json!({ "role": role, "content": content }));
        }
        messages.push(json!({ "role": "user", "content": user_text }));

        let resp = self
            .client
            .post(&self.cfg.api_base)
            .bearer_auth(&self.cfg.api_key)
            .json(&json!({
                "model": self.cfg.model,
                "messages": messages,
                "temperature": self.cfg.temperature,
                "stream": false,
            }))
            .send()
            .await
            .context("调用 LLM API 失败（网络或 URL 错误）")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("LLM API 返回错误 {status}: {body}");
        }

        let v: Value = resp.json().await.context("解析 LLM 响应 JSON 失败")?;
        let text = v["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or("")
            .to_string();
        Ok(text)
    }
}

/// 无模型回显实现，用于本地零配置联调。
pub struct MockLlm {
    reply: String,
}

impl MockLlm {
    pub fn new() -> Self {
        Self {
            reply: "这是本地 mock 语音助手的回复。".to_string(),
        }
    }

    /// 返回固定文案，并回显用户文本，便于联调时确认 ASR 链路。
    pub async fn chat(&self, _history: &[(String, String)], user_text: &str) -> Result<String> {
        if user_text.trim().is_empty() {
            Ok(self.reply.clone())
        } else {
            Ok(format!("{}（你说：{}）", self.reply, user_text))
        }
    }
}

/// LLM 引擎枚举：真实 HTTP 或本地 mock。
pub enum Llm {
    Http(LlmClient),
    Mock(MockLlm),
}

impl Llm {
    pub async fn chat(&self, history: &[(String, String)], user_text: &str) -> Result<String> {
        match self {
            Llm::Http(c) => c.chat(history, user_text).await,
            Llm::Mock(m) => m.chat(history, user_text).await,
        }
    }
}

/// 依据配置构造 LLM 引擎：`backend = "mock"` 用本地回显，否则真实 HTTP。
pub fn build_llm(cfg: &LlmConfig) -> Llm {
    if cfg.backend.eq_ignore_ascii_case("mock") {
        return Llm::Mock(MockLlm::new());
    }
    Llm::Http(LlmClient::new(cfg))
}
