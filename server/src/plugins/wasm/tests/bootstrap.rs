//! 默认插件引导测试（不调 Docker：解析、跳过与降级路径）。

use crate::plugins::wasm::artifacts::ArtifactDir;
use crate::plugins::wasm::bootstrap::{load_manifest, BootstrapEntry};
use crate::plugins::wasm::tests::fast_limits;
use crate::plugins::wasm::WasmHost;

fn repo_root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .to_path_buf()
}

#[test]
fn bootstrap_manifest_parses_data_driven_entries() {
    let entries = load_manifest(&repo_root()).expect("引导表应可解析");
    // 数据表（非代码）：weather 是当前唯一官方默认插件
    assert_eq!(
        entries,
        vec![BootstrapEntry {
            id: "weather.official".into(),
            dir: "weather".into()
        }],
        "{entries:?}"
    );
}

#[test]
fn ensure_plugin_skips_when_artifact_present() {
    let dir = std::env::temp_dir().join(format!("xz-boot-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let artifacts = ArtifactDir::for_location(
        dir.join("config.db")
            .to_str()
            .map(str::to_string)
            .as_deref(),
    )
    .expect("有效位置");
    artifacts.save("weather.official", b"placeholder").unwrap();

    let host = WasmHost::new(fast_limits()).expect("host");
    let entry = BootstrapEntry {
        id: "weather.official".into(),
        dir: "weather".into(),
    };
    // 产物已在 → 直接跳过（不构建、不装载 placeholder 字节——scan 才是装载路径）
    let built = crate::plugins::wasm::bootstrap::ensure_plugin(
        &std::sync::Arc::new(host),
        &artifacts,
        &entry,
        &repo_root(),
        false,
    )
    .expect("应跳过");
    assert!(!built);
}

#[test]
fn artifact_digest_is_stable_and_detects_tampering() {
    let bytes = b"wasm-bytes";
    let digest = ArtifactDir::digest(bytes);
    assert_eq!(digest.len(), 64);
    assert!(crate::config::PluginEntryConfig::valid_artifact_pin(
        &digest
    ));
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
fn ensure_plugin_degrades_without_docker() {
    let dir = std::env::temp_dir().join(format!("xz-boot-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let artifacts = ArtifactDir::for_location(
        dir.join("config.db")
            .to_str()
            .map(str::to_string)
            .as_deref(),
    )
    .expect("有效位置");

    let host = std::sync::Arc::new(WasmHost::new(fast_limits()).expect("host"));
    let entry = BootstrapEntry {
        id: "weather.official".into(),
        dir: "weather".into(),
    };
    // 源在、产物缺、Docker 不可用 → 明确降级（指向上传替代），不是 panic
    let err = crate::plugins::wasm::bootstrap::ensure_plugin(
        &host,
        &artifacts,
        &entry,
        &repo_root(),
        false,
    )
    .unwrap_err();
    let msg = format!("{err:#}");
    assert!(msg.contains("Docker 不可用"), "{msg}");
    assert!(msg.contains("上传"), "应指明替代路径：{msg}");
}
