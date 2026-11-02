//! NLU build contracts. Docker training and real model execution are deliberately excluded.
//!
//! docker 真训练不在单测范围（同 sherpa 口径：单测桩不冒充可运行服务）；
//! 这里验证：前置校验拒绝口径、状态端点形状、日志尾部截断（UTF-8 边界安全）、
//! docker.sock 挂载路径的宿主侧换算。

use super::*;

/// 进程级构建槽位（`job()`）是全局单例：凡是会改写它的用例都先抢这把锁，
/// 否则并行跑测试时"我重置了 job"会打断别人对 `log_tail` 的断言。
pub(super) static JOB_TEST_GATE: std::sync::Mutex<()> = std::sync::Mutex::new(());

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
    // 空闲态如实给 null，不假装跑过某个底座；前端只在有构建时读这两个键。
    assert!(body["base_model"].is_null());
    assert!(body["base_model_source"].is_null());
}

#[test]
fn status_exposes_the_arm_table_and_flags_the_default() {
    // 界面选择器的数据源：`bases` 必须自带 default 标记，客户端不再硬编码档位。
    let body = status_json(&BuildJob::new());
    let bases = body["bases"].as_array().unwrap();
    assert_eq!(bases.len(), BASE_MODELS.len());
    let default = bases
        .iter()
        .find(|b| b["default"] == true)
        .expect("必须有一臂被标记为默认");
    assert_eq!(default["name"], DEFAULT_BASE_MODEL);
    for base in bases {
        assert!(base["hub"].as_str().unwrap().starts_with("hfl/"));
        assert!(base["onnx_estimate_mb"].as_u64().unwrap() > 0);
        assert!(!base["note"].as_str().unwrap().is_empty());
    }
    // 空闲态不假装跑过某个底座。
    assert!(body["base_model"].is_null());
    assert!(body["origin"].is_null());
}

#[test]
fn base_model_defaults_to_the_shipped_arm_and_never_silently_falls_back() {
    // 缺省臂 = h288（与 train.py 的 BASE_NAME 缺省、前端 view 初值同批改）。
    let (base, source) = resolve_base(None).unwrap();
    assert_eq!(base.name, DEFAULT_BASE_MODEL);
    assert_eq!(base.name, "minirbt-h288");
    assert_eq!(base.hub, "hfl/minirbt-h288");
    assert_eq!(source, BASE_SOURCE_DEFAULT);
    let (explicit, source) = resolve_base(Some("rbt3")).unwrap();
    assert_eq!(explicit.name, "rbt3");
    assert_eq!(source, BASE_SOURCE_REQUEST);
    let (small, source) = resolve_base(Some("minirbt-h288")).unwrap();
    assert_eq!(small.hub, "hfl/minirbt-h288");
    assert_eq!(source, BASE_SOURCE_REQUEST);
    // 空白对齐 train.py 的 .strip()：两侧对 " rbt3" 的口径必须一致（脚本接受、服务端就不能 400）。
    for name in [" rbt3", "rbt3 ", "\tminirbt-h288 "] {
        let (base, _) = resolve_base(Some(name)).unwrap();
        assert!(
            base.name == "rbt3" || base.name == "minirbt-h288",
            "{name:?}"
        );
    }
    // 历史写法（直接传 hub id）、大小写不符、空串、未来臂都必须报错而不是回落默认臂。
    for name in ["hfl/rbt3", "RBT3", "   ", "", "rbt4-h312"] {
        let err = resolve_base(Some(name)).unwrap_err();
        assert!(err.contains("allowed"), "{err}");
        assert!(err.contains("minirbt-h288"), "{err}");
    }
}

