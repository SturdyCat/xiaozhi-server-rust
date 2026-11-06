//! 设备公共能力 helper 回归。

use super::shared::quantize_volume;

#[test]
fn quantize_volume_rounds_clamps_and_rejects_non_finite() {
    assert_eq!(quantize_volume(0.0), 0);
    assert_eq!(quantize_volume(0.4), 0);
    assert_eq!(quantize_volume(0.5), 1);
    assert_eq!(quantize_volume(63.6), 64);
    assert_eq!(quantize_volume(127.0), 127);
    assert_eq!(quantize_volume(200.0), 127);
    assert_eq!(quantize_volume(-5.0), 0);
    assert_eq!(quantize_volume(f32::NAN), 0);
    assert_eq!(quantize_volume(f32::INFINITY), 0);
}

#[test]
fn capability_ids_are_stable_and_device_scoped() {
    assert_eq!(
        super::ids::DEVICE_AUDIO_VOLUME_SET,
        "device.audio.volume.set"
    );
    assert!(super::ids::DEVICE_AUDIO_VOLUME_SET.starts_with("device."));
    assert!(super::ids::DEVICE_LIFECYCLE_RESTART.starts_with("device."));
}

#[test]
fn parse_volume_level_rejects_non_integer_and_out_of_range_values() {
    use super::shared::parse_volume_level;
    use serde_json::json;
    assert_eq!(parse_volume_level(&json!({"level": 127})), Some(127));
    assert_eq!(parse_volume_level(&json!({"level": 128})), None);
    assert_eq!(parse_volume_level(&json!({"level": 1.5})), None);
    assert_eq!(parse_volume_level(&json!({"level": -1})), None);
}

#[test]
fn adapter_support_matrix_is_explicit_and_native() {
    use super::registry::supported_capabilities;
    assert!(supported_capabilities("echomuse").contains(&super::ids::DEVICE_AUDIO_VOLUME_SET));
    assert!(supported_capabilities("esp_home").contains(&super::ids::DEVICE_AUDIO_MUTE_GET));
    assert_eq!(
        supported_capabilities("tuya"),
        &[super::ids::DEVICE_STATUS_GET]
    );
    assert!(supported_capabilities("unknown").is_empty());
}

#[test]
fn legacy_mcp_and_skill_actions_resolve_to_shared_capabilities() {
    use super::ids;
    use super::registry::capability_for_action;
    assert_eq!(
        capability_for_action("self.audio_speaker.set_volume"),
        Some(ids::DEVICE_AUDIO_VOLUME_SET)
    );
    assert_eq!(
        capability_for_action("device_volume_set"),
        Some(ids::DEVICE_AUDIO_VOLUME_SET)
    );
    assert_eq!(
        capability_for_action("self.reboot"),
        Some(ids::DEVICE_LIFECYCLE_RESTART)
    );
    assert_eq!(capability_for_action("unknown.action"), None);
}

#[test]
fn capability_descriptors_are_versioned_and_not_hot_path() {
    let descriptors = super::registry::CapabilityRegistry::descriptors();
    assert_eq!(descriptors.len(), 5);
    assert!(descriptors
        .iter()
        .all(|descriptor| descriptor.version == "1"));
    assert!(descriptors.iter().all(|descriptor| !descriptor.hot_path));
}

#[test]
fn registry_routes_native_capability_without_wasm() {
    use super::registry::{
        CapabilityError, CapabilityInput, CapabilityOutput, CapabilityRegistry,
        DeviceCapabilityAdapter,
    };
    use std::sync::Arc;

    struct FakeAudio;
    impl DeviceCapabilityAdapter for FakeAudio {
        fn adapter_id(&self) -> &'static str {
            "fake-audio"
        }
        fn supports(&self, capability: &str) -> bool {
            capability == super::ids::DEVICE_AUDIO_VOLUME_SET
        }
        fn call(
            &self,
            capability: &str,
            input: CapabilityInput,
        ) -> Result<CapabilityOutput, CapabilityError> {
            assert_eq!(capability, super::ids::DEVICE_AUDIO_VOLUME_SET);
            match input {
                CapabilityInput::Volume { level } => Ok(CapabilityOutput::Volume { level }),
                CapabilityInput::Empty => {
                    Err(CapabilityError::InvalidInput("volume required".into()))
                }
            }
        }
    }

    let registry = CapabilityRegistry::new(vec![Arc::new(FakeAudio)]);
    assert_eq!(registry.adapter_ids(), vec!["fake-audio"]);
    assert_eq!(
        registry.call_action(
            "self.audio_speaker.set_volume",
            CapabilityInput::Volume { level: 42 },
        ),
        Ok(CapabilityOutput::Volume { level: 42 })
    );
    assert!(matches!(
        registry.call("device.unknown.action", CapabilityInput::Empty),
        Err(CapabilityError::Unsupported { .. })
    ));
}
