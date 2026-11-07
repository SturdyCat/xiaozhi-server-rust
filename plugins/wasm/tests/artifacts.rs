//! 插件产物落盘与完整性测试（digest 稳定 / Ed25519 attestation 校验）。
//!
//! 从原 bootstrap 测试迁移而来：服务端已不再做 Docker 自动构建，但产物层
//! （digest / 钉版 / Ed25519 attestation admission）仍是上传即启用模型的核心，
//! 其覆盖须保留。

use crate::config::PluginEntryConfig;
use crate::plugins::wasm::artifacts::ArtifactDir;

#[test]
fn artifact_digest_is_stable_and_detects_tampering() {
    let bytes = b"wasm-bytes";
    let digest = ArtifactDir::digest(bytes);
    assert_eq!(digest.len(), 64);
    assert!(PluginEntryConfig::valid_artifact_pin(&digest));
    assert_eq!(digest, ArtifactDir::digest(bytes));
    assert_ne!(digest, ArtifactDir::digest(b"tampered"));

    let dir = std::env::temp_dir().join(format!("xz-artifact-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let artifacts = ArtifactDir::for_location(dir.join("config.db").to_str()).unwrap();
    artifacts.save("demo.plugin", bytes).unwrap();
    assert_eq!(
        artifacts.digest_for("demo.plugin").as_deref(),
        Some(digest.as_str())
    );
}

#[test]
fn artifact_attestation_rejects_malformed_or_wrong_signature() {
    let digest = ArtifactDir::digest(b"wasm-bytes");
    assert!(!ArtifactDir::verify_attestation(
        "not-base64",
        "not-base64",
        &digest
    ));
    // RFC 8032 test vector 1: prove the positive Ed25519 path over an exact message.
    assert!(ArtifactDir::verify_attestation(
        "11qYAYKxCrfVS/7TyWQHOg7hcvPapiMlrwIaaPcHURo=",
        "5VZDAMNgrHKQhuLMgG6CioSHfx645dl02HPgZSJJAVVfuIIVkKM7rMYeOXAc+bRr0lv18FlbviRlUUFDjnoQCw==",
        "",
    ));
    assert!(!ArtifactDir::verify_attestation(
        "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=",
        "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA==",
        &digest,
    ));
}
