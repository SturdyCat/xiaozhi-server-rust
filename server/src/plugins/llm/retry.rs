//! LLM 重试装饰器：把"什么时候能重试"从会话编排里抽出来，套在**任何** [`LlmProvider`]
//! 外面（装饰器模式，provider 自己不重试——对齐 DSH "服务不重试、执行器重试"的分工）。
//!
//! ## 三条语音场景纪律（与编辑器场景的关键差异）
//!
//! 1. **有界且短**：最多 [`MAX_RETRIES`] 次、总预算 [`RETRY_BUDGET`]——用户不会等 10 秒
//!    （DSH 是 5 次/500ms→10s，那是 agent 场景；照抄会毁掉语音体验）。
//! 2. **已吐出文本就不重试**：调用方收到 `LlmEvent::Text` 时很可能已经在下行音频了，
//!    重试会让用户"听两遍前半句"。判据用**是否向调用方交付过文本**，而不是时间。
//! 3. **只重试瞬时失败**：判定走 [`LlmFailure::retryable`]（具名分类），
//!    不做字符串匹配——4xx（除 429）重试只是浪费预算。

use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use anyhow::Result;

use super::failure::{retry_delay, LlmFailure, MAX_RETRIES, RETRY_BUDGET};
use super::{Llm, LlmEvent, LlmProvider, LlmTurnResult, ToolSpec, TurnPrompt};

/// 重试装饰器：包住真实 provider。
pub struct RetryingLlm {
    inner: Llm,
}

impl RetryingLlm {
    pub fn new(inner: Llm) -> Self {
        Self { inner }
    }
}

impl LlmProvider for RetryingLlm {
    fn chat_stream<'a>(
        &'a self,
        history: &'a [(String, String)],
        user_text: &'a str,
        prompt: &'a TurnPrompt,
        tools: Option<&'a [ToolSpec]>,
        on_event: Box<dyn FnMut(LlmEvent) -> bool + Send + 'a>,
    ) -> Pin<Box<dyn Future<Output = Result<LlmTurnResult>> + Send + 'a>> {
        Box::pin(async move {
            let started = Instant::now();
            // 回调需跨尝试复用：放进 Arc<Mutex<..>>；每次尝试包一层"转发闭包"，
            // 顺带记录本次尝试是否向调用方交付过文本（决定还能不能重试）。
            let cb = Arc::new(Mutex::new(on_event));
            let mut attempt: u32 = 0;
            loop {
                let delivered = Arc::new(AtomicBool::new(false));
                let wrapper: Box<dyn FnMut(LlmEvent) -> bool + Send + 'a> = {
                    let cb = cb.clone();
                    let delivered = delivered.clone();
                    Box::new(move |event: LlmEvent| {
                        if matches!(event, LlmEvent::Text(_)) {
                            delivered.store(true, Ordering::Relaxed);
                        }
                        let mut guard = cb.lock().unwrap_or_else(|e| e.into_inner());
                        guard(event)
                    })
                };

                match self
                    .inner
                    .chat_stream(history, user_text, prompt, tools, wrapper)
                    .await
                {
                    Ok(out) => return Ok(out),
                    Err(e) => {
                        let failure = LlmFailure::classify(&e);
                        let delivered_text = delivered.load(Ordering::Relaxed);
                        let can_retry = failure.retryable() && !delivered_text && attempt < MAX_RETRIES;
                        if !can_retry {
                            if delivered_text && failure.retryable() {
                                tracing::warn!(
                                    "LLM 失败（{}）但已向调用方交付文本，不再重试（避免重复播报）",
                                    failure.category()
                                );
                            } else if !failure.retryable() {
                                tracing::warn!(
                                    "LLM 失败（{}）：不可重试，直接返回。{}",
                                    failure.category(),
                                    failure.hint()
                                );
                            }
                            return Err(e);
                        }
                        attempt += 1;
                        let delay = retry_delay(attempt);
                        if started.elapsed() + delay > RETRY_BUDGET {
                            tracing::warn!(
                                "LLM 重试预算用尽（已耗时 {:?}，本次失败 {}）：放弃重试。{}",
                                started.elapsed(),
                                failure.category(),
                                failure.hint()
                            );
                            return Err(e);
                        }
                        tracing::warn!(
                            "LLM 失败（{}），{:?} 后进行第 {attempt}/{MAX_RETRIES} 次重试。{}",
                            failure.category(),
                            delay,
                            failure.hint()
                        );
                        tokio::time::sleep(delay).await;
                    }
                }
            }
        })
    }
}

#[cfg(test)]
mod tests;
