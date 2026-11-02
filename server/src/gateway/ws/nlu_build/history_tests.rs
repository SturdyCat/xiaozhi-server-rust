use super::*;
use crate::config::store::{sqlite::SqliteConfigStore, ConfigStore};
use crate::{
    config::Config,
    gateway::ws::nlu_build::{INTEGRITY_INCOMPLETE, INTEGRITY_UNVERIFIED},
};

// ---------------------------------------------------------------------------
// history 的纯函数与端到端行为。目录造数复用 artifact fixture 的手工方式
// （写三件套 + manifest），不真跑训练。
// ---------------------------------------------------------------------------

fn active_id_of(bert_model: &str) -> Option<String> {
    active_build_id(bert_model)
}

#[test]
fn active_build_id_takes_the_parent_directory_of_bert_model() {
    let id = uuid::Uuid::new_v4().to_string();
    let model = format!("/data/models/nlu-builds/{id}/model.onnx");
    assert_eq!(active_id_of(&model).as_deref(), Some(id.as_str()));
    // 旧式路径 / 外部路径：父目录不是规范 UUID → None，不猜。
    assert_eq!(active_id_of("/models/rbt3/model.onnx"), None);
    assert_eq!(active_id_of(""), None);
    assert_eq!(active_id_of("/"), None);
}

#[test]
fn rows_sort_none_last_then_newest_first_then_build_id() {
    let mk = |created: Option<&str>, id: &str| HistoryRow {
        head: ManifestHead {
            build_id: id.into(),
            created_at: created.map(String::from),
            base_model: None,
            base_model_source: None,
            labels: Vec::new(),
            files: Vec::new(),
            integrity: INTEGRITY_UNVERIFIED,
            missing: Vec::new(),
        },
        active: false,
        error: None,
    };
    let mut rows = vec![
        mk(None, "bbbbbbbb-0000-0000-0000-000000000000"),
        mk(Some("2026-11-02T08:00:00Z"), "cccccccc-0000-0000-0000-000000000000"),
        mk(Some("2026-11-02T09:00:00Z"), "aaaaaaaa-0000-0000-0000-000000000000"),
        mk(None, "aaaaaaaa-0000-0000-0000-000000000000"),
        mk(Some("2026-11-02T08:00:00Z"), "bbbbbbbb-0000-0000-0000-000000000000"),
    ];
    sort_rows(&mut rows);
    let ids: Vec<&str> = rows.iter().map(|r| r.head.build_id.as_str()).collect();
    // 有时间的新→旧（同秒按 build_id 字典序），None 垫底（同样按字典序）。
    assert_eq!(
        ids,
        vec![
            "aaaaaaaa-0000-0000-0000-000000000000", // 09:00
            "bbbbbbbb-0000-0000-0000-000000000000", // 08:00
            "cccccccc-0000-0000-0000-000000000000", // 08:00
            "aaaaaaaa-0000-0000-0000-000000000000", // None
            "bbbbbbbb-0000-0000-0000-000000000000", // None
        ]
    );
}

/// 造一个受管候选目录（manifest + 三件套），返回 (data_root, build_id, dir)。
fn seed(data: &std::path::Path, id: &str, labels: &[&str]) -> std::path::PathBuf {
    let dir = data.join("models").join("nlu-builds").join(id);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("model.onnx"), b"fixture").unwrap();
    std::fs::write(dir.join("tokenizer.json"), b"fixture").unwrap();
    std::fs::write(dir.join("labels.txt"), labels.join("\n") + "\n").unwrap();
    let manifest = serde_json::json!({
        "build_id": id,
        "source_revision": "saved",
        "source_input_sha256": "input",
        "trainer_image": "img",
        "trainer_image_id": "img-id",
        "trainer_script_sha256": "script",
        "labels": labels,
        "files": [
            {"name":"model.onnx","bytes":7,"sha256":"x"},
            {"name":"tokenizer.json","bytes":7,"sha256":"x"},
            {"name":"labels.txt","bytes":labels.join("\n").len()+1,"sha256":"x"}
        ],
        "created_at": "2026-11-02T08:00:00Z",
        "base_model": "rbt3",
        "base_model_source": "default"
    });
    std::fs::write(dir.join("manifest.json"), serde_json::to_vec(&manifest).unwrap()).unwrap();
    dir
}

const L3: [&str; 4] = ["a", "b", "c", "other"];

