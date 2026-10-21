//! Session-pinned playback policy. Legacy audio geometry remains restart-only.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlaybackMode {
    #[default]
    Interactive,
    Announcement,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PlaybackConfig {
    pub mode: PlaybackMode,
    pub announcement_start_ms: u32,
    pub announcement_start_max_ms: u32,
}

impl Default for PlaybackConfig {
    fn default() -> Self {
        Self {
            mode: PlaybackMode::Interactive,
            announcement_start_ms: 5000,
            announcement_start_max_ms: 10000,
        }
    }
}

impl PlaybackConfig {
    pub fn normalize(&mut self) {
        self.announcement_start_ms = self.announcement_start_ms.min(120_000);
        self.announcement_start_max_ms = self
            .announcement_start_max_ms
            .clamp(self.announcement_start_ms, 120_000);
    }
    /// Only startup gating changes; reservoir capacity and device queue safety
    /// never grow as a side effect of selecting announcement playback.
    pub fn apply(&self, params: &mut crate::app::session::SessionParams) {
        if self.mode == PlaybackMode::Announcement {
            params.downlink_buffer_start_ms =
                self.announcement_start_ms.min(params.downlink_buffer_ms);
            params.downlink_buffer_start_max_ms = self
                .announcement_start_max_ms
                .max(params.downlink_buffer_start_ms);
        }
    }
}

#[cfg(test)]
mod tests;
