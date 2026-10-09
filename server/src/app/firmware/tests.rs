//! `server/src/app/firmware.rs` 的单元测试。
//!
//! 自该文件的内联 `#[cfg(test)] mod tests` 外移（AGENTS.md §5.10：测试一律独立文件），
//! 外层只保留 `#[cfg(test)]` 模块声明。

use super::*;

#[test]
fn sanitize_version_blocks_path_and_keeps_semver() {
    assert_eq!(sanitize_version(" 2.0.4 "), Some("2.0.4".into()));
    assert_eq!(sanitize_version("2.0.4-beta1"), Some("2.0.4-beta1".into()));
    // 路径穿越 / 注入：一律拒绝（版本会拼进文件名与下载 URL）
    assert_eq!(sanitize_version("../etc/passwd"), None);
    assert_eq!(sanitize_version("2.0.4/x"), None);
    assert_eq!(sanitize_version("2.0.4?x=1"), None);
    assert_eq!(sanitize_version(""), None);
}

#[test]
fn latest_roundtrip_from_disk() {
    let dir = std::env::temp_dir().join(format!("xz_fw_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    // env 是进程级：本测试独占该变量，先设后清
    std::env::set_var("XIAOZHI_FIRMWARE_DIR", &dir);
    assert!(latest().is_none(), "无 latest.json 应为 None");
    let meta = FirmwareMeta {
        version: "2.0.4".into(),
        size: 3,
        sha256: "abc".into(),
        uploaded_at: 1,
    };
    std::fs::write(dir.join("latest.json"), serde_json::to_string(&meta).unwrap()).unwrap();
    let got = latest().unwrap();
    assert_eq!(got.version, "2.0.4");
    // 损坏元数据：降级为 None 而非 panic
    std::fs::write(dir.join("latest.json"), "{broken").unwrap();
    assert!(latest().is_none());
    std::env::remove_var("XIAOZHI_FIRMWARE_DIR");
    let _ = std::fs::remove_dir_all(&dir);
}
