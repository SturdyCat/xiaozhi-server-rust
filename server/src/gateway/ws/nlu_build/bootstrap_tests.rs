//! 首启自举的纯逻辑回归：状态机、退避、前置检查、冒烟解析与失败归因。
//!
//! 这里**不**跑 docker、不加载真实 ONNX（单测桩不冒充真实链路）：门禁的推理部分由
//! 真机验收覆盖，本文件锁的是"判定规则与状态迁移"——写错这些比写错推理更难发现。

use super::*;
use crate::plugins::nlu::NluConfig;

fn nlu_with_intents() -> NluConfig {
    let mut nlu = NluConfig::default();
    nlu.enabled = true;
    nlu
}

fn idle() -> State {
    State::default()
}

#[test]
fn mode_defaults_to_adopt_and_unknown_values_fail_closed() {
    assert_eq!(mode_from_env(None), Mode::Adopt);
    assert_eq!(mode_from_env(Some("")), Mode::Adopt);
    assert_eq!(mode_from_env(Some(" adopt ")), Mode::Adopt);
    assert_eq!(mode_from_env(Some("build-only")), Mode::BuildOnly);
    assert_eq!(mode_from_env(Some("off")), Mode::Off);
    // 拼错不能变成"自动启用"：一律按关闭处理。
    for raw in ["ON", "true", "yes", "0", "adoptt"] {
        assert_eq!(mode_from_env(Some(raw)), Mode::Off, "{raw}");
    }
}

#[test]
fn backoff_is_immediate_then_ten_minutes_then_one_hour() {
    // attempts = 已经失败过几次；返回值是"下一次还要等多久"。
    assert_eq!(backoff_secs(0), 0, "第一次尝试立即");
    assert_eq!(backoff_secs(1), 600, "失败 1 次后等 10min");
    assert_eq!(backoff_secs(2), 3600, "失败 2 次后等 60min");
    assert_eq!(backoff_secs(9), 3600);

    let mut st = idle();
    assert_eq!(backoff_remaining(&st, 1_000), None);
    st.attempts = 1;
    st.last_attempt_at = Some(1_000);
    // 验收口径：失败 1 次后**重启也不立刻重试**（wall-clock 记在 state.json 里）。
    assert_eq!(backoff_remaining(&st, 1_000), Some(600));
    assert_eq!(backoff_remaining(&st, 1_000 + 599), Some(1));
    assert_eq!(backoff_remaining(&st, 1_000 + 600), None);
    st.attempts = 2;
    assert_eq!(backoff_remaining(&st, 1_000 + 600), Some(3000));
    // 时钟回拨（NTP/手动改表）不能变成更长的等待，也不能 panic。
    assert_eq!(backoff_remaining(&st, 10), Some(3600));
}

