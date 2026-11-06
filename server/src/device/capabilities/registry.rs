//! Native device capability routing.
//!
//! This registry is for low-frequency device actions (restart, volume, status), not
//! PCM/VAD/inference/WS/Tuya LAN hot paths. Adapters own their protocol and secrets;
//! the registry only performs capability routing and preserves structured errors.

use std::sync::Arc;

use super::ids;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CapabilityDescriptor {
    pub id: &'static str,
    pub version: &'static str,
    pub hot_path: bool,
}

/// Low-frequency device actions intentionally stay outside the WASM hot path.
pub const CAPABILITIES: &[CapabilityDescriptor] = &[
    CapabilityDescriptor {
        id: ids::DEVICE_AUDIO_VOLUME_GET,
        version: "1",
        hot_path: false,
    },
    CapabilityDescriptor {
        id: ids::DEVICE_AUDIO_VOLUME_SET,
        version: "1",
        hot_path: false,
    },
    CapabilityDescriptor {
        id: ids::DEVICE_AUDIO_MUTE_GET,
        version: "1",
        hot_path: false,
    },
    CapabilityDescriptor {
        id: ids::DEVICE_LIFECYCLE_RESTART,
        version: "1",
        hot_path: false,
    },
    CapabilityDescriptor {
        id: ids::DEVICE_STATUS_GET,
        version: "1",
        hot_path: false,
    },
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CapabilityInput {
    Empty,
    Volume { level: u8 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CapabilityOutput {
    Accepted,
    Volume { level: u8 },
    Status { online: bool },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CapabilityError {
    InvalidInput(String),
    Unsupported { capability: String },
    Offline,
    Failed(String),
}

/// A hardware adapter implements only the capabilities its protocol truly supports.
/// It must not expose credentials, sockets, or raw frames through this trait.
pub trait DeviceCapabilityAdapter: Send + Sync {
    fn adapter_id(&self) -> &'static str;
    fn supports(&self, capability: &str) -> bool;
    fn call(
        &self,
        capability: &str,
        input: CapabilityInput,
    ) -> Result<CapabilityOutput, CapabilityError>;
}

#[derive(Default, Clone)]
pub struct CapabilityRegistry {
    adapters: Vec<Arc<dyn DeviceCapabilityAdapter>>,
}

impl CapabilityRegistry {
    pub fn new(adapters: Vec<Arc<dyn DeviceCapabilityAdapter>>) -> Self {
        Self { adapters }
    }

    pub fn adapter_ids(&self) -> Vec<&'static str> {
        self.adapters
            .iter()
            .map(|adapter| adapter.adapter_id())
            .collect()
    }

    pub fn descriptors() -> &'static [CapabilityDescriptor] {
        CAPABILITIES
    }

    /// Dispatch an existing MCP/skill action through the same native registry.
    pub fn call_action(
        &self,
        action: &str,
        input: CapabilityInput,
    ) -> Result<CapabilityOutput, CapabilityError> {
        let capability =
            capability_for_action(action).ok_or_else(|| CapabilityError::Unsupported {
                capability: action.to_string(),
            })?;
        self.call(capability, input)
    }

    /// Dispatch a low-frequency native action. The first supporting adapter owns
    /// protocol execution; no WASM dispatch is inserted into this path.
    pub fn call(
        &self,
        capability: &str,
        input: CapabilityInput,
    ) -> Result<CapabilityOutput, CapabilityError> {
        if !is_known_capability(capability) {
            return Err(CapabilityError::Unsupported {
                capability: capability.to_string(),
            });
        }
        self.adapters
            .iter()
            .find(|adapter| adapter.supports(capability))
            .ok_or_else(|| CapabilityError::Unsupported {
                capability: capability.to_string(),
            })?
            .call(capability, input)
    }
}

/// Static adapter support matrix. Protocol handlers remain in each device module.
pub fn supported_capabilities(adapter_id: &str) -> &'static [&'static str] {
    match adapter_id {
        "echomuse" | "esp_home" => &[
            ids::DEVICE_AUDIO_VOLUME_GET,
            ids::DEVICE_AUDIO_VOLUME_SET,
            ids::DEVICE_AUDIO_MUTE_GET,
            ids::DEVICE_STATUS_GET,
        ],
        "tuya" => &[ids::DEVICE_STATUS_GET],
        _ => &[],
    }
}

/// Resolve existing MCP/skill names to the shared capability ID.
/// The aliases preserve the public device/tool contract while converging execution.
pub fn capability_for_action(action: &str) -> Option<&'static str> {
    match action {
        "self.audio_speaker.set_volume" | "device_volume_set" => Some(ids::DEVICE_AUDIO_VOLUME_SET),
        "self.reboot" | "device_restart" => Some(ids::DEVICE_LIFECYCLE_RESTART),
        "device_mute" => Some(ids::DEVICE_AUDIO_MUTE_GET),
        _ => None,
    }
}

fn is_known_capability(capability: &str) -> bool {
    matches!(
        capability,
        ids::DEVICE_AUDIO_VOLUME_GET
            | ids::DEVICE_AUDIO_VOLUME_SET
            | ids::DEVICE_AUDIO_MUTE_GET
            | ids::DEVICE_LIFECYCLE_RESTART
            | ids::DEVICE_STATUS_GET
    )
}
