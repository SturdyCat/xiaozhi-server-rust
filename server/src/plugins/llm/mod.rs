//! LLM 插件：`LlmProvider` trait + 工厂。第一个实现为
//! [`openai::LlmClient`]（OpenAI 兼容 Responses API，SSE 流式 + 原生工具调用）。
//!
//! 新增 LLM 后端（本地推理/其他厂商）：在 `plugins/llm/` 加实现文件、实现
//! [`LlmProvider`]、`build_llm` 加分发分支即可，会话编排不动。
//!
//! ## 护栏（借自 DSH 的 LLM 插件，见 `docs/plugin-architecture-unification.md` §4）
//!
//! - [`failure`]：具名失败分类（[`LlmFailure`]）+ 令牌用量口径（[`TokenUsage`]）。
//!   **按分类路由，不做字符串匹配**；`retryable()` 决定能否重试。
//! - [`retry`]：重试装饰器（有界、总预算 8s、**已吐出文本不重试**）。
//! - `openai`：建连/整请求/流空闲三层超时；错误一律以 [`LlmFailure`] 为根错误抛出。
//!
//! 与 ASR/TTS/VAD 不同，LLM 是**异步 HTTP 调用**（`reqwest` + `rustls`），
//! 不占用 tokio worker 计算线程，无需 `spawn_blocking` 隔离。
//! 多轮历史由调用方（[`crate::app::session`]）维护；本模块只负责单次请求。

use serde::{Deserialize, Serialize};

use crate::config::{canonical_engine, EngineSpec};

/// LLM 可选实现（`[llm].engine` 的权威表）。当前只有 OpenAI 兼容 Responses 一条路径。
pub const LLM_ENGINES: &[EngineSpec] = &[EngineSpec {
    id: "openai",
    aliases: &[],
}];
const DEFAULT_LLM_ENGINE: &str = "openai";

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LlmConfig {
    /// 实现 id（`[llm].engine`，约定见 `crate::config` 模块文档）；空 = 默认 `openai`。
    #[serde(default)]
    pub engine: String,
    /// OpenAI 兼容 Responses API 地址（如 `https://api.example.com/v1/responses`）。
    #[serde(default = "default_api_base")]
    pub api_base: String,
    #[serde(default)]
    pub api_key: String,
    #[serde(default = "default_llm_model")]
    pub model: String,
    #[serde(default = "default_system_prompt")]
    pub system_prompt: String,
    #[serde(default = "default_max_history")]
    pub max_history: usize,
    #[serde(default = "default_temperature")]
    pub temperature: f32,
    /// 是否流式（SSE）。默认 `true`：逐 token 返回，配合 session 按句下发降低首字延迟；
    /// `false` 退回整段（兼容不支持 SSE 的端点）。
    #[serde(default = "default_stream")]
    pub stream: bool,
}
fn default_api_base() -> String {
    "https://api.example.com/v1/responses".into()
}

fn default_llm_model() -> String {
    "gpt-4o".into()
}

fn default_system_prompt() -> String {
    "你是一个有用且简洁的中文语音助手。".into()
}

fn default_max_history() -> usize {
    10
}

fn default_temperature() -> f32 {
    0.7
}

fn default_stream() -> bool {
    true
}

pub mod failure;
mod openai;
mod retry;

pub use failure::TokenUsage;
// bin crate 内部暂无直接引用者（装饰器/实现走 `failure::` 路径）：作为插件对外 API 面保留
#[allow(unused_imports)]
pub use failure::LlmFailure;
pub use retry::RetryingLlm;

// bin crate 内部暂无直接引用者：作为插件对外 API 面保留
#[allow(unused_imports)]
pub use openai::LlmClient;

use anyhow::Result;
use std::future::Future;
use std::pin::Pin;
use serde_json::Value;
use std::sync::Arc;

pub use crate::plugins::prompt::TurnPrompt;

/// 设备/服务端可向 LLM 声明的工具规格（Responses 扁平结构 `tools[]` 项）。
#[derive(Debug, Clone)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    /// JSON Schema 对象（`parameters`），描述工具入参。
    pub parameters: Value,
}