#[test]
fn base_model_env_matches_trainer_contract() {
    // env 是本维度唯一的传递通道（input.json 语义不动、产物白名单不扩）。
    let (base, _) = resolve_base(None).unwrap();
    assert_eq!(
        base.env_args(),
        [
            "MODEL_NAME=minirbt-h288".to_string(),
            "LEARNING_RATE=0.0001".to_string(),
            "EPOCHS=10".to_string()
        ]
    );
    let (base, _) = resolve_base(Some("rbt3")).unwrap();
    assert_eq!(
        base.env_args(),
        [
            "MODEL_NAME=rbt3".to_string(),
            "LEARNING_RATE=0.00005".to_string(),
            "EPOCHS=6".to_string()
        ]
    );
}

#[test]
fn base_model_table_matches_trainer_source_profile_for_profile() {
    // 双表一致性：服务端表与编译进二进制的 train.py 的 BASES 必须**逐字段**相同。
    // 只锁键集合不够：`hub` 在服务端只用于日志（training.rs 的 base_hub=），真实下载读的是
    // train.py 的表，只改一边的 hub 会让测试全绿而日志骗人；lr/epochs 同理会训练出
    // 与表上记录不一致的超参。纯内存断言，不启动 docker、不读工作区外的文件。
    let profiles = python_bases_profiles(TRAINER_SOURCE);
    assert!(
        !profiles.is_empty(),
        "未能从 TRAINER_SOURCE 解析出 BASES profile，解析方式需要更新"
    );
    let served = BASE_MODELS
        .iter()
        .map(|base| {
            (
                base.name.to_string(),
                (base.hub.to_string(), base.learning_rate, base.epochs),
            )
        })
        .collect::<std::collections::BTreeMap<_, _>>();
    assert_eq!(profiles, served);
    assert!(profiles.contains_key(DEFAULT_BASE_MODEL));
    // 保序解析与内容解析必须看到同样多的臂：否则"首项"断言会因为解析漏行而假绿。
    assert_eq!(
        python_bases_order(TRAINER_SOURCE).len(),
        profiles.len(),
        "python_bases_order 与 python_bases_profiles 解析出的臂数不一致"
    );
    // 缺省臂必须等于 `BASES` 的**首项**（顺序即语义）：改默认臂时服务端与脚本
    // 必须同批改，否则 standalone `docker run` 会训另一个底座。
    assert_eq!(
        python_bases_order(TRAINER_SOURCE)
            .first()
            .map(String::as_str),
        Some(DEFAULT_BASE_MODEL),
        "服务端 DEFAULT_BASE_MODEL 必须等于 train.py BASES 的首项"
    );
}

/// 训练镜像的缺省臂（`docker run <image>` 这类 standalone 运行的入口）。
const TRAINER_DOCKERFILE: &str = include_str!("../../../../../tools/nlu-trainer/Dockerfile");

#[test]
fn trainer_image_defaults_to_the_same_arm() {
    // 缺省臂在**四处**同时存在：train.py 的 `BASE_NAME`、`base.rs` 的 `DEFAULT_BASE_MODEL`、
    // Dockerfile 的 `ENV MODEL_NAME=`、前端 view 初值。QA 复核发现 Dockerfile 漏改
    // （值还是 rbt3，注释却写着 h288）——只改值不加断言，下次还会漂，所以在这里钉住。
    let value = TRAINER_DOCKERFILE
        .lines()
        .find_map(|line| line.trim().strip_prefix("MODEL_NAME="))
        .expect("Dockerfile 里必须有 MODEL_NAME=");
    assert_eq!(value.trim(), DEFAULT_BASE_MODEL);
}

