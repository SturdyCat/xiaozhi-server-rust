//! 设备公共能力层：跨硬件适配器复用的 native 契约与纯逻辑 helper。
//!
//! 这里描述的是接入设备提供的能力，不是服务器自身的“系统能力”。
//! EchoMuse、Xiaozhi ESP、ESPHome 等 adapter 可以实现同一能力；业务技能和
//! 业务插件复用此层，避免重复实现音量、生命周期、状态和参数边界。
//!
//! 性能边界：PCM、VAD、推理、WS/keepalive、Tuya LAN worker 等 hot path
//! 直接留在 native adapter，不经 WASM。只有低频、可审计且确有替换需求的
//! control path 才允许由 WASM host effect 间接调用本层。

pub mod registry;
pub mod shared;

/// 稳定的设备公共能力命名空间；当前只作为 native 路由和文档的单一口径。
pub mod ids {
    pub const DEVICE_AUDIO_VOLUME_GET: &str = "device.audio.volume.get";
    pub const DEVICE_AUDIO_VOLUME_SET: &str = "device.audio.volume.set";
    pub const DEVICE_AUDIO_MUTE_GET: &str = "device.audio.mute.get";
    pub const DEVICE_LIFECYCLE_RESTART: &str = "device.lifecycle.restart";
    pub const DEVICE_STATUS_GET: &str = "device.status.get";
}

#[cfg(test)]
mod tests;