#[test]
fn load_manifest_reports_unverified_without_hashing() {
    let data = std::env::temp_dir().join(format!("xz-hist-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(data.join("models").join("nlu-builds")).unwrap();
    let id = uuid::Uuid::new_v4().to_string();
    seed(&data, &id, &L3);
    let head = load_manifest(&data, &id).unwrap();
    assert_eq!(head.build_id, id);
    assert_eq!(head.integrity, INTEGRITY_UNVERIFIED);
    assert_eq!(head.created_at.as_deref(), Some("2026-11-02T08:00:00Z"));
    assert_eq!(head.base_model.as_deref(), Some("rbt3"));
    assert!(head.missing.is_empty());
    std::fs::remove_dir_all(&data).unwrap();
}

#[test]
fn load_manifest_maps_every_failure_to_a_typed_variant() {
    let data = std::env::temp_dir().join(format!("xz-hist-{}", uuid::Uuid::new_v4()));
    let id = uuid::Uuid::new_v4().to_string();
    // 根不存在
    assert!(matches!(
        load_manifest(&data, &id),
        Err(CandidateError::RootMissing)
    ));
    std::fs::create_dir_all(data.join("models").join("nlu-builds")).unwrap();
    // 目录不存在
    assert!(matches!(
        load_manifest(&data, &id),
        Err(CandidateError::DirectoryMissing)
    ));
    // 非规范 id 视为目录不存在（404 族），不是 corrupt
    assert!(matches!(
        load_manifest(&data, "../evil"),
        Err(CandidateError::DirectoryMissing)
    ));
    let dir = seed(&data, &id, &L3);
    // 缺文件（磁盘有 manifest 记录但文件没了）
    std::fs::remove_file(dir.join("tokenizer.json")).unwrap();
    match load_manifest(&data, &id) {
        Ok(head) => {
            assert_eq!(head.integrity, INTEGRITY_INCOMPLETE);
            assert!(head.missing.iter().any(|m| m.contains("tokenizer.json")));
        }
        Err(e) => panic!("expected incomplete row, got {e}"),
    }
    // manifest 解析失败
    std::fs::write(dir.join("tokenizer.json"), b"fixture").unwrap();
    std::fs::write(dir.join("manifest.json"), b"{not json").unwrap();
    assert!(matches!(
        load_manifest(&data, &id),
        Err(CandidateError::Corrupt { .. })
    ));
    // manifest build_id 与目录名不符
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.join("manifest.json")).unwrap_or_default())
            .unwrap_or(json!({}));
    let _ = manifest; // 上一步已写坏 manifest，这里只做目录名不一致的独立用例：
    let other = uuid::Uuid::new_v4().to_string();
    seed(&data, &other, &L3);
    let mut tampered = serde_json::from_slice::<serde_json::Value>(
        &std::fs::read(data.join("models").join("nlu-builds").join(&other).join("manifest.json"))
            .unwrap(),
    )
    .unwrap();
    tampered["build_id"] = json!(id);
    std::fs::write(
        data.join("models").join("nlu-builds").join(&other).join("manifest.json"),
        serde_json::to_vec(&tampered).unwrap(),
    )
    .unwrap();
    match load_manifest(&data, &other) {
        Err(CandidateError::Corrupt { detail }) => {
            assert!(detail.contains("build_id mismatch"), "{detail}");
        }
        other => panic!("expected corrupt, got {other:?}"),
    }
    std::fs::remove_dir_all(&data).unwrap();
}

#[test]
fn http_mapping_sends_404_only_for_missing_roots_or_directories() {
    use axum::http::StatusCode;
    let (status, code, _) = candidate_http_error(&CandidateError::RootMissing);
    assert_eq!((status, code), (StatusCode::NOT_FOUND, "candidate_not_found"));
    let (status, code, _) = candidate_http_error(&CandidateError::DirectoryMissing);
    assert_eq!((status, code), (StatusCode::NOT_FOUND, "candidate_not_found"));
    let incomplete = CandidateError::Incomplete {
        missing: vec!["model.onnx".into()],
    };
    let (status, code, _) = candidate_http_error(&incomplete);
    assert_eq!((status, code), (StatusCode::CONFLICT, "candidate_incomplete"));
    let corrupt = CandidateError::Corrupt {
        detail: "x".into(),
    };
    let (status, code, _) = candidate_http_error(&corrupt);
    assert_eq!((status, code), (StatusCode::CONFLICT, "candidate_corrupt"));
    let hash = CandidateError::HashMismatch {
        name: "model.onnx".into(),
        expected: "a".into(),
        actual: "b".into(),
    };
    let (status, code, _) = candidate_http_error(&hash);
    assert_eq!(
        (status, code),
        (StatusCode::CONFLICT, "candidate_hash_mismatch")
    );
    let labels = CandidateError::LabelsChanged {
        detail: "x".into(),
    };
    let (status, code, _) = candidate_http_error(&labels);
    assert_eq!(
        (status, code),
        (StatusCode::CONFLICT, "candidate_labels_changed")
    );
}

