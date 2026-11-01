//! NLU build contracts. Docker training and real model execution are deliberately excluded.
//!
//! docker 真训练不在单测范围（同 sherpa 口径：单测桩不冒充可运行服务）；
//! 这里验证：前置校验拒绝口径、状态端点形状、日志尾部截断（UTF-8 边界安全）、
//! docker.sock 挂载路径的宿主侧换算。

use super::*;

#[test]
fn training_catalog_contains_command_and_canonical_labels_but_not_deleted_seeds() {
    let mut cfg = Config::default();
    cfg.nlu
        .bert_label_map
        .insert("old_external_label".into(), "end_session".into());
    let nlu = training_catalog(&cfg).unwrap();
    let command = nlu
        .intents
        .iter()
        .find(|i| i.name == "end_session")
        .unwrap();
    assert_eq!(command.templates, cfg.command.keywords);
    assert!(nlu.bert_label_map.is_empty());
    let effective = crate::plugins::skill::effective_catalog(&cfg.command, &cfg.nlu).unwrap();
    assert_eq!(
        serde_json::to_value(&nlu.intents).unwrap(),
        serde_json::to_value(&effective.intents).unwrap()
    );
    for name in ["reminder_list", "reminder_cancel"] {
        let intent = nlu.intents.iter().find(|i| i.name == name).unwrap();
        let plan = intent.action.plan.as_ref().unwrap();
        assert_eq!(plan.steps[1].api, "replies.say");
        assert_eq!(plan.steps[1].args["text"], "{{steps.0.speech}}");
        assert!(intent.action.reply.is_empty());
    }
    assert!(!nlu.intents.iter().any(|i| i.name == "help_guide_demo"));
    cfg.nlu.intents.clear();
    let only_command = training_catalog(&cfg).unwrap();
    assert_eq!(only_command.intents.len(), 1);
    assert_eq!(only_command.intents[0].name, "end_session");
    cfg.command.enabled = false;
    assert!(training_catalog(&cfg).is_err());
}

#[test]
fn validate_rejects_when_nlu_disabled() {
    // 默认已改为 enabled=true（标准行为）；显式关闭的用户点训练必须被拦下并说清修法。
    let mut nlu = crate::plugins::nlu::NluConfig::default();
    nlu.enabled = false;
    let err = validate_buildable(&nlu).unwrap_err();
    assert!(err.contains("enabled=false"), "{err}");
}

#[test]
fn validate_rejects_when_no_intents() {
    let mut nlu = crate::plugins::nlu::NluConfig::default();
    nlu.enabled = true;
    nlu.intents.clear();
    let err = validate_buildable(&nlu).unwrap_err();
    assert!(err.contains("intents 为空"), "{err}");
}

#[test]
fn validate_accepts_builtin_reminders() {
    let mut nlu = crate::plugins::nlu::NluConfig::default();
    nlu.enabled = true;
    // Shipped defaults are actual config seeds, not hidden reinjection during training.
    assert!(validate_buildable(&nlu).is_ok());
}

#[tokio::test]
async fn status_body_shape() {
    let body = status_json(&BuildJob::new());
    assert_eq!(body["state"], "idle");
    assert_eq!(body["running"], false);
    assert!(body["expected_config"]["bert_enabled"].as_bool().unwrap());
}

#[tokio::test]
async fn push_log_keeps_tail_only_and_utf8_safe() {
    // push_log 现自带锁（训练期间不得跨子进程持锁），测试也走异步入口。
    push_log(&"训".repeat(6 * 1024)).await; // 多字节字符，验证边界回退
    push_log(&"x".repeat(20 * 1024)).await;
    let job = job().lock().await;
    assert!(job.log.len() <= 8 * 1024 + 32);
    assert!(job.log.is_char_boundary(0));
    assert!(job.log.ends_with('\n'));
}

#[test]
fn mount_host_path_bare_metal_unchanged() {
    // 无 mountinfo 匹配（裸机或非容器路径）：原样返回。
    assert_eq!(mount_host_path("/data/nlu-train", ""), "/data/nlu-train");
}

#[test]
fn mount_host_path_maps_container_data_bind() {
    // compose：宿主 ./server-data → 容器 /data（bind 挂载源为绝对路径）。
    let info = "721 719 0:99 / /data rw,relatime - ext4 /srv/xiaozhi/server-data rw\n";
    assert_eq!(
        mount_host_path("/data/nlu-train", info),
        "/srv/xiaozhi/server-data/nlu-train"
    );
    assert_eq!(mount_host_path("/data", info), "/srv/xiaozhi/server-data");
}

#[test]
fn mount_host_path_prefers_longest_match() {
    // 嵌套挂载取最长 mountpoint 前缀。
    let info = concat!(
        "10 1 0:5 / /data rw - ext4 /mnt/host-data rw\n",
        "11 10 0:6 / /data/models rw - nfs 10.0.0.9:/exports/models rw\n",
    );
    assert_eq!(
        mount_host_path("/data/models/nlu", info),
        "10.0.0.9:/exports/models/nlu"
    );
    assert_eq!(
        mount_host_path("/data/nlu-train", info),
        "/mnt/host-data/nlu-train"
    );
}

#[test]
fn mount_host_path_ignores_prefix_collision() {
    // /data 不应匹配 /database。
    let info = "10 1 0:5 / /database rw - ext4 /mnt/other rw\n";
    assert_eq!(mount_host_path("/data/nlu-train", info), "/data/nlu-train");
}

#[test]
fn mount_host_path_subtree_root() {
    // 挂载源端是子树（root 非 /，如镜像分层 bind）：定位到子树内同相对位置。
    let info = "10 1 0:5 /server-data /data rw - ext4 /mnt/host rw\n";
    assert_eq!(
        mount_host_path("/data/nlu-train", info),
        "/mnt/host/server-data/nlu-train"
    );
}

