//! `[tuya]` 配置段单测：normalize 收敛 + 凭证判定 + 指纹稳定性。

use super::*;

#[test]
fn default_engine_is_native() {
    let cfg = TuyaConfig::default();
    assert_eq!(cfg.engine_id(), "native");
}

#[test]
fn normalize_converges_engine_aliases() {
    for (raw, expected) in [
        ("backend", "native"),
        ("LAN", "native"),
        (" wasm ", "wasm-policy"),
    ] {
        let mut cfg = TuyaConfig {
            engine: raw.into(),
            ..TuyaConfig::default()
        };
        cfg.normalize();
        assert_eq!(cfg.engine, expected);
        assert_eq!(cfg.engine_id(), expected);
    }
}

#[test]
fn serde_accepts_legacy_backend_key() {
    let cfg: TuyaConfig = toml::from_str("backend = \"lan\"").expect("legacy backend key");
    assert_eq!(cfg.engine_id(), "native");
}

#[test]
fn normalize_preserves_unknown_engine_for_registry_validation() {
    let mut cfg = TuyaConfig {
        engine: "future".into(),
        ..TuyaConfig::default()
    };
    cfg.normalize();
    assert_eq!(cfg.engine, "future");
    assert_eq!(cfg.engine_id(), "future");
}

#[test]
fn normalize_trims_and_converges_region() {
    let mut cfg = TuyaConfig {
        region: " CN ".into(),
        access_id: " id ".into(),
        access_secret: " sec ".into(),
        sync_interval_secs: 10,
        ..TuyaConfig::default()
    };
    cfg.normalize();
    assert_eq!(cfg.region, "cn");
    assert_eq!(cfg.access_id, "id");
    assert_eq!(cfg.access_secret, "sec");
    // 小于 60s 的显式配置收敛到 60（0 = 仅手动保持原样）
    assert_eq!(cfg.sync_interval_secs, 60);
}

#[test]
fn normalize_unknown_region_falls_back_to_cn() {
    let mut cfg = TuyaConfig {
        region: "mars".into(),
        ..TuyaConfig::default()
    };
    cfg.normalize();
    assert_eq!(cfg.region, "cn");
}

#[test]
fn normalize_keeps_zero_sync_interval() {
    let mut cfg = TuyaConfig {
        sync_interval_secs: 0,
        ..TuyaConfig::default()
    };
    cfg.normalize();
    assert_eq!(cfg.sync_interval_secs, 0);
}

#[test]
fn credentials_ready_requires_both_parts() {
    let mut cfg = TuyaConfig::default();
    assert!(!cfg.credentials_ready());
    cfg.access_id = "id".into();
    assert!(!cfg.credentials_ready());
    cfg.access_secret = "sec".into();
    assert!(cfg.credentials_ready());
}

#[test]
fn fingerprint_is_stable_and_sensitive() {
    let mut a = TuyaConfig {
        region: "cn".into(),
        access_id: "id".into(),
        access_secret: "sec".into(),
        ..TuyaConfig::default()
    };
    let b = a.clone();
    assert_eq!(a.credentials_fingerprint(), b.credentials_fingerprint());
    // 任一凭证字段变化都改变指纹（热生效检测的依据）
    a.access_secret = "sec2".into();
    assert_ne!(a.credentials_fingerprint(), b.credentials_fingerprint());
    a = b.clone();
    a.access_id = "id2".into();
    assert_ne!(a.credentials_fingerprint(), b.credentials_fingerprint());
    a = b.clone();
    a.region = "eu".into();
    assert_ne!(a.credentials_fingerprint(), b.credentials_fingerprint());
}
