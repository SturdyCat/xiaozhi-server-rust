//! `server/src/app/metrics.rs` 的单元测试。
//!
//! 覆盖 `/api/metrics` 快照形状（缺口②验收）：
//! ① latency 节各阶段字段齐全、分位单调；② 老字段（turns/tts/errors）逐一不变、
//! 只增不改；③ TurnLatency 各阶段独立入窗（None 不进窗）；④ emit 无订阅者不 panic。
//!
//! 另覆盖**轮次级可归因（turn_id）**的聚合语义：
//! ⑤ 一轮的 5 个分段只算**一个**样本/窗口（`stages_of_one_turn_count_as_one_sample_per_window`）；
//! ⑥ `turn_id` 不参与任何计数（`turn_id_does_not_affect_aggregation`）；
//! ⑦ 混入 canned/aiui 轮时语音轮的分位数不被污染（`mixed_anchors_do_not_inflate_samples`）。

use super::{Metrics, TURN_WINDOW};
use crate::app::events::{ServerEvent, TurnId, TurnStages, ANCHOR_SPEECH_END};

fn stages(
    asr_end_ms: Option<u64>,
    llm_first_token_ms: Option<u64>,
    sentence_ready_ms: Option<u64>,
    tts_first_pcm_ms: Option<u64>,
    first_audio_ms: Option<u64>,
) -> TurnStages {
    TurnStages {
        anchor: ANCHOR_SPEECH_END,
        asr_end_ms,
        llm_first_token_ms,
        sentence_ready_ms,
        tts_first_pcm_ms,
        first_audio_ms,
    }
}

/// latency 节形状：五个阶段各含 p50/p90/p95/p99/max/samples，且分位单调不减。
#[test]
fn latency_section_shape_and_quantiles_monotonic() {
    let m = Metrics::new();
    for i in 1..=20u64 {
        m.observe(&ServerEvent::TurnLatency {
            session_id: "s".into(),
            device_id: "d".into(),
            turn_id: i,
            stages: stages(
                Some(i * 10),
                Some(i * 20),
                Some(i * 30),
                Some(i * 40),
                Some(i * 50),
            ),
            interrupted: false,
        });
    }
    let j = m.snapshot_json();
    let lat = j.get("latency").expect("应有 latency 节");
    for key in [
        "asr_end_ms",
        "llm_first_token_ms",
        "sentence_ready_ms",
        "tts_first_pcm_ms",
        "first_audio_ms",
    ] {
        let n = lat
            .get(key)
            .unwrap_or_else(|| panic!("latency.{key} 应存在"));
        for q in ["p50_ms", "p90_ms", "p95_ms", "p99_ms", "max_ms", "samples"] {
            assert!(n.get(q).is_some(), "latency.{key}.{q} 应存在");
        }
        // 分位单调：p50 ≤ p90 ≤ p95 ≤ p99 ≤ max
        let v = |q: &str| n[q].as_u64().expect("分位应为非负整数");
        assert!(v("p50_ms") <= v("p90_ms"), "{key} p50 ≤ p90");
        assert!(v("p90_ms") <= v("p95_ms"), "{key} p90 ≤ p95");
        assert!(v("p95_ms") <= v("p99_ms"), "{key} p95 ≤ p99");
        assert!(v("p99_ms") <= v("max_ms"), "{key} p99 ≤ max");
        assert_eq!(v("samples"), 20, "{key} 样本数应为 20");
    }
    // 分位取值抽查：asr 样本 10..=200，p50（向上取整位 = 第 10 个）= 100
    assert_eq!(lat["asr_end_ms"]["p50_ms"], 100);
    assert_eq!(lat["asr_end_ms"]["max_ms"], 200);
}

/// None 阶段不进窗（缓存命中轮的 tts_first_pcm 不拉低其余轮次的分位）。
#[test]
fn none_stages_do_not_enter_windows() {
    let m = Metrics::new();
    // 第一轮：缓存命中（tts_first_pcm = None）
    m.observe(&ServerEvent::TurnLatency {
        session_id: "s".into(),
        device_id: "d".into(),
        turn_id: 1,
        stages: stages(Some(50), Some(60), Some(70), None, Some(80)),
        interrupted: false,
    });
    // 第二轮：正常合成
    m.observe(&ServerEvent::TurnLatency {
        session_id: "s".into(),
        device_id: "d".into(),
        turn_id: 2,
        stages: stages(Some(150), Some(160), Some(170), Some(180), Some(190)),
        interrupted: false,
    });
    let j = m.snapshot_json();
    let lat = &j["latency"];
    assert_eq!(lat["asr_end_ms"]["samples"], 2, "asr 两轮都有");
    assert_eq!(
        lat["tts_first_pcm_ms"]["samples"], 1,
        "缓存命中轮不进 tts_first_pcm 窗"
    );
    assert_eq!(lat["tts_first_pcm_ms"]["max_ms"], 180, "只含正常轮的值");
}