/// 造一份"三件套齐全"的 nlu 配置（`labels` 内容可控）。
fn nlu_with_files(tag: &str, labels: &str) -> (NluConfig, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!("xz-bootstrap-{tag}-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let model = dir.join("model.onnx");
    let vocab = dir.join("tokenizer.json");
    let file = dir.join("labels.txt");
    for path in [&model, &vocab] {
        std::fs::write(path, b"x").unwrap();
    }
    std::fs::write(&file, labels.as_bytes()).unwrap();
    let mut nlu = nlu_with_intents();
    nlu.bert_model = model.display().to_string();
    nlu.bert_vocab = vocab.display().to_string();
    nlu.bert_labels = file.display().to_string();
    (nlu, dir)
}

#[test]
fn precheck_separates_human_decisions_from_unready_environments() {
    let nlu = nlu_with_intents();
    let st = idle();
    let ok =
        |dev: bool, mode: Mode, feature: bool, nlu: &NluConfig, art: ArtifactState, st: &State| {
            precheck(dev, mode, feature, nlu, art, st, 1_000)
        };
    assert!(ok(false, Mode::Adopt, true, &nlu, ArtifactState::Missing, &st).is_none());

    // 人为决定 → 终态 Stop（不该轮询）。
    for (block, reason) in [
        (
            ok(true, Mode::Adopt, true, &nlu, ArtifactState::Missing, &st),
            "dev_mode",
        ),
        (
            ok(false, Mode::Off, true, &nlu, ArtifactState::Missing, &st),
            "bootstrap_off",
        ),
        (
            ok(false, Mode::Adopt, false, &nlu, ArtifactState::Missing, &st),
            "feature_disabled",
        ),
    ] {
        match block.expect("必须被拦下") {
            Block::Stop { reason: got, .. } => assert_eq!(got, reason),
            other => panic!("{reason} 应记为 skipped，实际 {other:?}"),
        }
    }

    // 环境没就绪 → 长间隔重查，**不是**终态：首个 30s tick 撞上
    // `[nlu].enabled=false` 或卷还没挂好时，自举不能就此永久停摆。
    let mut disabled = nlu_with_intents();
    disabled.enabled = false;
    match ok(
        false,
        Mode::Adopt,
        true,
        &disabled,
        ArtifactState::Missing,
        &st,
    )
    .unwrap()
    {
        Block::Wait { reason, seconds } => {
            assert_eq!(reason, "nlu_disabled");
            assert_eq!(seconds, RECHECK_SECS);
        }
        other => panic!("{other:?}"),
    }
    match ok(
        false,
        Mode::Adopt,
        true,
        &nlu,
        ArtifactState::Unreadable("labels.txt: EACCES".into()),
        &st,
    )
    .unwrap()
    {
        Block::Wait { reason, .. } => assert_eq!(reason, "artifact_unreadable"),
        other => panic!("{other:?}"),
    }

    // 产物齐全：无需自举，终态。
    let (ready, dir) = nlu_with_files("pre", "other\n");
    match ok(
        false,
        Mode::Adopt,
        true,
        &ready,
        ArtifactState::Present,
        &st,
    )
    .unwrap()
    {
        Block::Stop { reason, .. } => assert_eq!(reason, "artifact_present"),
        other => panic!("{other:?}"),
    }
    let _ = std::fs::remove_dir_all(dir);

    // 终态与次数用尽都是 skipped，不再自动重跑。`stale` 必须在内，否则重启会拿
    // 同一份过期配置反复重训一版注定不能采用的模型。
    for phase in [
        Phase::Success,
        Phase::Adopted,
        Phase::GatedFailed,
        Phase::Stale,
        Phase::GivingUp,
        Phase::Skipped,
    ] {
        let mut terminal = idle();
        terminal.phase = phase;
        match ok(
            false,
            Mode::Adopt,
            true,
            &nlu,
            ArtifactState::Missing,
            &terminal,
        )
        .unwrap()
        {
            Block::Stop { reason, .. } => assert_eq!(reason, "phase_terminal"),
            other => panic!("{phase:?} → {other:?}"),
        }
    }
    let mut exhausted = idle();
    exhausted.attempts = MAX_ATTEMPTS;
    match ok(
        false,
        Mode::Adopt,
        true,
        &nlu,
        ArtifactState::Missing,
        &exhausted,
    )
    .unwrap()
    {
        Block::Stop { reason, .. } => assert_eq!(reason, "attempts_exhausted"),
        other => panic!("{other:?}"),
    }

    // 退避未到是"暂缓"，不是终态：状态机必须还能继续。
    let mut cooling = idle();
    cooling.attempts = 1;
    cooling.last_attempt_at = Some(1_000);
    match ok(
        false,
        Mode::Adopt,
        true,
        &nlu,
        ArtifactState::Missing,
        &cooling,
    )
    .unwrap()
    {
        Block::Wait { reason, seconds } => {
            assert_eq!(reason, "backoff");
            assert_eq!(seconds, 600);
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn stale_blocks_retry_but_failed_does_not() {
    let mut st = idle();
    st.phase = Phase::Stale;
    assert!(st.phase.blocks_retry(), "stale 是一次性终态");
    st.phase = Phase::Failed;
    assert!(!st.phase.blocks_retry(), "可重试失败必须还能继续");
}

#[test]
fn artifact_state_ignores_the_switch_but_not_missing_or_unreadable_files() {
    let (mut nlu, dir) = nlu_with_files("state", "other\n");
    // 关键语义：开关关着但产物齐全 = 用户主动关闭，不是"还没训过"。
    nlu.bert_enabled = false;
    assert_eq!(artifact_state(&nlu), ArtifactState::Present);

    // labels 空白等于没有标签可用（模型跑起来没有输出空间）。
    let file = std::path::PathBuf::from(&nlu.bert_labels);
    std::fs::write(&file, b"  \n\n").unwrap();
    assert_eq!(artifact_state(&nlu), ArtifactState::Missing);
    std::fs::write(&file, b"other\n").unwrap();
    assert_eq!(artifact_state(&nlu), ArtifactState::Present);

    // 任一路径缺失（含空串）都要重训。
    std::fs::remove_file(&nlu.bert_model).unwrap();
    assert_eq!(artifact_state(&nlu), ArtifactState::Missing);
    let mut blank = NluConfig::default();
    blank.bert_labels = file.display().to_string();
    assert_eq!(artifact_state(&blank), ArtifactState::Missing);

    // 文件在、读不出（权限/挂载）→ 环境问题，不能当成"没训练"去烧 attempts。
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let (locked, locked_dir) = nlu_with_files("locked", "other\n");
        let path = std::path::PathBuf::from(&locked.bert_labels);
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000)).unwrap();
        // root 无视权限位，那种环境下这条断言无意义。
        if std::fs::read_to_string(&path).is_err() {
            assert!(matches!(
                artifact_state(&locked),
                ArtifactState::Unreadable(_)
            ));
        }
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644));
        let _ = std::fs::remove_dir_all(locked_dir);
    }
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn state_file_round_trips_atomically_and_recovers_from_garbage() {
    let dir = std::env::temp_dir().join(format!("xz-bootstrap-state-{}", uuid::Uuid::new_v4()));
    let path = state_path(&dir);
    assert_eq!(load_state(&path), State::default(), "缺失文件按 idle");

    let mut st = idle();
    st.phase = Phase::Failed;
    st.attempts = 2;
    st.first_seen_at = 1_700_000_000;
    st.last_attempt_at = Some(1_700_000_100);
    st.last_error_kind = Some("weights_unavailable".into());
    st.last_error = Some("OSError: can't load".into());
    st.last_build_id = Some("b".into());
    st.last_base_model = Some(DEFAULT_BASE_MODEL.into());
    st.build_source_revision = Some("rev-a".into());
    st.current_revision = Some("rev-b".into());
    st.smoke = Some(SmokeSummary {
        ok: 4,
        bad_class: 0,
        rejected: 3,
        positives: 4,
        negatives: 3,
    });
    st.bad_class_detail = vec![MissDetail {
        text: "十分钟后提醒我喝水".into(),
        expected: "reminder_countdown".into(),
        got: "light_control".into(),
        reason: "wrong_intent:light_control".into(),
        confidence: 0.91,
    }];
    save_state(&path, &st).unwrap();
    assert_eq!(load_state(&path), st);
    // 原子写：临时文件不许残留（残留会被下一次 rename 覆盖，但目录里不该看到）。
    let leftovers: Vec<_> = std::fs::read_dir(dir.join("nlu-bootstrap"))
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| n.starts_with('.'))
        .collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");

    std::fs::write(&path, b"{not json").unwrap();
    assert_eq!(load_state(&path), State::default(), "坏文件按 idle，不许崩");
    std::fs::write(&path, br#"{"schema":99,"phase":"adopted","attempts":9}"#).unwrap();
    assert_eq!(
        load_state(&path),
        State::default(),
        "未知 schema 按 idle 重置"
    );
    // 缺字段的旧文件：serde default 补齐，不报错（所以新增字段不用升 schema）。
    std::fs::write(&path, br#"{"phase":"stale"}"#).unwrap();
    let partial = load_state(&path);
    assert_eq!(partial.phase, Phase::Stale);
    assert_eq!(partial.attempts, 0);
    assert!(partial.smoke.is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn save_patch_strict_keeps_secrets_and_rejects_a_stale_revision() {
    // 这是本轮唯一能证明"自动启用不会写坏配置"的东西：全仓 `save_patch` 之前零单测。
    use crate::config::store::{sqlite::SqliteConfigStore, ConfigStore};
    use crate::gateway::ws::config::{config_revision, save_patch_strict, SaveError};
    // 必须是**真实文件**库：`:memory:` 没有落盘文件，就断言不了"盘上一个字节都没动"。
    let dir = std::env::temp_dir().join(format!("xz-save-strict-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let db = dir.join("data.db");
    let db_path = db.display().to_string();
    let mut cfg = crate::config::Config::default();
    cfg.llm.api_key = "sk-live-must-survive".into();
    let store = SqliteConfigStore::open(&db_path).unwrap();
    store.persist(&cfg).unwrap();
    let engines = Arc::new(crate::runtime::Engines::new_dev(&cfg, store).unwrap());

    let before = serde_json::to_value(engines.config_store.load().unwrap()).unwrap();
    let revision = config_revision(&engines.config_store.load().unwrap());
    let patch = serde_json::json!({"nlu": {
        "bert_enabled": true,
        "bert_model": "/data/models/nlu-builds/x/model.onnx",
        "bert_vocab": "/data/models/nlu-builds/x/tokenizer.json",
        "bert_labels": "/data/models/nlu-builds/x/labels.txt",
    }});
    let outcome = match save_patch_strict(&engines, &patch, Some(&revision)) {
        Ok(outcome) => outcome,
        Err(e) => panic!("保存应当成功，实际 {e:?}"),
    };
    assert!(!outcome.revision.is_empty());

    let after = engines.config_store.load().unwrap();
    // (a) 密钥原值保留（`merge_client_patch` 的 sanitize 仍然生效——这就是不许另开写库路径的原因）。
    assert_eq!(after.llm.api_key, "sk-live-must-survive");
    // (b) 其他段逐字段不变。
    let strip = |v: &serde_json::Value| {
        let mut v = v.clone();
        v.as_object_mut().unwrap().remove("nlu");
        v
    };
    assert_eq!(
        strip(&before),
        strip(&serde_json::to_value(&after).unwrap()),
        "除 nlu 之外的配置一个字段都不许动"
    );
    assert!(after.nlu.bert_enabled);
    assert_eq!(after.nlu.bert_model, "/data/models/nlu-builds/x/model.onnx");

    // (c) 错 revision → typed RevisionConflict，且盘上**内容与 mtime 都不变**。
    // 只断 `Ok/Err` 是不够的：写进去又因冲突回滚，接口看着一样、盘上已经动了。
    let settled = serde_json::to_value(engines.config_store.load().unwrap()).unwrap();
    let before_bytes = std::fs::read(&db).unwrap();
    let before_mtime = std::fs::metadata(&db).unwrap().modified().unwrap();
    let err = match save_patch_strict(&engines, &patch, Some("deadbeef")) {
        Ok(_) => panic!("错 revision 必须被拒绝"),
        Err(e) => e,
    };
    assert!(matches!(err, SaveError::RevisionConflict { .. }));
    assert!(!err.is_retryable_io(), "冲突不是 IO，绝不能进重试分支");
    assert_eq!(
        serde_json::to_value(engines.config_store.load().unwrap()).unwrap(),
        settled
    );
    assert_eq!(
        std::fs::read(&db).unwrap(),
        before_bytes,
        "冲突时盘上内容一个字节都不许动"
    );
    assert_eq!(
        std::fs::metadata(&db).unwrap().modified().unwrap(),
        before_mtime,
        "冲突时连 mtime 都不许变（说明一次写事务都没开始）"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn missing_docker_cli_is_attributed_without_panicking() {
    // 真链路（不是桩）：run_build → training 里去 `docker image inspect`，本机没有 CLI/socket
    // 时命令起不来，必须被归因成 docker_unavailable。
    // **本机有 docker 时这条会静默 return**（会真的去拉镜像/训练），报告里必须照实说明。
    if std::process::Command::new("docker")
        .arg("--version")
        .output()
        .is_ok()
    {
        return;
    }
    let _gate = super::super::tests::JOB_TEST_GATE
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    use crate::config::store::{sqlite::SqliteConfigStore, ConfigStore};
    let mut cfg = crate::config::Config::default();
    let data = std::env::temp_dir().join(format!("xz-bootstrap-docker-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&data).unwrap();
    cfg.memory.db_path = data.join("data.db").display().to_string();
    let store = SqliteConfigStore::open(":memory:").unwrap();
    store.persist(&cfg).unwrap();
    let engines = Arc::new(crate::runtime::Engines::new_dev(&cfg, store).unwrap());
    let started = run_build(
        &engines,
        None,
        Some(DEFAULT_BASE_MODEL.into()),
        ORIGIN_BOOTSTRAP,
    )
    .await;
    let build_id = match started {
        BuildStart::Started { build_id, .. } => build_id,
        other => panic!("应当启动构建，实际 {}", describe_start(&other)),
    };
    let waited = tokio::time::timeout(Duration::from_secs(60), wait_build_done(&build_id))
        .await
        .expect("构建任务应在超时前结束");
    let snapshot = match waited {
        WaitOutcome::Done(snapshot) => snapshot,
        WaitOutcome::TakenOver { by } => panic!("单槽位内不应被接管: {by}"),
    };
    assert_eq!(snapshot.state, "failed");
    assert_eq!(
        classify_failure(&snapshot.error, &snapshot.log_tail),
        "docker_unavailable",
        "error={}",
        snapshot.error
    );
    let _ = std::fs::remove_dir_all(&data);
}

fn describe_start(start: &BuildStart) -> &'static str {
    match start {
        BuildStart::Started { .. } => "started",
        BuildStart::Busy => "busy",
        BuildStart::RevisionConflict => "revision_conflict",
        BuildStart::Invalid { .. } => "invalid",
        BuildStart::Unavailable { .. } => "unavailable",
    }
}

#[tokio::test]
async fn run_build_reports_busy_when_the_single_slot_is_taken() {
    // 手动与自举共用唯一槽位：抢不到必须返回 Busy（HTTP → 409，自举 → 让路），
    // 绝不允许"打断在跑的 docker 进程"这条捷径。
    let _gate = super::super::tests::JOB_TEST_GATE
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    use crate::config::store::{sqlite::SqliteConfigStore, ConfigStore};
    let mut cfg = crate::config::Config::default();
    let data = std::env::temp_dir().join(format!("xz-bootstrap-busy-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&data).unwrap();
    cfg.memory.db_path = data.join("data.db").display().to_string();
    let store = SqliteConfigStore::open(":memory:").unwrap();
    store.persist(&cfg).unwrap();
    let engines = Arc::new(crate::runtime::Engines::new_dev(&cfg, store).unwrap());
    {
        let mut j = job().lock().await;
        *j = super::super::BuildJob {
            running: true,
            state: "running",
            ..Default::default()
        };
    }
    let started = run_build(&engines, None, None, ORIGIN_BOOTSTRAP).await;
    assert!(matches!(started, BuildStart::Busy), "占用中必须让路");
    // 复原槽位，别把状态留给后面的用例。
    {
        let mut j = job().lock().await;
        *j = super::super::BuildJob::new();
    }
    let _ = std::fs::remove_dir_all(&data);
}

#[tokio::test]
async fn dev_mode_stops_without_training_and_records_skipped() {
    // 真链路（docker）不在这里跑；这里只证明"前置不满足就不动手且留下可解释状态"。
    use crate::config::store::{sqlite::SqliteConfigStore, ConfigStore};
    let mut cfg = crate::config::Config::default();
    let data = std::env::temp_dir().join(format!("xz-bootstrap-run-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&data).unwrap();
    cfg.memory.db_path = data.join("data.db").display().to_string();
    let store = SqliteConfigStore::open(":memory:").unwrap();
    store.persist(&cfg).unwrap();
    let engines = Arc::new(crate::runtime::Engines::new_dev(&cfg, store).unwrap());
    assert_eq!(run_once_for_test(&engines).await, "stop");
    let st = load_state(&state_path(&data));
    assert_eq!(st.phase, Phase::Skipped);
    assert_eq!(st.last_error_kind.as_deref(), Some("dev_mode"));
    assert_eq!(st.attempts, 0, "跳过不算尝试，不能消耗退避额度");
    let _ = std::fs::remove_dir_all(&data);
}