#[test]
fn mount_host_path_skips_longer_mountpoint_without_panic() {
    // 真实事故：mountinfo 里存在比目标路径更长的兄弟挂载点，
    // 旧实现先按 mnt.len() 切片再校验前缀，直接 byte index 越界 panic。
    let info = concat!(
        "10 1 0:5 / /data rw - ext4 /mnt/host-data rw\n",
        "12 10 0:7 / /data/nlu-train-env rw - ext4 /mnt/host-data/nlu-train-env rw\n",
    );
    assert_eq!(
        mount_host_path("/data/nlu-train", info),
        "/mnt/host-data/nlu-train"
    );
}

#[test]
fn map_via_mounts_replaces_longest_destination() {
    // 真实事故（UGOS NAS）：mountinfo source 只是设备名，需走自己容器 inspect
    // 的 Mounts 做精确映射；嵌套挂载取最长 Destination。
    let mounts = vec![
        (
            "/data".to_string(),
            "/volume3/docker/xiaozhi-server-rust/server-data".to_string(),
        ),
        (
            "/data/nlu-train-env".to_string(),
            "/volume3/docker/xiaozhi-server-rust/server-data/nlu-train-env".to_string(),
        ),
    ];
    assert_eq!(
        map_via_mounts("/data/nlu-train", &mounts),
        Some("/volume3/docker/xiaozhi-server-rust/server-data/nlu-train".to_string())
    );
}

#[test]
fn map_via_mounts_rejects_non_boundary_prefix() {
    // /database 不是 /data 的挂载点，不能只按字符串前缀命中。
    let mounts = vec![("/database".to_string(), "/host/db".to_string())];
    assert_eq!(map_via_mounts("/data/nlu-train", &mounts), None);
}

#[test]
fn saved_snapshot_reads_latest_sqlite_not_startup_or_live_cache() {
    use crate::config::store::{sqlite::SqliteConfigStore, ConfigStore};
    let store = SqliteConfigStore::open(":memory:").unwrap();
    let old = Config::default();
    store.persist(&old).unwrap();
    let old_revision = super::super::config::config_revision(&store.load().unwrap());
    let mut saved = old.clone();
    saved.nlu.intents.push(crate::plugins::nlu::IntentDef {
        name: "new_saved_skill".into(),
        templates: vec!["显示帮助".into(), "介绍功能".into()],
        action: crate::plugins::nlu::ActionDef {
            kind: "reply".into(),
            reply: "帮助".into(),
            ..Default::default()
        },
    });
    store.persist(&saved).unwrap();
    let (actual, revision) = saved_snapshot(store.as_ref(), None).unwrap();
    assert!(actual
        .nlu
        .intents
        .iter()
        .any(|i| i.name == "new_saved_skill"));
    assert_ne!(revision, old_revision);
    assert_eq!(revision, super::super::config::config_revision(&actual));
    assert!(saved_snapshot(store.as_ref(), Some(&old_revision)).is_err());
    assert!(saved_snapshot(store.as_ref(), Some(&revision)).is_ok());
}

fn dev_fixture() -> Arc<Engines> {
    use crate::config::store::{sqlite::SqliteConfigStore, ConfigStore};
    let mut cfg = Config::default();
    cfg.server.expected_token = "admin-test".into();
    cfg.memory.db_path = std::env::temp_dir()
        .join(format!("xz-nlu-build-{}", uuid::Uuid::new_v4()))
        .join("data.db")
        .display()
        .to_string();
    let store = SqliteConfigStore::open(":memory:").unwrap();
    store.persist(&cfg).unwrap();
    Engines::new_dev(&cfg, store).unwrap()
}

#[tokio::test]
async fn build_handlers_require_admin_token_and_dev_rejects_real_training() {
    let engines = dev_fixture();
    assert_eq!(
        build_status(
            State(engines.clone()),
            HeaderMap::new(),
            Query(HashMap::new())
        )
        .await
        .status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        start_build(
            State(engines.clone()),
            HeaderMap::new(),
            Query(HashMap::new()),
            Bytes::new()
        )
        .await
        .status(),
        StatusCode::UNAUTHORIZED
    );
    let mut headers = HeaderMap::new();
    headers.insert("authorization", "Bearer admin-test".parse().unwrap());
    assert_eq!(
        start_build(
            State(engines.clone()),
            headers.clone(),
            Query(HashMap::new()),
            Bytes::new()
        )
        .await
        .status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert_eq!(
        build_status(State(engines), headers, Query(HashMap::new()))
            .await
            .status(),
        StatusCode::OK
    );
}

#[tokio::test]
async fn start_build_rejects_stale_revision_before_spawning_docker() {
    let mut engines = Arc::try_unwrap(dev_fixture()).ok().unwrap();
    engines.dev_mode = false; // Keep unavailable providers; this path must stop before spawning.
    let engines = Arc::new(engines);
    let before = serde_json::to_value(engines.config_store.load().unwrap()).unwrap();
    let mut headers = HeaderMap::new();
    headers.insert("authorization", "Bearer admin-test".parse().unwrap());
    let result = start_build(
        State(engines.clone()),
        headers,
        Query(HashMap::new()),
        Bytes::from_static(br#"{"expected_revision":"stale"}"#),
    )
    .await;
    assert_eq!(result.status(), StatusCode::CONFLICT);
    let bytes = axum::body::to_bytes(result.into_body(), 8192)
        .await
        .unwrap();
    assert_eq!(
        serde_json::from_slice::<Value>(&bytes).unwrap()["error_code"],
        "revision_conflict"
    );
    assert_eq!(
        serde_json::to_value(engines.config_store.load().unwrap()).unwrap(),
        before
    );
}