/// 老字段不变（additive 纪律）：新增 TurnLatency 消费不改变既有计数语义。
#[test]
fn legacy_fields_unchanged_by_turn_latency() {
    let m = Metrics::new();
    m.observe(&ServerEvent::SessionStart {
        session_id: "s".into(),
        device_id: "d".into(),
        client_id: "c".into(),
        is_test: false,
    });
    m.observe(&ServerEvent::MessageAdded {
        session_id: "s".into(),
        device_id: "d".into(),
        turn_id: Some(7),
        role: "assistant".into(),
        text: "你好".into(),
        elapsed_ms: Some(1234),
    });
    m.observe(&ServerEvent::TtsRound {
        session_id: "s".into(),
        device_id: "d".into(),
        turn_id: 7,
        sentences: 2,
        frames: 40,
        interrupted: true,
        audio_ms: 2400,
    });
    m.observe(&ServerEvent::Error {
        session_id: "s".into(),
        device_id: "d".into(),
        message: "boom".into(),
    });
    // 这些老事件之外再叠加若干 TurnLatency
    for i in 0..5u64 {
        m.observe(&ServerEvent::TurnLatency {
            session_id: "s".into(),
            device_id: "d".into(),
            turn_id: 100 + i,
            stages: stages(Some(1), Some(2), Some(3), Some(4), Some(5)),
            interrupted: false,
        });
    }
    let j = m.snapshot_json();
    // turns 节的老字段原样保留（elapsed_p90/p99 是本批新增档位）
    assert_eq!(j["turns"]["total"], 1);
    assert_eq!(j["turns"]["elapsed_p50_ms"], 1234);
    assert_eq!(j["turns"]["elapsed_p95_ms"], 1234);
    assert_eq!(j["turns"]["elapsed_max_ms"], 1234);
    assert_eq!(j["turns"]["samples"], 1);
    assert_eq!(j["sessions"]["started"], 1);
    assert_eq!(j["tts"]["rounds"], 1);
    assert_eq!(j["tts"]["frames"], 40);
    assert_eq!(j["tts"]["audio_ms"], 2400);
    assert_eq!(j["tts"]["interrupted"], 1);
    assert_eq!(j["errors"]["count"], 1);
    assert_eq!(j["errors"]["last"], "boom");
    // 新增档位也生效
    assert_eq!(j["turns"]["elapsed_p90_ms"], 1234);
    assert_eq!(j["turns"]["elapsed_p99_ms"], 1234);
}

/// 窗口截断：超过 [`TURN_WINDOW`] 轮丢最旧（样本数有上界）。
#[test]
fn stage_windows_are_bounded() {
    let m = Metrics::new();
    for i in 0..(TURN_WINDOW as u64 + 50) {
        m.observe(&ServerEvent::TurnLatency {
            session_id: "s".into(),
            device_id: "d".into(),
            turn_id: i + 1,
            stages: stages(Some(1), None, None, None, None),
            interrupted: false,
        });
    }
    let j = m.snapshot_json();
    assert_eq!(j["latency"]["asr_end_ms"]["samples"], TURN_WINDOW);
    assert_eq!(
        j["latency"]["first_audio_ms"]["samples"], 0,
        "未发生阶段的窗口保持空"
    );
    assert_eq!(j["latency"]["first_audio_ms"]["p50_ms"], 0, "空窗分位为 0");
}

/// 无订阅者 emit 不 panic（既有语义保持；本测试构造的就是无订阅者场景）。
#[test]
fn observe_without_subscribers_is_safe() {
    let m = Metrics::new();
    m.observe(&ServerEvent::TurnLatency {
        session_id: "s".into(),
        device_id: "d".into(),
        turn_id: 1,
        stages: stages(Some(1), Some(2), Some(3), Some(4), Some(5)),
        interrupted: true,
    });
    assert_eq!(m.snapshot_json()["latency"]["asr_end_ms"]["samples"], 1);
}