/// 一次收集完整的工具调用（来自 LLM 的 `function_call` 项）。
///
/// `id` 对应 Responses 的 `call_id`（回填 `function_call_output.call_id` 用）。
/// 字段由阶段三（MCP 工具闭环）消费，当前仅 session 聚合路径不读。
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    /// 工具入参的原始 JSON 字符串（如 `{"location":"Beijing"}`）。
    pub arguments: String,
}

/// 流式过程中回传给调用方的事件。
/// `Text`/`ToolCall` 载荷由阶段二（按句下发）/阶段三（MCP 闭环）消费。
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub enum LlmEvent {
    /// 文本增量片段（`response.output_text.delta` 的 `delta`）。
    Text(String),
    /// 一个完整收集的工具调用（流结束后才回传）。
    ToolCall(ToolCall),
    /// 本轮流转结束。
    Done,
}

/// 一轮对话的汇总结果（已收齐全部文本与工具调用）。
#[derive(Debug, Clone, Default)]
pub struct LlmTurnResult {
    pub text: String,
    /// 阶段三（MCP 工具闭环）消费；当前聚合路径仅用 `text`。
    #[allow(dead_code)]
    pub tool_calls: Vec<ToolCall>,
    /// 本轮令牌用量（上游未返回时为 `None`）。用于回答"记忆/灵魂注入让 prompt 长了多少"。
    pub usage: Option<TokenUsage>,
}

/// LLM 提供方 trait：文本对话 + 可选工具规格，流式回传 [`LlmEvent`]。
/// 手动装箱 future（与 [`crate::app::transport::Transport`] 同风格，不引 async-trait）。
///
/// `prompt` 是**本轮组装好的上下文**（稳定 `instructions` + 可选记忆块，见
/// [`crate::plugins::prompt::TurnPrompt`]）：把"系统提示词怎么来"从 provider 里摘出去，
/// provider 只负责"把它发出去"。人格（[`crate::plugins::soul`]）与记忆
/// （[`crate::plugins::memory`]）因此可以在**不改 provider** 的前提下接入。
pub trait LlmProvider: Send + Sync {
    /// `on_event` 回调返回 `false` 表示**取消**（如设备 abort 打断）：实现应立即停止
    /// 并返回已累积的部分文本（不视为错误）。
    fn chat_stream<'a>(
        &'a self,
        history: &'a [(String, String)],
        user_text: &'a str,
        prompt: &'a TurnPrompt,
        tools: Option<&'a [ToolSpec]>,
        on_event: Box<dyn FnMut(LlmEvent) -> bool + Send + 'a>,
    ) -> Pin<Box<dyn Future<Output = Result<LlmTurnResult>> + Send + 'a>>;
}

/// LLM 引擎：`build_llm` 依据 `[llm]` 配置构造（api_key 为空时调用会在服务端报 401/403）。
pub type Llm = Arc<dyn LlmProvider>;

/// 依据配置构造 LLM 引擎。
///
/// 外层套 [`RetryingLlm`]：**provider 自己不重试**，重试/退避/预算统一由装饰器负责
/// （对齐 DSH "服务不重试、执行器重试"）；换 provider 时自动获得同样的护栏。
pub fn build_llm(cfg: &LlmConfig) -> Llm {
    let inner: Llm = Arc::new(openai::LlmClient::new(cfg));
    Arc::new(RetryingLlm::new(inner))
}

impl Default for LlmConfig {
    fn default() -> Self {
        LlmConfig {
            engine: String::new(), // 空 → engine_id() 取默认；normalize() 会补全
            api_base: default_api_base(),
            api_key: String::new(),
            model: default_llm_model(),
            system_prompt: default_system_prompt(),
            max_history: default_max_history(),
            temperature: default_temperature(),
            stream: default_stream(),
        }
    }
}

impl LlmConfig {
    /// 规范化后的实现 id（空 → 默认；未知值原样返回，由校验报错）。
    pub fn engine_id(&self) -> String {
        canonical_engine(&self.engine, LLM_ENGINES, DEFAULT_LLM_ENGINE)
    }
    pub fn normalize(&mut self) {
        self.engine = self.engine_id();
    }
}
