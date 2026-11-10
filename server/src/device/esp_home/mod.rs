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
//! - **目录策略（阶段三第一切片）**：`[esp_home].engine = "wasm-policy"` 时，桥在绑定端口前
//!   经 `connector.esp-home` 扩展点向 WASM 策略插件征询「实体放行 + 显示名」决策
//!   （[`policy`]，fail-closed：插件不可用桥不启动）；协议握手、加密、状态采集与命令执行
//!   仍由 native 承载，key 语义不变；
//! - **生产接管（S3v）**：`[esp_home].engine = "wasm-session"` 时协议面（握手/加密/
//!   消息派发/实体应答）也整体迁入 esphome.official 插件（[`session`]）：宿主桥只做
//!   TCP accept、原始字节搬运、状态观测注入与命令执行；psk 改由插件配置命名空间的
//!   `noise_psk`（secret 字段）提供，`[esp_home].encryption_key` 不参与协议。
//!   缺 wasm-plugins feature / 缺宿主 / 缺插件、或与 `ble_proxy=true` 组合，桥不启动
//!   （fail-closed）；native 默认路径不变，native 删除仍以 P8c 真机 + HA 客户端回归
//!   为门槛；
//! - 编译期开关 `--features esp_home`，运行期开关 `[esp_home].enabled`；默认关闭，
//!   关闭时零依赖、零监听、零行为变化。
//!
//! 协议握手与 noise_psk 加密由 crate `esphome-native-api` 托管；本模块只补「实体列表 +
//! 状态应答 + 命令分发 + BLE 代理」。之所以不用该 crate 的高层 `esphomeserver::EspHomeServer`：
//! 它在 v3.0.0 仍是空壳（实体列表循环被 TODO 注释掉、从不发状态响应），详见 `server.rs`。

pub mod config;

pub use config::EspHomeConfig;
pub use config::ESPHOME_ENGINES;

#[cfg(feature = "esp_home")]
pub mod actions;
#[cfg(feature = "esp_home")]
pub mod bluetooth;
#[cfg(feature = "wasm-plugins")]
pub mod policy;
#[cfg(feature = "esp_home")]
pub mod server;
/// S3v：`engine = "wasm-session"` 的连接编排（协议面进插件；依赖 WASM 宿主）。
#[cfg(all(feature = "esp_home", feature = "wasm-plugins"))]
pub mod session;
#[cfg(feature = "esp_home")]
pub mod state;

#[cfg(all(test, feature = "wasm-plugins"))]
mod policy_tests;

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