/// QA Round 1 建议 5（分窗口径）：仅 speech_end / inject 轮入窗——canned（兜底/问候/
/// 告别，常走缓存零合成）与 aiui（云端闭环）不与语音轮混窗（否则拉低语音轮分位）。
#[test]
fn only_speech_and_inject_anchors_enter_latency_windows() {
    use crate::app::events::{ANCHOR_AIUI, ANCHOR_CANNED, ANCHOR_INJECT};

    let m = Metrics::new();
    let mk = |anchor: &'static str, turn_id: TurnId, v: u64| ServerEvent::TurnLatency {
        session_id: "s".into(),
        device_id: "d".into(),
        turn_id,
        stages: TurnStages {
            anchor,
            asr_end_ms: Some(v),
            llm_first_token_ms: Some(v),
            sentence_ready_ms: Some(v),
            tts_first_pcm_ms: Some(v),
            first_audio_ms: Some(v),
        },
        interrupted: false,
    };
    // 语音轮（基准值 100）
    m.observe(&mk(ANCHOR_SPEECH_END, 1, 100));
    // 注入轮（值 200，应入窗）
    m.observe(&mk(ANCHOR_INJECT, 2, 200));
    // canned / aiui 轮（值 9999，**不得**入窗）
    m.observe(&mk(ANCHOR_CANNED, 3, 9999));
    m.observe(&mk(ANCHOR_AIUI, 4, 9999));

    let j = m.snapshot_json();
    let asr = &j["latency"]["asr_end_ms"];
    assert_eq!(asr["samples"], 2, "仅 speech_end + inject 两轮入窗");
    assert_eq!(
        asr["max_ms"], 200,
        "canned/aiui 的 9999 不得混入（否则 max 失真）"
    );
    assert_eq!(asr["p50_ms"], 100);
    // note 文案写明口径（前端展示直接用）
    let note = j["latency"]["note"].as_str().unwrap_or_default();
    assert!(
        note.contains("speech_end") && note.contains("inject"),
        "note 应写明入窗锚点口径: {note}"
    );
    assert!(
        note.contains("canned") || note.contains("不入窗"),
        "note 应说明排除口径: {note}"
    );
}

/// ## 关键回归：一轮的 5 个分段只算**一个**样本（不是 5 个）
///
/// 直接读聚合结果（`snapshot_json`）断言，不只测"事件发出去了"：
/// 走**完整的一轮事件序列**（用户消息 + 助手消息 + TurnLatency 带 5 段 + TtsRound）
/// 重复 N 轮，断言：
/// - `turns.total == N`（一轮 = 一轮，不是 5 轮）；
/// - **每个**阶段窗口 `samples == N`——一轮共推 5 个样本，但分属 5 个窗口，
///   每窗口只 +1。若哪天有人改成"一个事件往每窗口推 5 次"或"按分段累加 turns"，
///   `samples` 会变成 5N / `turns.total` 变成 5N，本测试即红。
#[test]
fn stages_of_one_turn_count_as_one_sample_per_window() {
    const N: u64 = 20;
    let m = Metrics::new();
    for i in 1..=N {
        let turn = i; // 一轮一个 id
                      // 用户消息：与助手同轮（不参与轮次计数，见 observe 的注释）
        m.observe(&ServerEvent::MessageAdded {
            session_id: "s".into(),
            device_id: "d".into(),
            turn_id: Some(turn),
            role: "user".into(),
            text: "问".into(),
            elapsed_ms: None,
        });
        // 助手整轮回复：轮次计数与整窗耗时的唯一来源
        m.observe(&ServerEvent::MessageAdded {
            session_id: "s".into(),
            device_id: "d".into(),
            turn_id: Some(turn),
            role: "assistant".into(),
            text: "答".into(),
            elapsed_ms: Some(1000 + i),
        });
        // 一条 TurnLatency 带满 5 个阶段
        m.observe(&ServerEvent::TurnLatency {
            session_id: "s".into(),
            device_id: "d".into(),
            turn_id: turn,
            stages: stages(
                Some(i * 10),
                Some(i * 20),
                Some(i * 30),
                Some(i * 40),
                Some(i * 50),
            ),
            interrupted: false,
        });
        m.observe(&ServerEvent::TtsRound {
            session_id: "s".into(),
            device_id: "d".into(),
            turn_id: turn,
            sentences: 2,
            frames: 40,
            interrupted: false,
            audio_ms: 2400,
        });
    }
    let j = m.snapshot_json();
    assert_eq!(j["turns"]["total"], N, "N 轮就是 N 轮（用户消息不计轮）");
    assert_eq!(j["turns"]["samples"], N, "整轮耗时窗按轮次计");
    assert_eq!(j["tts"]["rounds"], N, "TtsRound 一轮一条");
    for key in [
        "asr_end_ms",
        "llm_first_token_ms",
        "sentence_ready_ms",
        "tts_first_pcm_ms",
        "first_audio_ms",
    ] {
        assert_eq!(
            j["latency"][key]["samples"],
            N,
            "latency.{key}: 一轮只进 1 个样本（一轮共 5 段但分属 5 个窗口），\
             若为 {} 说明按分段重复计数了",
            N * 5
        );
    }
    // 分位取值仍只由这 N 个样本决定（未受同一轮自身重复加权）
    assert_eq!(j["latency"]["asr_end_ms"]["p50_ms"], 10 * N.div_ceil(2));
    assert_eq!(j["latency"]["asr_end_ms"]["max_ms"], N * 10);
}

