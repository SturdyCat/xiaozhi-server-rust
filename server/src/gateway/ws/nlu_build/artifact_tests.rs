use super::*;
use crate::plugins::nlu::NluConfig;

#[test]
fn legacy_manifest_does_not_invent_command_label_or_expand_its_catalog() {
    let (data, id, dir, artifact) = fixture();
    let mut legacy = serde_json::to_value(&artifact).unwrap();
    legacy.as_object_mut().unwrap().remove("catalog_source");
    std::fs::write(
        dir.join("manifest.json"),
        serde_json::to_vec(&legacy).unwrap(),
    )
    .unwrap();
    let (loaded, _) = load_candidate(&data, &id).unwrap();
    assert!(loaded.catalog_source.is_none());
    assert_eq!(loaded.labels, artifact.labels);
    assert_eq!(
        loaded.labels,
        [
            "reminder_countdown",
            "reminder_schedule",
            "reminder_repeat",
            "other"
        ]
    );
    assert!(!loaded
        .labels
        .iter()
        .any(|l| l == "reminder_monthly" || l == "reminder_list" || l == "end_session"));
    assert!(!loaded
        .config(&dir)
        .intents
        .iter()
        .any(|i| i.name == "end_session"));
}

fn candidate(id: &str) -> Artifact {
    // A legacy snapshot must not grow when the shipped default seed grows.
    let nlu: NluConfig = serde_json::from_value(json!({"enabled":true,"intents":[
        {"name":"reminder_countdown","templates":["旧版倒计时"],"action":{"kind":"reply","reply":"旧版仅应答"}},
        {"name":"reminder_schedule","templates":["旧版定时"],"action":{"kind":"reply","reply":"旧版仅应答"}},
        {"name":"reminder_repeat","templates":["旧版循环"],"action":{"kind":"reply","reply":"旧版仅应答"}}
    ],"slots":[]})).unwrap();
    Artifact {
        build_id: id.into(),
        source_revision: "saved-revision".into(),
        source_input_sha256: sha256(b"input"),
        trainer_image: "dependency-image".into(),
        trainer_image_id: "sha256:environment".into(),
        trainer_script_sha256: sha256(b"trusted-source"),
        labels: nlu
            .intents
            .iter()
            .map(|i| i.name.clone())
            .chain(std::iter::once("other".into()))
            .collect(),
        catalog_source: None,
        files: vec![],
        nlu,
    }
}

fn fixture() -> (PathBuf, String, PathBuf, Artifact) {
    let data = std::env::temp_dir().join(format!("xz-nlu-artifact-{}", uuid::Uuid::new_v4()));
    let id = uuid::Uuid::new_v4().to_string();
    let dir = output_dir(&data, &id).unwrap();
    std::fs::create_dir_all(&dir).unwrap();
    let a = candidate(&id);
    std::fs::write(dir.join("model.onnx"), b"fixture-not-real-model").unwrap();
    std::fs::write(dir.join("tokenizer.json"), b"fixture-not-real-tokenizer").unwrap();
    std::fs::write(dir.join("labels.txt"), a.labels.join("\n") + "\n").unwrap();
    let a = a.collect(&dir).unwrap();
    write_manifest(&dir, &a).unwrap();
    (data, id, dir, a)
}

#[test]
fn manifest_checks_hash_labels_source_and_version_paths() {
    let (data, id, dir, a) = fixture();
    let (loaded, actual) = load_candidate(&data, &id).unwrap();
    assert_eq!(actual, dir.canonicalize().unwrap());
    assert_eq!(loaded.model_sha256(), sha256(b"fixture-not-real-model"));
    assert_eq!(loaded.source_revision, "saved-revision");
    assert_eq!(loaded.labels, a.labels);
    let wire = loaded.public_json(&actual);
    assert_eq!(wire["auto_activated"], false);
    assert_eq!(wire["trainer_script_sha256"], sha256(b"trusted-source"));
    assert!(wire.get("nlu").is_none());
    assert!(wire["expected_config"]["bert_model"]
        .as_str()
        .unwrap()
        .contains(&id));
}

#[test]
fn mutated_model_and_missing_manifest_are_not_candidates() {
    let (data, id, dir, _) = fixture();
    std::fs::write(dir.join("model.onnx"), b"mutated").unwrap();
    let err = load_candidate(&data, &id).err().unwrap().to_string();
    assert!(err.contains("integrity mismatch"), "{err}");
    let missing = uuid::Uuid::new_v4().to_string();
    std::fs::create_dir(output_dir(&data, &missing).unwrap()).unwrap();
    assert!(load_candidate(&data, &missing).is_err());
}

#[test]
fn traversal_external_and_noncanonical_ids_are_rejected() {
    let data = std::env::temp_dir();
    for id in [
        "../outside",
        "/tmp/model.onnx",
        "not-an-id",
        "00000000000000000000000000000000",
    ] {
        assert!(output_dir(&data, id).is_err());
    }
}

#[test]
fn labels_require_exact_source_order_and_uniqueness() {
    let (data, id, dir, _) = fixture();
    std::fs::write(dir.join("labels.txt"), "other\nother\n").unwrap();
    assert!(labels_at(&dir.canonicalize().unwrap()).is_err());
    std::fs::write(dir.join("labels.txt"), "other\nreminder_countdown\n").unwrap();
    assert!(candidate(&id).collect(&dir).is_err());
    assert!(load_candidate(&data, &id).is_err());
}

#[test]
fn independent_versions_do_not_overwrite_or_activate_old_model() {
    let (data, id, dir, a) = fixture();
    let other = uuid::Uuid::new_v4().to_string();
    assert_ne!(output_dir(&data, &other).unwrap(), dir);
    let before = serde_json::to_value(&a.nlu).unwrap();
    let isolated = a.config(&dir);
    assert!(isolated.bert_enabled);
    assert_eq!(serde_json::to_value(&a.nlu).unwrap(), before);
    assert_eq!(
        load_candidate(&data, &id).unwrap().0.model_sha256(),
        a.model_sha256()
    );
}

#[cfg(unix)]
#[test]
fn symlink_model_and_version_escape_are_rejected() {
    use std::os::unix::fs::symlink;
    let (data, _id, dir, _) = fixture();
    let external = std::env::temp_dir().join(format!("xz-nlu-external-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&external).unwrap();
    let id = uuid::Uuid::new_v4().to_string();
    symlink(&external, output_dir(&data, &id).unwrap()).unwrap();
    assert!(load_candidate(&data, &id).is_err());
    // A symlink in an otherwise managed version is never followed by candidate verification.
    symlink(dir.join("model.onnx"), external.join("model.onnx")).unwrap();
    assert!(controlled_file(&external.canonicalize().unwrap(), "model.onnx").is_err());
}
