use super::*;
#[test]
fn old_config_is_interactive_and_invalid_mode_is_rejected() {
    let cfg: PlaybackConfig = serde_json::from_str("{}").unwrap();
    assert_eq!(cfg.mode, PlaybackMode::Interactive);
    assert_eq!(cfg.announcement_start_ms, 5000);
    assert_eq!(cfg.announcement_start_max_ms, 10000);
    assert!(serde_json::from_str::<PlaybackConfig>(r#"{"mode":"unknown"}"#).is_err());
}
#[test]
fn normalize_is_bounded_and_idempotent() {
    let mut cfg = PlaybackConfig {
        mode: PlaybackMode::Announcement,
        announcement_start_ms: u32::MAX,
        announcement_start_max_ms: 0,
    };
    cfg.normalize();
    cfg.normalize();
    assert_eq!(cfg.announcement_start_ms, 120000);
    assert_eq!(cfg.announcement_start_max_ms, 120000);
}

#[test]
fn policy_is_session_pinned_and_never_expands_reservoir_or_device_queues() {
    use crate::media::media::AudioCodec;
    use crate::workflow::session::SessionParams;
    let mut cfg = crate::config::Config::default();
    cfg.audio.downlink_buffer_ms = 800;
    cfg.audio.downlink_buffer_start_ms = 400;
    cfg.audio.downlink_buffer_start_max_ms = 900;
    let old = SessionParams::configured_defaults(&cfg, AudioCodec::Opus);
    cfg.playback.mode = PlaybackMode::Announcement;
    let next = SessionParams::configured_defaults(&cfg, AudioCodec::Opus);
    let echo = SessionParams::configured_defaults(&cfg, AudioCodec::PcmS16Le);
    assert_eq!(old.downlink_buffer_start_ms, 400);
    assert_eq!(next.downlink_buffer_start_ms, 800);
    assert_eq!(next.downlink_buffer_start_max_ms, 10000);
    assert_eq!(next.downlink_buffer_ms, old.downlink_buffer_ms);
    assert_eq!(next.downlink_lead_ms, old.downlink_lead_ms);
    assert_eq!(echo.downlink_frame_ms, 40);
    // EchoMuse speaker 线缆是 48kHz mono（设备不重采样，16k 会被播成 3 倍速）。
    assert_eq!(echo.downlink_sr, 48000);
    assert_eq!(echo.downlink_lead_ms, 0);
    assert_eq!(
        crate::plugins::registry::section_hot("playback"),
        crate::plugins::registry::HotReload::NextSession
    );
    for section in ["audio", "echomuse"] {
        assert_eq!(
            crate::plugins::registry::section_hot(section),
            crate::plugins::registry::HotReload::RestartOnly
        );
    }
}