/// `turn_id` 是**纯诊断字段**：同一批阶段事件，id 取不同值时聚合结果必须逐字节相同。
///
/// 这条钉住"加 turn_id 不改变任何既有口径"——它是本任务对 `/api/metrics` 的
/// **零投影变更**承诺的可执行证据。
#[test]
fn turn_id_does_not_affect_aggregation() {
    let snap_with = |base: TurnId| {
        let m = Metrics::new();
        for k in 0..5u64 {
            m.observe(&ServerEvent::TurnLatency {
                session_id: "s".into(),
                device_id: "d".into(),
                turn_id: base * 1000 + k,
                stages: stages(Some(k * 10), Some(k * 20), None, Some(k * 40), Some(k * 50)),
                interrupted: false,
            });
            m.observe(&ServerEvent::MessageAdded {
                session_id: "s".into(),
                device_id: "d".into(),
                turn_id: Some(base * 1000 + k),
                role: "assistant".into(),
                text: "答".into(),
                elapsed_ms: Some(500 + k),
            });
        }
        m.snapshot_json()
    };
    assert_eq!(
        snap_with(1),
        snap_with(9_999_991),
        "turn_id 取值不得影响任何聚合结果（uptime 除外，本例同刻构造）"
    );
}

/// 语音轮之间夹着 canned/aiui 轮：样本数只按语音/注入轮增长，且不污染分位。
///
/// 这是"一轮 = 一个样本"在**混排**下的表现：canned/aiui 既不进阶段窗口，
/// 也不该让语音轮的样本数虚增。
#[test]
fn mixed_anchors_do_not_inflate_samples() {
    use crate::app::events::{ANCHOR_AIUI, ANCHOR_CANNED};

    let m = Metrics::new();
    let mut turn = 0u64;
    for i in 1..=10u64 {
        turn += 1;
        m.observe(&ServerEvent::TurnLatency {
            session_id: "s".into(),
            device_id: "d".into(),
            turn_id: turn,
            stages: stages(
                Some(i * 10),
                Some(i * 20),
                Some(i * 30),
                Some(i * 40),
                Some(i * 50),
            ),
            interrupted: false,
        });
        // 每轮语音之间夹一条问候语（canned）与一条 AIUI 轮（值 9999，绝不能进窗）
        for (anchor, bad) in [(ANCHOR_CANNED, 9999u64), (ANCHOR_AIUI, 9999)] {
            turn += 1;
            m.observe(&ServerEvent::TurnLatency {
                session_id: "s".into(),
                device_id: "d".into(),
                turn_id: turn,
                stages: TurnStages {
                    anchor,
                    asr_end_ms: Some(bad),
                    llm_first_token_ms: Some(bad),
                    sentence_ready_ms: Some(bad),
                    tts_first_pcm_ms: Some(bad),
                    first_audio_ms: Some(bad),
                },
                interrupted: false,
            });
        }
    }
    let j = m.snapshot_json();
    // 逐阶段核对：样本数只按 10 个语音轮增长，且各阶段 max 只可能来自语音轮的
    // `i * 系数`（最大 10 × 50 = 500），绝不可能是被排除的 canned/aiui 的 9999。
    for (key, factor) in [
        ("asr_end_ms", 10u64),
        ("llm_first_token_ms", 20),
        ("sentence_ready_ms", 30),
        ("tts_first_pcm_ms", 40),
        ("first_audio_ms", 50),
    ] {
        assert_eq!(
            j["latency"][key]["samples"], 10,
            "latency.{key} 只按 10 个语音轮计"
        );
        assert_eq!(
            j["latency"][key]["max_ms"],
            10 * factor,
            "latency.{key}: canned/aiui 的 9999 不得混入"
        );
    }
    // turns 只由 assistant MessageAdded 驱动：本测试没发助手消息，故为 0
    assert_eq!(j["turns"]["total"], 0, "没有助手消息就没有轮次");
}
