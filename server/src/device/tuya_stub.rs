//! Pure-base Tuya compatibility shell.
//!
//! The full native Tuya data plane is a legacy-business module. Pure-base keeps
//! the configuration shape and an inert engine shell so shared runtime structs
//! remain source-compatible while no Tuya worker, route, or dependency is built.

#[path = "tuya/config.rs"]
pub mod config;

pub use config::TuyaConfig;

/// Inert placeholder used only by the shared `Engines` shape in pure-base.
#[derive(Debug, Default)]
pub struct TuyaEngine;

impl TuyaEngine {
    pub fn new(_dir: &std::path::Path) -> Self {
        Self
    }
}
