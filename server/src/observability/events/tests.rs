//! `server/src/app/events.rs` 的单元测试：`TurnTrace` 的时间口径与 first-only 语义
//!（QA Round 1 阻塞 1 的行为级回归——`mark_first_audio_at` 必须按**传入时刻**计
//! elapsed，而不是 mark 调用时刻），以及轮次标识 `turn_id` 的生成/一致性契约。

use std::sync::Arc;
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

/// 轮次 id 单调递增且互不相同（进程内唯一）。
///
/// 断言用「严格递增」而不是具体数值：计数器是**进程级**的，本文件里的其他测试也会
/// 分配 id，具体值随并行执行顺序而变——但递增关系在任何顺序下都成立。
#[test]
fn turn_ids_are_strictly_increasing_and_distinct() {
    let ids: Vec<u64> = (0..8)
        .map(|_| TurnTrace::new(ANCHOR_SPEECH_END).turn_id())
        .collect();
    for w in ids.windows(2) {
        assert!(
            w[1] > w[0],
            "轮次 id 必须严格递增（可排序性是选 u64 自增的理由之一），实测 {} → {}",
            w[0],
            w[1]
        );
    }
    // 唯一性：8 个 id 两两不同（严格递增已蕴含，这里显式钉住意图）
    let mut sorted = ids.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(sorted.len(), ids.len(), "轮次 id 必须互不相同");
}

/// id 从 1 起（0 留作「无轮次」哨兵，日志里出现 `turn=0` 一眼可辨为异常）。
#[test]
fn turn_id_never_zero() {
    for _ in 0..64 {
        assert_ne!(TurnTrace::new(ANCHOR_SPEECH_END).turn_id(), 0, "0 是哨兵值");
    }
}

/// 轮次内一致：同一个 trace 无论 mark 多少次、快照取几次，id 恒定；
/// 不同锚点（= 不同轮次）拿到不同 id。
#[test]
fn turn_id_is_stable_within_a_turn() {
    let trace = TurnTrace::new(ANCHOR_SPEECH_END);
    let id = trace.turn_id();
    // 反复读取 + 夹着 mark 读取：id 必须不变（否则一轮的日志会散成多个 turn=）
    for _ in 0..3 {
        assert_eq!(trace.turn_id(), id, "id 在轮内必须恒定");
        trace.mark_asr_end();
        trace.mark_llm_first_token();
        let _ = trace.snapshot();
    }
    assert_eq!(trace.turn_id(), id);
    // 克隆共享同一个轮次（producer/预取/发送端各自持一份 Arc）
    let cloned: Arc<TurnTrace> = Arc::clone(&trace);
    assert_eq!(cloned.turn_id(), id, "Arc 克隆不得改变轮次");
    assert_eq!(cloned.turn_id(), trace.turn_id());
}

/// mark 返回「是否首次写入」——这是首 token / 首句日志的判据（与 first-only 同源）。
#[test]
fn mark_reports_first_write_only_once() {
    let t = TurnTrace::new(ANCHOR_SPEECH_END);
    assert!(t.mark_asr_end(), "首次 mark 必须返回 true");
    assert!(!t.mark_asr_end(), "重复 mark 必须返回 false");
    assert!(t.mark_llm_first_token());
    assert!(!t.mark_llm_first_token());
    assert!(t.mark_sentence_ready());
    assert!(!t.mark_sentence_ready());
    assert!(t.mark_tts_first_pcm());
    assert!(!t.mark_tts_first_pcm());
    // `mark_first_audio_at(None)` 不算一次写入（未发出任何帧）
    assert!(!t.mark_first_audio_at(None), "None 不该被当成首次写入");
    assert!(t.mark_first_audio_at(Some(Instant::now())));
    assert!(
        !t.mark_first_audio_at(Some(Instant::now())),
        "重复 mark 返回 false"
    );
}

/// 轮次摘要：已发生的阶段带毫秒值，未发生的显示 `-`（不能读成 0）。
///
/// 这行摘要就是 `grep turn=N` 之后回答「慢在哪一段」的那一行，故钉住其形状。
#[test]
fn summary_marks_missing_stages_with_dash() {
    let trace = TurnTrace::new(ANCHOR_SPEECH_END);
    let empty = trace.snapshot().summary();
    for stage in ["asr=", "llm首字=", "首句=", "首pcm=", "首帧="] {
        assert!(empty.contains(stage), "摘要应含阶段 {stage}，实测 {empty}");
    }
    assert_eq!(
        empty.matches('-').count(),
        5,
        "未发生的 5 个阶段都该是 -：{empty}"
    );

    trace.mark_asr_end();
    std::thread::sleep(Duration::from_millis(15));
    trace.mark_llm_first_token();
    let got = trace.snapshot().summary();
    assert_eq!(
        got.matches('-').count(),
        3,
        "已 mark 的 2 个阶段不再显示 -：{got}"
    );
    assert!(
        got.contains("llm首字=1") || got.contains("llm首字=2"),
        "已发生的阶段应带实际毫秒值：{got}"
    );
}
