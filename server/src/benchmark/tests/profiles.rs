use super::*;

#[tokio::test]
async fn media_profiles_use_generic_builders_without_playback_claims() {
    let mut cfg = Config::default();
    cfg.audio.downlink_buffer_ms = 1000;
    cfg.audio.downlink_lead_ms = 200;
    cfg.playback.mode = crate::config::playback::PlaybackMode::Announcement;
    cfg.playback.announcement_start_ms = 5000;
    let factory = Arc::new(FakeProviders::default());
    let (service, store) = service(&cfg, factory.clone());
    let out = service.run(request("output.profile", None)).await;
    assert_eq!(out.result.state, "ok");
    assert_eq!(out.result.readiness, "configured");
    assert_eq!(out.result.details["played"], false);
    assert_eq!(out.result.details["socket_written"], false);
    assert_eq!(out.result.details["opus_verified"], false);
    let profiles = out.result.details["terminal_defaults"].as_array().unwrap();
    assert_eq!(profiles.len(), 2);
    for profile in profiles {
        assert_eq!(profile["terminal"], "generic");
        assert_eq!(profile["clamps"]["requested_start_ms"], 5000);
        assert_eq!(profile["clamps"]["start_clamped"], true);
        assert!(profile["queue"]["transport_ms"].is_null());
        assert!(profile["queue"]["transport_bytes"].is_null());
        assert_eq!(profile["negotiated"], false);
        assert_eq!(profile["played"], false);
        assert_eq!(profile["codec_verified"], false);
    }
    assert!(factory.seen.lock().unwrap().is_empty());
    assert_eq!(store.writes.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn capability_validation_errors_do_not_reflect_raw_config() {
    let mut cfg = Config::default();
    cfg.nlu=serde_json::from_value(json!({"enabled":true,"intents":[{"name":"invalid","templates":["hello"],"action":{"kind":"https://user:password@provider.test/sk-private"}}]})).unwrap();
    let (service, _) = service(&cfg, Arc::new(FakeProviders::default()));
    let caps = service.capabilities().await;
    let wire = serde_json::to_string(&caps).unwrap();
    assert!(!wire.contains("password"));
    assert!(!wire.contains("sk-private"));
    let nlu = caps["actions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["action_id"] == "nlu.classify")
        .unwrap();
    assert_eq!(nlu["available"], false);
}
