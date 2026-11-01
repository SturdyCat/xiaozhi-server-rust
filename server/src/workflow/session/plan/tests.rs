//! Behavior regressions run real Session hooks with model-free dependencies.
use super::*;
use crate::config::Config;
use crate::plugins::memory::*;
use crate::workflow::protocol::{IncomingFrame, Transport};
use crate::workflow::session::{Session, SessionParams};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct Wire {
    sent: Vec<String>,
    fail: bool,
}
impl Transport for Wire {
    async fn send_text(&mut self, text: String) -> anyhow::Result<()> {
        anyhow::ensure!(!self.fail, "injected transport failure");
        self.sent.push(text);
        Ok(())
    }
    async fn send_binary(&mut self, _: Vec<u8>) -> anyhow::Result<()> {
        Ok(())
    }
    async fn recv(&mut self) -> IncomingFrame {
        IncomingFrame::Closed
    }
    fn try_recv(&mut self) -> Option<IncomingFrame> {
        None
    }
    async fn keepalive(&mut self) {}
}

#[derive(Default)]
struct Memory {
    recalls: AtomicUsize,
    records: Mutex<Vec<CompletedTurn>>,
    extracts: AtomicUsize,
    events: Mutex<Vec<&'static str>>,
    fail_recall: bool,
    fail_record: bool,
}
impl MemoryProvider for Memory {
    fn name(&self) -> &'static str {
        "test"
    }
    fn recall<'a>(&'a self, _: Option<&'a str>, _: &'a str) -> BoxFut<'a, RecallResult> {
        Box::pin(async move {
            self.recalls.fetch_add(1, Ordering::Relaxed);
            self.events.lock().unwrap().push("recall");
            RecallResult {
                block: Some("real recalled context".into()),
                error: self.fail_recall.then(|| "recall failed".into()),
                ..Default::default()
            }
        })
    }
    fn record<'a>(&'a self, turn: CompletedTurn) -> BoxFut<'a, anyhow::Result<()>> {
        Box::pin(async move {
            anyhow::ensure!(!self.fail_record, "record failed");
            self.records.lock().unwrap().push(turn);
            Ok(())
        })
    }
    fn extract_pending(&self, _: usize) -> BoxFut<'_, ExtractOutcome> {
        Box::pin(async move {
            self.extracts.fetch_add(1, Ordering::Relaxed);
            ExtractOutcome::default()
        })
    }
    fn maintain(&self, _: MaintainOpts) -> BoxFut<'_, anyhow::Result<MaintainOutcome>> {
        Box::pin(async { Ok(MaintainOutcome::skipped()) })
    }
    fn stats(&self) -> MemoryStats {
        MemoryStats::default()
    }
    fn clear(&self, _: Option<String>) -> BoxFut<'_, anyhow::Result<u64>> {
        Box::pin(async { Ok(0) })
    }
}