#[test]
fn request_parsing_keeps_deny_unknown_fields_and_accepts_base_model() {
    let req: BuildRequest = serde_json::from_slice(br#"{"base_model":"minirbt-h288"}"#).unwrap();
    assert_eq!(req.base_model.as_deref(), Some("minirbt-h288"));
    let bare: BuildRequest = serde_json::from_slice(b"{}").unwrap();
    assert!(bare.base_model.is_none());
    // 未知键依旧拒绝：底座不能靠另一种字段名混进来。
    assert!(serde_json::from_slice::<BuildRequest>(br#"{"base":"rbt3"}"#).is_err());
}

#[tokio::test]
async fn start_build_rejects_invalid_base_model_with_400_before_reading_config() {
    let mut engines = Arc::try_unwrap(dev_fixture()).ok().unwrap();
    engines.dev_mode = false; // 走到真实校验分支；非法底座在读配置前就应当失败。
    let engines = Arc::new(engines);
    let mut headers = HeaderMap::new();
    headers.insert("authorization", "Bearer admin-test".parse().unwrap());
    let result = start_build(
        State(engines),
        headers,
        Query(HashMap::new()),
        Bytes::from_static(br#"{"base_model":"hfl/rbt3"}"#),
    )
    .await;
    assert_eq!(result.status(), StatusCode::BAD_REQUEST);
    let bytes = axum::body::to_bytes(result.into_body(), 8192)
        .await
        .unwrap();
    let body = serde_json::from_slice::<Value>(&bytes).unwrap();
    assert_eq!(body["error_code"], "invalid_base_model");
    assert!(body["error"].as_str().unwrap().contains("minirbt-h288"));
}

/// 从训练脚本源码里取 `BASES` 每一臂的 profile：`name → (hub, lr, epochs)`。
///
/// 只认这一种写法：`BASES = {` 起始、单独一行 `}` 结束、每臂一行、嵌套字典写在行内。
/// 行内字典是 JSON 兼容字面量（`{"hub": "hfl/rbt3", "lr": 5e-5, "epochs": 6.0}`，5e-5/6.0
/// 都是合法 JSON 数字），所以直接 serde_json 解析，能逐字段比对而不只是比键。
/// 解析失败（profile 为空）由调用方断言暴露，而不是静默通过。
/// `BASES` 的键**按源码出现顺序**（`python_bases_profiles` 用 BTreeMap 排序，
/// 只能锁内容、锁不了"首项"，而缺省臂的语义恰好是首项）。
fn python_bases_order(source: &str) -> Vec<String> {
    let mut order = Vec::new();
    let mut inside = false;
    for line in source.lines() {
        let trimmed = line.trim_end();
        if !inside {
            inside = trimmed.trim_start().starts_with("BASES = {");
            continue;
        }
        if trimmed.trim() == "}" {
            break;
        }
        let Some(rest) = trimmed
            .strip_prefix("    ")
            .and_then(|r| r.strip_prefix('"'))
        else {
            continue;
        };
        if let Some((name, tail)) = rest.split_once('"') {
            if tail.contains('{') {
                order.push(name.to_string());
            }
        }
    }
    order
}

fn python_bases_profiles(source: &str) -> std::collections::BTreeMap<String, (String, f64, f64)> {
    let mut profiles = std::collections::BTreeMap::new();
    let mut inside = false;
    for line in source.lines() {
        let trimmed = line.trim_end();
        if !inside {
            inside = trimmed.trim_start().starts_with("BASES = {");
            continue;
        }
        if trimmed.trim() == "}" {
            break;
        }
        let Some(rest) = trimmed
            .strip_prefix("    ")
            .and_then(|r| r.strip_prefix('"'))
        else {
            continue;
        };
        let Some((name, tail)) = rest.split_once('"') else {
            continue;
        };
        let (Some(start), Some(end)) = (tail.find('{'), tail.rfind('}')) else {
            continue;
        };
        let Ok(profile) = serde_json::from_str::<Value>(&tail[start..=end]) else {
            continue;
        };
        let Some(hub) = profile.get("hub").and_then(Value::as_str) else {
            continue;
        };
        let Some(lr) = profile.get("lr").and_then(Value::as_f64) else {
            continue;
        };
        let Some(epochs) = profile.get("epochs").and_then(Value::as_f64) else {
            continue;
        };
        profiles.insert(name.to_string(), (hub.to_string(), lr, epochs));
    }
    profiles
}

#[tokio::test]
async fn push_log_keeps_tail_only_and_utf8_safe() {
    let _gate = JOB_TEST_GATE.lock().unwrap_or_else(|e| e.into_inner());
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
