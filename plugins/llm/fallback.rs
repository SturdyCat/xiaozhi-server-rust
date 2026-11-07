//! LLM 主备切换装饰器：主用失败后按序尝试备用端点（`[llm.fallback]`）。
//!
//! 机制对齐 **hermes-agent** 的 fallback providers（NousResearch/hermes-agent，
//! `website/docs/user-guide/features/fallback-providers.md`），按本仓库语音场景裁剪：
//!
//! - **触发面按 [`LlmFailure`] 具名分类路由**（绝不解析错误文案）：限流 429 / 5xx /
//!   连接 / 超时 / 空回复在主用**重试预算耗尽后**切换（`RetryingLlm` 已把预算耗尽
//!   表达为最终 `Err`，本装饰器只看最终结果）；鉴权/配额 401/402/403 **立即**切换
//!   （重试主用不会好，白等）。参数类 4xx（model 写错/请求非法）换备用也救不了，不切。
//! - **冷藏（bench）**：限流/鉴权/配额是"短期不自愈"的失败——若上游声明了
//!   `Retry-After`，主用冷藏**到恢复时刻**（步数清零）；没声明则指数退避
//!   60s → 4h（hermes 同参数）。冷藏期内每轮直接从备用开始，不再撞一次主用
//!   （省一次失败延迟）。其余失败（5xx/断连等瞬时类）**不冷藏**：按轮作用域，
//!   下一轮先回主用（hermes 的 turn-scoped 语义）。
//! - [`LlmFallbackConfig::min_switch_reset_seconds`]：限流即将恢复（窗口比它短）时
//!   不切备用，本轮直接失败走会话兜底——用户显式配置了"宁等主用"的取舍才生效。
//! - **已吐文本绝不切换**：调用方收到 `Text` 后很可能已在下行音频，换 provider 重说
//!   会"听两遍前半句"。判据与 `retry.rs` 相同：是否向调用方交付过文本。
//! - **取消透传**：`Ok{cancelled}` 不是故障，直接返回，不可能触发切换。
//! - 备用链**不套重试装饰器**（见 `build_llm`）：主用已耗尽 8s 预算，语音不能在
//!   备用上再来一轮退避；备用逐个试一次，全失败则返回最后一个错误走会话兜底。

use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::Result;

use super::failure::LlmFailure;
use super::{Llm, LlmEvent, LlmProvider, LlmTurnResult, ToolSpec, TurnPrompt};

/// 主用冷藏的指数退避基数（上游未声明 `Retry-After` 时的第一步）。
/// hermes 同参数：60s 起步。
const BENCH_BASE: Duration = Duration::from_secs(60);
/// 主用冷藏上界（hermes：4h）。到点后自然回主用探测。
const BENCH_MAX: Duration = Duration::from_secs(4 * 3600);

/// 主备切换装饰器：`chain[0]` = 主用（已包 `RetryingLlm`），其余为备用（裸 provider）。
pub struct FallbackLlm {
    chain: Vec<Llm>,
    min_switch_reset: Duration,
    /// 主用冷藏状态（跨轮共享）。
    bench: Mutex<Bench>,
}

#[derive(Default)]
struct Bench {
    /// 主用冷藏到期时刻（`None` = 未冷藏，正常先试主用）。
    until: Option<Instant>,
    /// 无 `Retry-After` 时的指数退避步数（连续失败时翻倍，成功服务后清零）。
    step: u32,
}

impl FallbackLlm {
    pub fn new(chain: Vec<Llm>, min_switch_reset: Duration) -> Self {
        Self {
            chain,
            min_switch_reset,
            bench: Mutex::new(Bench::default()),
        }
    }
}

