//! ESPHome Native API 桥：把 emOS / EchoMuse Dot 2 暴露为标准 ESPHome 节点。
//!
//! ## 范围
//!
//! - **状态面（阶段 A）**：只读 `binary_sensor`（在线 / 聆听 / 思考 / 播报 / 静音），
//!   验证「发现 → 实体列表 → 状态回推」闭环；
//! - **控制面（阶段 B）**：可写 `number`（音量 0..127，命令经 [`actions`] 翻成 EchoMuse
//!   的 `volume_set` 下发）。**静音只有上报没有下发**（协议无 `mute_set`），故为只读传感器；
//! - **`bluetooth_proxy`**：把 Dot 2 的 BLE 无线电暴露给 native-api 客户端（[`bluetooth`]）。
//!   能力边界严格按设备侧实测结果宣告——配对/清缓存/描述符读写设备没有，就不报；
//! - 编译期开关 `--features esp_home`，运行期开关 `[esp_home].enabled`；默认关闭，
//!   关闭时零依赖、零监听、零行为变化。
//!
//! 协议握手与 noise_psk 加密由 crate `esphome-native-api` 托管；本模块只补「实体列表 +
//! 状态应答 + 命令分发 + BLE 代理」。之所以不用该 crate 的高层 `esphomeserver::EspHomeServer`：
//! 它在 v3.0.0 仍是空壳（实体列表循环被 TODO 注释掉、从不发状态响应），详见 `server.rs`。

pub mod config;

#[cfg(feature = "esp_home")]
pub mod actions;
#[cfg(feature = "esp_home")]
pub mod bluetooth;
#[cfg(feature = "esp_home")]
pub mod server;
#[cfg(feature = "esp_home")]
pub mod state;

pub use config::EspHomeConfig;

/// `[esp_home].enabled` 时由 `main` 调用：后台起桥，失败只记日志（不阻断主服务）。
///
/// `engines` 同时提供两样东西：`registry`（Dot 2 是否在线的唯一真相源）与 `config`
/// （BLE 代理要经 `device/echomuse` 的门面访问设备，那里需要全局配置）。
#[cfg(feature = "esp_home")]
pub fn spawn(cfg: EspHomeConfig, engines: std::sync::Arc<crate::runtime::Engines>) {
    tokio::spawn(async move {
        if let Err(e) = server::serve(cfg, engines).await {
            tracing::error!("[esp_home] 桥退出：{e:#}");
        }
    });
}
