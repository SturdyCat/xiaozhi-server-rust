//! LLM 失败的**具名分类**与令牌用量口径。
//!
//! ## 为什么要分类（而不是拼字符串）
//!
//! 现状（`openai.rs`）把状态码拼进错误字符串：`bail!("LLM API 返回错误 {status}: {body}")`。
//! 调用方（重试器、会话降级、UI 上报）**无法据此判断该不该重试**，只能把所有错误一视同仁
//! ——要么全都重试（语音场景会白等），要么全都不重试（一次 502 就"AI 没反应"）。
//!
//! 这里的纪律借自 DSH `dsh-llm-retry`：*"route on this, never by parsing `message`"*。
//! 分类信息**随错误值传递**（[`LlmFailure`] 实现 `std::error::Error`，可 `downcast_ref`），
//! 不做字符串匹配。
//!
//! ## 与语音场景的取舍
//!
//! DSH 的重试上限是 5 次、退避 500ms→10s——那是**编辑器/代理**场景（用户愿意等）。
//! 语音助手必须在秒级给出反馈，因此：[`MAX_RETRIES`] 只给 2 次、总预算 [`RETRY_BUDGET`]
//! 8 秒，且**已经吐出过文本就绝不重试**（用户已开始听到前半句，重来会"说两遍"）。

use std::fmt;
use std::time::Duration;

/// 单次请求的重试上限（不含首次；=2 表示最多 3 次尝试）。
pub const MAX_RETRIES: u32 = 2;
/// 重试总预算：超过即放弃并走会话兜底（语音场景不能在"等 AI"上耗太久）。
pub const RETRY_BUDGET: Duration = Duration::from_secs(8);
/// 首次退避（指数：400ms → 800ms；带少量抖动，避免多设备同时重试打同一上游）。
pub const RETRY_BASE_DELAY: Duration = Duration::from_millis(400);

/// 第 `attempt` 次重试（1-based）的退避时长。
pub fn retry_delay(attempt: u32) -> Duration {
    let exp = RETRY_BASE_DELAY * 2u32.saturating_pow(attempt.saturating_sub(1));
    // 抖动 0~25%（无需引 rand：用纳秒低位做一个廉价的伪随机）
    let jitter_ns = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0) as u64;
    let jitter = exp.as_millis() as u64 * (jitter_ns % 250) / 1000;
    exp + Duration::from_millis(jitter)
}

/// LLM 调用的失败分类。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LlmFailure {
    /// 连接失败（DNS/TCP/TLS）：通常是网络或地址写错。
    Connect(String),
    /// 超时（建连超时 / 流空闲超时）：上游没有在预期时间内给出任何东西。
    Timeout(String),
    /// 传输中断（已建立连接但读流失败）。
    Transport(String),
    /// HTTP 非 2xx。
    Status { code: u16, body: String },
    /// 上游 200 但**没有任何文本也没有工具调用**（真·空回复）。
    EmptyResponse,
    /// 协议/解析错误（SSE 解析失败、响应 JSON 结构不符）。
    Protocol(String),
}

impl LlmFailure {
    /// 是否值得重试。分类依据（对齐 DSH 的可重试集合 `[EMPTY_RESPONSE, RATE_LIMIT,
    /// SERVER, TIMEOUT, TRANSPORT]`）：**客户端错误（4xx，除 429）不重试**。
    pub fn retryable(&self) -> bool {
        match self {
            LlmFailure::Connect(_) | LlmFailure::Timeout(_) | LlmFailure::Transport(_) => true,
            LlmFailure::EmptyResponse => true,
            LlmFailure::Status { code, .. } => *code == 429 || (500..600).contains(code),
            LlmFailure::Protocol(_) => false,
        }
    }