impl LlmProvider for FallbackLlm {
    fn chat_stream<'a>(
        &'a self,
        history: &'a [(String, String)],
        user_text: &'a str,
        prompt: &'a TurnPrompt,
        tools: Option<&'a [ToolSpec]>,
        on_event: Box<dyn FnMut(LlmEvent) -> bool + Send + 'a>,
    ) -> Pin<Box<dyn Future<Output = Result<LlmTurnResult>> + Send + 'a>> {
        Box::pin(async move {
            // 回调跨 provider 复用 + 记录"是否已交付文本"（交付后禁止切换）。
            let cb = Arc::new(Mutex::new(on_event));
            let delivered = Arc::new(AtomicBool::new(false));
            let total = self.chain.len();

            for (i, provider) in self.chain.iter().enumerate() {
                // 主用仍在冷藏期：本轮直接从备用开始，不再撞主用（省一次失败延迟）。
                if i == 0 {
                    let benched = self
                        .bench
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .until
                        .is_some_and(|t| Instant::now() < t);
                    if benched {
                        tracing::info!("主用 LLM 冷藏中（限流/配额未恢复），本轮直接使用备用");
                        continue;
                    }
                }

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

                match provider
                    .chat_stream(history, user_text, prompt, tools, wrapper)
                    .await
                {
                    // `Ok` 一律透传（含取消轮，见 `LlmTurnResult` 取消契约）。
                    // 主用恢复服务 → 清冷藏、退避步数归零（下次失败从 60s 重新起步）。
                    Ok(out) => {
                        if i == 0 {
                            let mut b = self.bench.lock().unwrap_or_else(|e| e.into_inner());
                            b.until = None;
                            b.step = 0;
                        } else {
                            tracing::info!("本轮由备用 LLM #{i} 服务（主用不可用）");
                        }
                        return Ok(out);
                    }
                    Err(e) => {
                        let failure = LlmFailure::classify(&e);
                        // 已出声再切 = 换个声音把前半句再说一遍，绝对禁止。
                        if delivered.load(Ordering::Relaxed) {
                            tracing::warn!(
                                "LLM 失败（{}）但已向调用方交付文本，不切备用（避免重复播报）",
                                failure.category()
                            );
                            return Err(e);
                        }
                        if !failure.switchable() {
                            return Err(e);
                        }
                        if i == 0 && failure.persistent() {
                            // 用户显式配置了"宁等主用"：限流马上就恢复（窗口 <
                            // min_switch_reset）时不切备用，本轮失败走会话兜底。
                            // 默认 0 = 恒切换（`d < 0` 恒假）。
                            if let Some(d) = failure.retry_after() {
                                if d < self.min_switch_reset {
                                    tracing::warn!(
                                        "主用限流将在 {d:?} 内恢复（< {:?}），本轮不切备用",
                                        self.min_switch_reset
                                    );
                                    return Err(e);
                                }
                            }
                            self.bench_primary(&failure);
                        }
                        if i + 1 == total {
                            tracing::warn!(
                                "主用与全部备用（{} 条）均失败（最后失败：{}）：本轮走会话兜底。{}",
                                total - 1,
                                failure.category(),
                                failure.hint()
                            );
                            return Err(e);
                        }
                        tracing::warn!(
                            "LLM 失败（{}），切换备用 #{}。{}",
                            failure.category(),
                            i + 1,
                            failure.hint()
                        );
                    }
                }
            }
            // chain 构造时保证非空，循环必经 return；此处不可达。
            unreachable!("FallbackLlm chain 不为空")
        })
    }
}

impl FallbackLlm {
    /// 主用失败后决定冷藏时长（仅 [`LlmFailure::persistent`] 类失败调用）：
    /// 有 `Retry-After` → 冷藏到恢复时刻（步数清零，下次仍以声明为准）；
    /// 无 → 指数退避 60s → 4h（hermes 同参数）。限流窗口短于 `min_switch_reset`
    /// 的情况在调用前已拦截（不切不冷藏）。
    fn bench_primary(&self, failure: &LlmFailure) {
        let mut b = self.bench.lock().unwrap_or_else(|e| e.into_inner());
        let until = match failure.retry_after() {
            Some(d) => {
                b.step = 0;
                Instant::now() + d
            }
            None => {
                let d = BENCH_BASE
                    .saturating_mul(2u32.saturating_pow(b.step))
                    .min(BENCH_MAX);
                b.step += 1;
                Instant::now() + d
            }
        };
        b.until = Some(until);
        tracing::warn!(
            "主用 LLM 失败（{}），冷藏至 {:?} 后自动回主用探测",
            failure.category(),
            until
        );
    }
}

#[cfg(test)]
mod tests;
