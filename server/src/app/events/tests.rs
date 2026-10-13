//! `server/src/app/events.rs` 的单元测试：`TurnTrace` 的时间口径与 first-only 语义
//!（QA Round 1 阻塞 1 的行为级回归——`mark_first_audio_at` 必须按**传入时刻**计
//! elapsed，而不是 mark 调用时刻）。

use std::time::{Duration, Instant};

use super::{TurnStages, TurnTrace, ANCHOR_INJECT, ANCHOR_SPEECH_END};

/// 阻塞 1 核心回归：`mark_first_audio_at` 记录的是**传入时刻相对锚点**的偏移——
/// 即使 mark 发生在首帧之后很久（生产路径：drain 返回、末帧已发完才调），
/// 记出来的值仍等于首帧真实偏移，而非"接近整轮耗时"。
#[test]
fn first_audio_uses_passed_instant_not_call_time() {
    let trace = TurnTrace::new(ANCHOR_SPEECH_END);
    let first_frame_at = trace_created_plus(trace.as_ref(), Duration::from_millis(300));
    // 模拟"整轮很长"：拖 1.2s 才走到 mark（生产上 drain 末帧返回后的时刻）
    std::thread::sleep(Duration::from_millis(1200));
    trace.mark_first_audio_at(Some(first_frame_at));
    let snap = trace.snapshot();
    let got = snap.first_audio_ms.expect("应已 mark");
    // 必须接近 300ms（首帧时刻），绝不能是 ~1500ms（调用时刻）
    assert!(
        (250..=500).contains(&got),
        "first_audio 应按传入时刻计（≈300ms），实测 {got}ms —— 若接近整轮耗时说明回退成调用时刻了"
    );
}

/// `None` 不 mark（未发出任何帧，如起跑闸即被打断的轮次）。
#[test]
fn first_audio_none_does_not_mark() {
    let trace = TurnTrace::new(ANCHOR_SPEECH_END);
    trace.mark_first_audio_at(None);
    assert!(trace.snapshot().first_audio_ms.is_none(), "None 不得 mark");
}

/// first-only：较早的时刻先 mark，更晚的重复 mark（甚至有值的 Some）不覆盖。
#[test]
fn first_audio_is_first_only() {
    let trace = TurnTrace::new(ANCHOR_SPEECH_END);
    let early = trace_created_plus(trace.as_ref(), Duration::from_millis(100));
    let late = trace_created_plus(trace.as_ref(), Duration::from_millis(900));
    trace.mark_first_audio_at(Some(early));
    trace.mark_first_audio_at(Some(late)); // 重复 mark：不得覆盖
    let got = trace.snapshot().first_audio_ms.unwrap();
    assert!(
        (80..=300).contains(&got),
        "first-only：应保留较早的 ~100ms，实测 {got}ms"
    );
}

/// 其余 mark 系列同样 first-only（None→Some 才写）。
#[test]
fn other_marks_are_first_only() {
    let trace = TurnTrace::new(ANCHOR_INJECT);
    trace.mark_llm_first_token();
    trace.mark_sentence_ready();
    let snap1 = trace.snapshot();
    let t1 = snap1.llm_first_token_ms.unwrap();
    std::thread::sleep(Duration::from_millis(50));
    trace.mark_llm_first_token(); // 重复：不覆盖
    let snap2 = trace.snapshot();
    assert_eq!(snap2.llm_first_token_ms, Some(t1), "重复 mark 不得覆盖");
    // 未 mark 的阶段保持 None
    assert!(snap2.asr_end_ms.is_none(), "注入轮没有 ASR 阶段");
    assert!(snap2.tts_first_pcm_ms.is_none());
    assert!(snap2.first_audio_ms.is_none());
}

/// 快照携带锚点（锚点语义随 TurnStages 带出，供订阅者分窗）。
#[test]
fn snapshot_carries_anchor() {
    let trace = TurnTrace::new(ANCHOR_SPEECH_END);
    assert_eq!(trace.snapshot().anchor, ANCHOR_SPEECH_END);
    let t2 = TurnTrace::new(ANCHOR_INJECT);
    assert_eq!(t2.snapshot().anchor, ANCHOR_INJECT);
}

/// 辅助：以 trace 的锚点为基准 + 偏移构造一个"过去的时刻"。
/// （TurnTrace::new 内部 t0 不可读，这里用「构造时刻 ≈ anchor 时刻」近似——
/// 紧跟构造调用，误差微秒级，测试断言区间已吸收。）
fn trace_created_plus(_trace: &TurnTrace, offset: Duration) -> Instant {
    Instant::now() + offset
}

// TurnStages::fresh 是私有构造，测试经 TurnTrace 快照取默认形状（覆盖 clone 语义）。
#[test]
fn turn_stages_is_cloneable_snapshot() {
    let trace = TurnTrace::new(ANCHOR_SPEECH_END);
    let a: TurnStages = trace.snapshot();
    let _b = a.clone(); // Clone 派生（emit 路径依赖）
    assert_eq!(a.anchor, ANCHOR_SPEECH_END);
}
