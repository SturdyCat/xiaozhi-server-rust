//! Pure-base EchoMuse compatibility shell.
//!
//! The native EchoMuse device plane is a legacy-business module. Pure-base keeps
//! only configuration and inert observation helpers required by shared code;
//! no device routes, sockets, mDNS, BLE, or media code are compiled.

#[path = "echomuse/config.rs"]
pub mod config;

pub use config::{EchomuseConfig, ECHOMUSE_ENGINES};

pub mod media {
    pub(crate) const AUDIO_QUEUE_MS: u64 = 2_000;
    pub(crate) const AUDIO_QUEUE_BYTES: usize = 256 * 1024;
}

pub mod registry {
    use std::path::PathBuf;

    pub fn data_dir(cfg: &crate::config::Config) -> PathBuf {
        std::path::Path::new(&cfg.memory.db_path)
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .map_or_else(|| PathBuf::from("/data"), |p| p.to_path_buf())
    }
}

pub mod ble {
    use tokio::sync::watch;

    pub const GATT_WAIT: std::time::Duration = std::time::Duration::from_secs(10);

    pub fn status_version() -> watch::Receiver<u64> {
        let (_tx, rx) = watch::channel(0_u64);
        rx
    }
}

pub fn subscribe_devices_version() -> tokio::sync::watch::Receiver<u64> {
    ble::status_version()
}

pub fn hub_status() -> Vec<(String, serde_json::Value)> {
    Vec::new()
}