    /// 稳定分类字符串（日志/UI/测试断言用；**不要**改语义，改名需同步测试）。
    pub fn category(&self) -> &'static str {
        match self {
            LlmFailure::Connect(_) => "connect",
            LlmFailure::Timeout(_) => "timeout",
            LlmFailure::Transport(_) => "transport",
            LlmFailure::Status { code, .. } if *code == 429 => "rate_limit",
            LlmFailure::Status { code, .. } if (500..600).contains(code) => "server",
            LlmFailure::Status { .. } => "client",
            LlmFailure::EmptyResponse => "empty_response",
            LlmFailure::Protocol(_) => "protocol",
        }
    }

    /// 面向用户/日志的一句话（含「接下来会怎样」，而不是复述错误码）。
    pub fn hint(&self) -> &'static str {
        match self {
            LlmFailure::Connect(_) => "连不上模型服务：检查 [llm].api_base 与网络（本机能否访问该地址）",
            LlmFailure::Timeout(_) => "模型服务响应超时：上游拥塞或地址不可达",
            LlmFailure::Transport(_) => "读模型响应时连接中断：网络不稳定",
            LlmFailure::Status { code: 429, .. } => "被上游限流（429）：稍后重试或换更低频的模型",
            LlmFailure::Status { code, .. } if *code == 401 || *code == 403 => {
                "上游拒绝鉴权：检查 [llm].api_key 是否有效"
            }
            LlmFailure::Status { code: 404, .. } => {
                "上游 404：api_base 必须是 **Responses** 端点，且 model 名要存在"
            }
            LlmFailure::Status { code, .. } if *code >= 500 => "上游服务端错误（5xx）：通常是临时的",
            LlmFailure::Status { .. } => "上游拒绝了请求（4xx）：检查 model/参数/额度",
            LlmFailure::EmptyResponse => "上游返回了空回复（既无文本也无工具调用）",
            LlmFailure::Protocol(_) => "响应格式不符合预期（Responses 协议解析失败）",
        }
    }

    /// 从 `reqwest` 错误归类（连接/超时/传输）。
    pub fn from_reqwest(e: &reqwest::Error) -> Self {
        let msg = e.to_string();
        if e.is_timeout() {
            LlmFailure::Timeout(msg)
        } else if e.is_connect() {
            LlmFailure::Connect(msg)
        } else {
            LlmFailure::Transport(msg)
        }
    }

    /// 从任意 `anyhow::Error` 里取回分类（重试器用）：先看是否本身就是 [`LlmFailure`]，
    /// 再看底层是否 reqwest 错误；都没有则按 `Protocol` 处理（保守：不重试）。
    pub fn classify(e: &anyhow::Error) -> Self {
        if let Some(f) = e.downcast_ref::<LlmFailure>() {
            return f.clone();
        }
        if let Some(r) = e.downcast_ref::<reqwest::Error>() {
            return LlmFailure::from_reqwest(r);
        }
        LlmFailure::Protocol(format!("{e:#}"))
    }
}

impl fmt::Display for LlmFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LlmFailure::Connect(m) => write!(f, "连接 LLM 失败: {m}"),
            LlmFailure::Timeout(m) => write!(f, "LLM 请求超时: {m}"),
            LlmFailure::Transport(m) => write!(f, "LLM 传输中断: {m}"),
            LlmFailure::Status { code, body } => {
                write!(f, "LLM API 返回错误 {code}: {body}")
            }
            LlmFailure::EmptyResponse => write!(f, "LLM 返回空回复"),
            LlmFailure::Protocol(m) => write!(f, "LLM 响应协议错误: {m}"),
        }
    }
}

impl std::error::Error for LlmFailure {}

/// 令牌用量。
///
/// **四个桶**（对齐 DSH `dsh-token-meter`）：`input`（未命中缓存的输入）、`output`、
/// `cache_read`（命中缓存读）、`cache_write`（写入缓存）。
/// **上下文压力口径** = `input + cache_read + cache_write`（**不含 output**）——
/// 它回答的是"这一轮把多少上下文推给了模型"，正是评估"记忆/灵魂注入让 prompt 变长了多少"
/// 需要的数字（output 只影响成本与时长）。
///
/// 只报**令牌数，不做金额换算**：单价随模型/渠道/时段变化，内置价目表必然过期
/// （DSH 全包也没有单价字段）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TokenUsage {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
}

impl TokenUsage {
    /// 上下文压力（见类型文档）。
    pub fn pressure(&self) -> u64 {
        self.input + self.cache_read + self.cache_write
    }

    pub fn total(&self) -> u64 {
        self.pressure() + self.output
    }

    /// 是否完全没有用量（测试与"上游没给 usage"判定用）。
    #[allow(dead_code)]
    pub fn is_zero(&self) -> bool {
        self.total() == 0
    }

    /// 累加（会话级多轮汇总）。
    pub fn add(&mut self, other: &TokenUsage) {
        self.input += other.input;
        self.output += other.output;
        self.cache_read += other.cache_read;
        self.cache_write += other.cache_write;
    }

    /// 解析 Responses 的 `usage` 对象（缺字段一律按 0；不因上游小改动而失败）。
    pub fn from_responses_usage(v: &serde_json::Value) -> TokenUsage {
        let n = |k: &str| v.get(k).and_then(|x| x.as_u64()).unwrap_or(0);
        // 缓存分桶在不同实现里位置不一：Responses 用 input_tokens_details.cached_tokens
        let cached = v
            .get("input_tokens_details")
            .and_then(|d| d.get("cached_tokens"))
            .and_then(|x| x.as_u64())
            .unwrap_or(0);
        let cache_write = v
            .get("input_tokens_details")
            .and_then(|d| d.get("cache_write_tokens"))
            .and_then(|x| x.as_u64())
            .unwrap_or(0);
        let input_total = n("input_tokens");
        TokenUsage {
            // 未命中缓存的输入 = 总输入 - 命中缓存读（避免与 cache_read 重复计数）
            input: input_total.saturating_sub(cached),
            output: n("output_tokens"),
            cache_read: cached,
            cache_write,
        }
    }

    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "input": self.input,
            "output": self.output,
            "cache_read": self.cache_read,
            "cache_write": self.cache_write,
            "pressure": self.pressure(),
            "total": self.total(),
        })
    }
}

#[cfg(test)]
mod tests;