fn binding(plugin: &str, position: &str, order: i32, policy: &str) -> Value {
    json!({"id":plugin,"plugin_id":plugin,"position_id":position,"order":order,"enabled":true,"on_error":policy,"condition":{"kind":"always"}})
}
fn config(bindings: Vec<Value>) -> Config {
    let mut cfg = Config::default();
    cfg.workflow = json!({"version":1,"bindings":bindings});
    cfg.command.reply.clear();
    cfg.memory.db_path = std::env::temp_dir()
        .join("xz-runtime-tests-unused.db")
        .to_string_lossy()
        .into();
    cfg
}
fn session<'a>(wire: &'a mut Wire, cfg: Config, memory: Arc<Memory>) -> Session<'a, Wire> {
    let store = crate::config::store::sqlite::SqliteConfigStore::open(":memory:").unwrap();
    let engines = crate::runtime::Engines::new_dev(&cfg, store).unwrap();
    let params = SessionParams::configured_defaults(&cfg, crate::media::media::AudioCodec::Opus);
    let mut no_command = cfg.command.clone();
    no_command.enabled = false;
    let mut no_nlu = cfg.nlu.clone();
    no_nlu.enabled = false;
    Session {
        transport: wire,
        engines: engines.clone(),
        params,
        session_id: "runtime-test".into(),
        device_id: "device-test".into(),
        workflow: crate::workflow::designer::compile(&cfg.workflow).unwrap(),
        skills: SkillRegistry::new(&cfg.command, &cfg.nlu, None),
        command_skills: SkillRegistry::new(&cfg.command, &no_nlu, None),
        nlu_skills: SkillRegistry::new(&no_command, &cfg.nlu, None),
        skills_bert: None,
        wake: crate::plugins::wakeup::WakeGreeting::new(&cfg.wakeup),
        eff_config: Arc::new(cfg),
        tts_lease: engines.tts_lease(),
        memory,
        memory_completed_through: None,
        bounded_turn: false,
        pending_segments: vec![],
        history: vec![],
        downlink_ts: 0,
        test_recording: false,
        test_buf: vec![],
        vad: None,
        device_vad: false,
        uplink_decoder: None,
        real_audio: false,
        is_test: false,
        abort: Arc::new(AtomicBool::new(false)),
        aiui: None,
        pacer: crate::workflow::downlink::DownlinkPacer::new(0),
        close_requested: false,
        mcp_enabled: false,
        mcp_next_id: 1,
        inject_rx: None,
    }
}

#[test]
fn missing_context_fails_closed_including_confidence() {
    let missing = HookContext {
        text: None,
        device_kind: None,
        confidence: None,
    };
    for condition in [
        Condition::TextContains {
            text: "".into(),
            case_sensitive: false,
        },
        Condition::DeviceKind {
            value: "xiaozhi_esp".into(),
        },
        Condition::Confidence {
            min: 0.0,
            max: None,
        },
    ] {
        assert!(!matches(&condition, &missing));
    }
    let ctx = HookContext {
        text: Some("Hello WORLD"),
        device_kind: Some("xiaozhi_esp"),
        confidence: Some(0.8),
    };
    assert!(matches(
        &Condition::TextContains {
            text: "world".into(),
            case_sensitive: false
        },
        &ctx
    ));
    assert!(!matches(
        &Condition::TextContains {
            text: "world".into(),
            case_sensitive: true
        },
        &ctx
    ));
    assert!(matches(
        &Condition::Confidence {
            min: 0.7,
            max: Some(0.8)
        },
        &ctx
    ));
}

#[tokio::test]
async fn empty_plan_disables_enhancements_and_llm_skill_escape_hatch() {
    let mut wire = Wire::default();
    let memory = Arc::new(Memory::default());
    let mut cfg = config(vec![]);
    cfg.llm.system_prompt = "plain core prompt".into();
    let mut s = session(&mut wire, cfg, memory.clone());
    assert!(!s.handle_skill_gate("退下").await);
    assert!(s.turn_skills("退下").tools().is_empty());
    let prompt = s.build_turn_prompt("你好").await.unwrap();
    assert_eq!(prompt.instructions, "plain core prompt");
    assert!(prompt.memory.is_none());
    s.run_wake_hooks(Some("你好")).await.unwrap();
    s.complete_memory_write(1, "u", "a").await.unwrap();
    assert_eq!(memory.recalls.load(Ordering::Relaxed), 0);
    assert!(memory.records.lock().unwrap().is_empty());
    assert!(s.transport.sent.is_empty());
}

#[tokio::test]
async fn command_condition_and_enabled_control_actual_side_effects() {
    for mode in 0..4 {
        let mut b = binding("command.gate", "link.asr.nlu", 0, "stop");
        if mode == 0 {
            b["enabled"] = json!(false);
        }
        if mode == 1 {
            b["condition"] = json!({"kind":"text_contains","text":"different"});
        }
        if mode == 2 {
            b["condition"] = json!({"kind":"confidence","min":0.0});
        }
        let mut wire = Wire::default();
        let mut s = session(&mut wire, config(vec![b]), Arc::new(Memory::default()));
        assert_eq!(s.handle_skill_gate("退下").await, mode == 3);
        assert_eq!(s.close_requested, mode == 3);
        assert_eq!(s.turn_skills("退下").active(), mode == 3);
    }
}

