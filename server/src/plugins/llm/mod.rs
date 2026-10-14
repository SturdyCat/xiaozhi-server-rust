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
    /// 残句定时出句等待（毫秒，缺口③）：LLM 流距上一 token 静默 ≥ 此值且残句非空时，
    /// 先把残句送 TTS（TTFA 上界收紧；思考型停顿不再无限等标点或 50 字硬切）。
    ///
    /// 0 = 关闭（回退现行为）。默认 1500：比常见 inter-token 间隙（20~100ms、偶发
    /// 300~800ms）高一个量级，避免误切；normalize clamp 到 `[0, 60000]`。
    #[serde(default = "default_sentence_flush_ms")]
    pub sentence_flush_ms: u32,
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

/// `[llm].sentence_flush_ms` 默认 1500（缺口③：残句定时出句等待）。
fn default_sentence_flush_ms() -> u32 {
    1500
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
///
/// ## 取消契约（`on_event` 返回 `false` 时，**两条实现路径都必须遵守**）
///
/// 调用方在 `on_event` 里返回 `false` = **取消**（设备 abort 打断 / 承载断开）。
/// 引擎必须把"取消"如实反映在返回值上，否则下游无从区分"这一轮正常说完了"与
/// "这一轮被用户掐了"——历史上正是这里出的缺陷：
///
/// - [`Self::cancelled`] = `true`：**唯一**的取消信号。下游判断"这轮不算数"只看它。
/// - [`Self::tool_calls`] **恒为空**：取消前累积到的工具调用**一律丢弃**。
///   工具调用的下游是 MCP（`tools/call` → 控制设备音量/播放音乐/开关灯），是**不可回滚的
///   外部副作用**；用户已经停止说话，设备就不该再执行上一轮的指令。
///   这条约束由**引擎层**保证，而不是靠每个调用方"记得先查 abort"——
///   阶段三的 MCP 闭环跑在独立 task 上，与会话的 `abort` 标志没有共享关系。
/// - 不发 [`LlmEvent::Done`]（流没正常走完）。
/// - **仍然返回 [`Self::usage`] 与已累积的 [`Self::text`]**：令牌已经花了，统计不能丢；
///   半句文本也要用于观测（"被打断的这一轮同样发生过"）。二者都**不是**有效意图。
/// - 取消**不是**故障，因此**不**归入 [`LlmFailure`]、**不**触发 [`RetryingLlm`] 重试
///   （重试一条已被用户放弃的请求＝白烧 token）。
///
/// 未知事件类型（非流式回退路径）同样适用：调用方已经不想要的响应，不能靠返回值交付副作用。
#[derive(Debug, Clone, Default)]
pub struct LlmTurnResult {
    pub text: String,
    /// 阶段三（MCP 工具闭环）消费；当前聚合路径仅用 `text`。
    /// ⚠️ 取消轮**恒为空**（见类型文档的取消契约）——执行前无需再查 `abort`。
    #[allow(dead_code)]
    pub tool_calls: Vec<ToolCall>,
    /// 本轮令牌用量（上游未返回时为 `None`）。用于回答"记忆/灵魂注入让 prompt 长了多少"。
    /// 取消轮**照常上报**（钱已经花了）。
    pub usage: Option<TokenUsage>,
    /// 本轮是否被调用方取消（`on_event` 返回过 `false`）。`false` = 正常走完。
    pub cancelled: bool,
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
    /// 并返回已累积的部分文本（**不视为错误**），且必须遵守
    /// [`LlmTurnResult`] 的**取消契约**——尤其是 `tool_calls` 恒为空、
    /// `cancelled = true`、不发 [`LlmEvent::Done`]。
    ///
    /// 取消**不得**用 [`LlmFailure`] 表达（用户主动打断不是故障，见
    /// [`failure::retryable`]：`Cancelled` 一旦可重试就是白烧 token）。
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
            sentence_flush_ms: default_sentence_flush_ms(),
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
        // clamp 到 [0, 60000]：0 = 关闭；上限防异常配置把 watchdog 周期拖到分钟级。
        self.sentence_flush_ms = self.sentence_flush_ms.min(60_000);
    }
}