/// 端到端：真实路由 + 鉴权 + 目录造数。空根 → 空列表不是 404；两份候选 →
/// 排序与 active 标记正确；坏 manifest → corrupt 行照常出现。
#[tokio::test]
async fn history_endpoint_lists_sorts_and_marks_active() {
    let data = std::env::temp_dir().join(format!("xz-hist-e2e-{}", uuid::Uuid::new_v4()));
    let builds = data.join("models").join("nlu-builds");
    std::fs::create_dir_all(&builds).unwrap();
    let mut cfg = Config::default();
    cfg.server.expected_token = "test-admin-token".into();
    // data_dir = memory.db_path 的父目录（见 nlu_build::data_dir）：把库放进造数根，
    // 服务的候选根才会落在这里。
    cfg.memory.db_path = data.join("data.db").display().to_string();
    let older = uuid::Uuid::new_v4().to_string();
    let newer = uuid::Uuid::new_v4().to_string();
    seed(&data, &older, &L3);
    seed(&data, &newer, &L3);
    // 把 newer 的 created_at 改新一点（seed 写的是同一时刻）。
    let manifest_path = builds.join(&newer).join("manifest.json");
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&manifest_path).unwrap()).unwrap();
    manifest["created_at"] = json!("2026-11-02T09:00:00Z");
    std::fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
    // 现役 = newer（cfg.nlu.bert_model 指向它的 model.onnx）。
    cfg.nlu.bert_enabled = true;
    cfg.nlu.bert_model = builds.join(&newer).join("model.onnx").display().to_string();
    // 第三行：manifest 坏掉的候选——corrupt 行照常出现在列表里（运维要能看见它），
    // build_id 取自目录名，error 带原因。
    let broken = uuid::Uuid::new_v4().to_string();
    seed(&data, &broken, &L3);
    std::fs::write(builds.join(&broken).join("manifest.json"), b"{not json").unwrap();
    let store = SqliteConfigStore::open(":memory:").unwrap();
    store.persist(&cfg).unwrap();
    let engines = Engines::new_dev(&cfg, store).unwrap();

    // 无鉴权 → 401（鉴权先于一切）。
    let denied = build_history(
        State(engines.clone()),
        HeaderMap::new(),
        axum::extract::Query(Default::default()),
    )
    .await;
    assert_eq!(denied.status(), axum::http::StatusCode::UNAUTHORIZED);

    let mut headers = HeaderMap::new();
    headers.insert("authorization", "Bearer test-admin-token".parse().unwrap());
    let response = build_history(State(engines), headers, axum::extract::Query(Default::default()))
        .await;
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), 1 << 20).await.unwrap();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body["total"], 3);
    assert_eq!(body["active_build_id"], json!(newer));
    let rows = body["builds"].as_array().unwrap();
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0]["build_id"], json!(newer), "新的在前");
    assert_eq!(rows[0]["active"], true);
    assert_eq!(rows[0]["integrity"], INTEGRITY_UNVERIFIED);
    assert_eq!(rows[1]["build_id"], json!(older));
    assert_eq!(rows[1]["active"], false);
    // corrupt 行：时间缺失垫底，error 带原因，build_id 来自目录名。
    assert_eq!(rows[2]["build_id"], json!(broken));
    assert_eq!(rows[2]["integrity"], INTEGRITY_CORRUPT);
    assert!(rows[2]["error"]
        .as_str()
        .unwrap()
        .contains("解析失败"));
    assert_eq!(rows[2]["active"], false);
    std::fs::remove_dir_all(&data).unwrap();
}

#[tokio::test]
async fn history_endpoint_treats_a_missing_root_as_an_empty_list() {
    let mut cfg = Config::default();
    cfg.server.expected_token = "test-admin-token".into();
    // data 目录压根没建：从未构建过 = 空列表，不是 404。
    cfg.memory.db_path = std::env::temp_dir()
        .join(format!("xz-hist-empty-{}", uuid::Uuid::new_v4()))
        .join("data.db")
        .display()
        .to_string();
    let store = SqliteConfigStore::open(":memory:").unwrap();
    store.persist(&cfg).unwrap();
    let engines = Engines::new_dev(&cfg, store).unwrap();
    let mut headers = HeaderMap::new();
    headers.insert("authorization", "Bearer test-admin-token".parse().unwrap());
    let response = build_history(
        State(engines),
        headers,
        axum::extract::Query(Default::default()),
    )
    .await;
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), 1 << 20).await.unwrap();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body["builds"], json!([]));
    assert_eq!(body["total"], 0);
}