#[tokio::test]
async fn command_stage_precedes_nlu_regardless_of_cross_stage_order_values() {
    for nlu_first in [true, false] {
        let mut cfg = config(vec![
            binding(
                "command.gate",
                "link.asr.nlu",
                if nlu_first { 9 } else { 0 },
                "stop",
            ),
            binding("nlu.rules", "nlu", if nlu_first { 0 } else { 9 }, "stop"),
        ]);
        cfg.nlu.intents.push(crate::plugins::nlu::IntentDef {
            name: "override".into(),
            templates: vec!["退下".into()],
            action: crate::plugins::nlu::ActionDef {
                kind: "reply".into(),
                reply: "".into(),
            },
        });
        let mut wire = Wire::default();
        let mut s = session(&mut wire, cfg, Arc::new(Memory::default()));
        assert!(s.handle_skill_gate("退下").await);
        assert_eq!(
            s.close_requested, true,
            "command is an earlier stage; cross-stage order cannot move NLU ahead"
        );
    }
}

#[tokio::test]
async fn prompt_hook_order_and_stop_continue_are_real() {
    for stop in [true, false] {
        for soul_first in [true, false] {
            let mut cfg = config(vec![
                binding(
                    "soul.profile",
                    "link.nlu.llm",
                    if soul_first { 0 } else { 9 },
                    if stop { "stop" } else { "continue" },
                ),
                binding(
                    "memory.recall",
                    "link.nlu.llm",
                    if soul_first { 9 } else { 0 },
                    "continue",
                ),
            ]);
            cfg.soul.preset = "does-not-exist".into();
            cfg.llm.system_prompt = "fallback".into();
            let memory = Arc::new(Memory::default());
            let mut wire = Wire::default();
            let mut s = session(&mut wire, cfg, memory.clone());
            let result = s.build_turn_prompt("question").await;
            assert_eq!(result.is_err(), stop);
            assert_eq!(
                memory.recalls.load(Ordering::Relaxed),
                usize::from(!stop || !soul_first)
            );
            if !stop {
                assert_eq!(result.unwrap().instructions, "fallback");
            }
        }
    }
}

#[tokio::test]
async fn device_condition_controls_recall_and_writeback() {
    for kind in ["xiaozhi_esp", "echomuse"] {
        let mut recall = binding("memory.recall", "link.nlu.llm", 0, "continue");
        recall["condition"] = json!({"kind":"device_kind","value":kind});
        let mut write = binding("memory.writeback", "end", 0, "continue");
        write["condition"] = recall["condition"].clone();
        let mut wire = Wire::default();
        let memory = Arc::new(Memory::default());
        let mut s = session(&mut wire, config(vec![recall, write]), memory.clone());
        let prompt = s.build_turn_prompt("question").await.unwrap();
        s.complete_memory_write(1, "question", "answer")
            .await
            .unwrap();
        assert_eq!(prompt.memory.is_some(), kind == "xiaozhi_esp");
        assert_eq!(
            memory.records.lock().unwrap().len(),
            usize::from(kind == "xiaozhi_esp")
        );
    }
}

#[tokio::test]
async fn recall_and_record_failure_obey_policy_without_background_extract() {
    for stop in [false, true] {
        let policy = if stop { "stop" } else { "continue" };
        let memory = Arc::new(Memory {
            fail_recall: true,
            fail_record: true,
            ..Default::default()
        });
        let mut wire = Wire::default();
        let mut s = session(
            &mut wire,
            config(vec![
                binding("memory.recall", "link.nlu.llm", 0, policy),
                binding("memory.writeback", "end", 0, policy),
            ]),
            memory.clone(),
        );
        assert_eq!(s.build_turn_prompt("u").await.is_err(), stop);
        assert_eq!(s.complete_memory_write(1, "u", "a").await.is_err(), stop);
        tokio::task::yield_now().await;
        assert_eq!(memory.extracts.load(Ordering::Relaxed), 0);
    }
}

