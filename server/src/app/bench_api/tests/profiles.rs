use super::*;

#[tokio::test]
async fn media_profiles_use_native_builders_and_show_clamps_without_playback_claims() {
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
    let echo = profiles
        .iter()
        .find(|p| p["terminal"] == "echomuse")
        .unwrap();
    let esp = profiles
        .iter()
        .find(|p| p["terminal"] == "esp_default")
        .unwrap();
    assert_eq!(echo["output"]["codec"], "pcm_s16le");
    assert_eq!(echo["output"]["sample_rate"], 48000);
    assert_eq!(echo["input"]["sample_rate"], 16000);
    assert_eq!(echo["output"]["frame_ms"], 40);
    assert_eq!(echo["output"]["codec_version"], 1);
    assert_eq!(echo["effective"]["lead_ms"], 0);
    assert_eq!(echo["effective"]["start_ms"], 1000);
    assert_eq!(echo["clamps"]["requested_start_ms"], 5000);
    assert_eq!(echo["clamps"]["start_clamped"], true);
    assert_eq!(
        echo["queue"]["transport_ms"],
        crate::app::echomuse::media::AUDIO_QUEUE_MS
    );
    assert_eq!(
        echo["queue"]["transport_bytes"],
        crate::app::echomuse::media::AUDIO_QUEUE_BYTES
    );
    assert!(echo["queue"]["device_capacity_ms"].is_null());
    assert!(echo["binary_protocol_version"].is_null());
    assert_eq!(esp["output"]["codec"], "opus");
    for p in profiles {
        assert_eq!(p["negotiated"], false);
        assert_eq!(p["played"], false);
        assert_eq!(p["codec_verified"], false);
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