#[tokio::test]
async fn wake_hook_failure_is_not_swallowed_and_only_detect_triggers() {
    for stop in [true, false] {
        let mut wire = Wire {
            fail: true,
            ..Default::default()
        };
        let mut s = session(
            &mut wire,
            config(vec![binding(
                "wakeup.greeting",
                "start",
                0,
                if stop { "stop" } else { "continue" },
            )]),
            Arc::new(Memory::default()),
        );
        assert!(s
            .handle_text(r#"{"type":"listen","state":"start"}"#)
            .await
            .is_ok());
        assert_eq!(
            s.handle_text(r#"{"type":"listen","state":"detect","text":"wake"}"#)
                .await
                .is_err(),
            stop
        );
    }
}

#[tokio::test]
async fn old_session_retains_plan_when_source_config_changes() {
    let mut cfg = config(vec![binding("command.gate", "link.asr.nlu", 0, "stop")]);
    let mut first_wire = Wire::default();
    let mut old = session(&mut first_wire, cfg.clone(), Arc::new(Memory::default()));
    cfg.workflow = json!({"version":1,"bindings":[]});
    let mut second_wire = Wire::default();
    let mut new = session(&mut second_wire, cfg, Arc::new(Memory::default()));
    assert!(old.handle_skill_gate("退下").await);
    assert!(!new.handle_skill_gate("退下").await);
}

#[tokio::test]
async fn completed_writeback_records_once_and_extracts_off_path() {
    let mut wire = Wire::default();
    let memory = Arc::new(Memory::default());
    let mut s = session(
        &mut wire,
        config(vec![binding("memory.writeback", "end", 0, "stop")]),
        memory.clone(),
    );
    s.complete_memory_write(10, "user", "assistant")
        .await
        .unwrap();
    s.complete_memory_write(10, "user", "assistant")
        .await
        .unwrap();
    assert_eq!(memory.records.lock().unwrap().len(), 1);
    assert_eq!(
        memory.extracts.load(Ordering::Relaxed),
        0,
        "expensive work must be spawned, not awaited"
    );
    tokio::task::yield_now().await;
    assert_eq!(memory.extracts.load(Ordering::Relaxed), 1);
    s.complete_memory_write(11, "next user", "next assistant")
        .await
        .unwrap();
    assert_eq!(
        memory.records.lock().unwrap().len(),
        2,
        "new completed turn must still write"
    );
}

struct BrokenBert;
impl crate::plugins::nlu::BertClassify for BrokenBert {
    fn classify_timed(&self, _: &str) -> anyhow::Result<crate::plugins::nlu::BertHit> {
        anyhow::bail!("classifier failed")
    }
}
#[tokio::test]
async fn classifier_failure_stops_actual_session_without_llm_fallthrough() {
    let mut wire = Wire::default();
    let mut s = session(
        &mut wire,
        config(vec![binding("nlu.rules", "nlu", 0, "stop")]),
        Arc::new(Memory::default()),
    );
    let mut no_command = s.eff_config.command.clone();
    no_command.enabled = false;
    s.nlu_skills = SkillRegistry::new(&no_command, &s.eff_config.nlu, Some(Arc::new(BrokenBert)));
    assert!(
        s.handle_skill_gate("unrelated sentence").await,
        "error must consume this turn, not reach LLM"
    );
}

#[tokio::test]
async fn disabled_skill_cannot_be_restored_by_llm_tool_nomination() {
    let mut wire = Wire::default();
    let mut s = session(&mut wire, config(vec![]), Arc::new(Memory::default()));
    let call = crate::plugins::llm::ToolCall {
        id: "x".into(),
        name: "execute_skill".into(),
        arguments: json!({"skill":"end_session","slots":{}}).to_string(),
    };
    assert!(s.execute_llm_tool_calls(&[call], 1, "退下").await.is_err());
    assert!(!s.close_requested);
}
