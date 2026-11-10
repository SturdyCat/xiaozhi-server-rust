//! ESPHome Native API 桥配置段（`[esp_home]`）。
//!
//! 默认 `enabled=false`：不监听、零行为变化。启用后由
//! [`super::server`] 在 `port`（默认 6053）上提供 ESPHome Native API，把 Dot 2 状态
//! 暴露为标准 `binary_sensor` 实体（阶段 A）。协议实现见 crate `esphome-native-api`。

use serde::{Deserialize, Serialize};

use crate::config::{canonical_engine, EngineSpec};

/// `[esp_home].engine` 可选实现（配置层权威表，与 tuya/echomuse 同口径）。
///
/// `wasm-policy` = 桥启动期经 `connector.esp-home` 扩展点向 WASM 插件征询实体目录
/// 放行/显示名决策；协议握手、加密、状态采集与命令执行仍由 native 承载（阶段三
/// ESPHome 第一切片，数据面不出原生）。
///
/// `wasm-session`（S3v 生产接管）= 目录决策之外，**协议面（握手/加密/消息派发/实体
/// 应答）也整体迁入 esphome.official 插件**：宿主桥只做 TCP accept、原始字节搬运、
/// 状态观测注入与命令执行。psk 不再来自 `[esp_home].encryption_key`，而是插件配置
/// 命名空间的 `noise_psk`（secret 字段）。缺 wasm-plugins feature / 缺宿主 /
/// 缺插件、或与 `ble_proxy=true` 组合，桥**不启动**（fail-closed，不静默降级）。
pub const ESPHOME_ENGINES: &[EngineSpec] = &[
    EngineSpec {
        id: "native",
        aliases: &[],
    },
    EngineSpec {
        id: "wasm-policy",
        aliases: &["wasm"],
    },
    EngineSpec {
        id: "wasm-session",
        aliases: &[],
    },
];
const DEFAULT_ESPHOME_ENGINE: &str = "native";

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct EspHomeConfig {
    /// 桥实现：`native`（宿主全原生）、`wasm-policy`（启动期实体目录经 WASM 策略
    /// 插件决策）或 `wasm-session`（S3v：协议面也进插件，psk 经插件配置）。
    /// 旧值 `wasm` 仍可读；均为 RestartOnly（桥是启动期拉起的后台任务）。
    #[serde(default = "default_engine")]
    pub engine: String,
    /// 是否启用桥。**默认关闭**（可选模块，不影响现有语音/设备路径）。
    #[serde(default)]
    pub enabled: bool,
    /// 监听端口（ESPHome 官方默认 6053）。
    #[serde(default = "default_port")]
    pub port: u16,
    /// noise_psk（base64 编码的 32 字节）；空 = 明文（仅可信局域网）。
    #[serde(default)]
    pub encryption_key: String,
    /// 设备节点名（ESPHome 的 `name`：mDNS 实例名与主机名前缀）。
    #[serde(default = "default_device_name")]
    pub device_name: String,
    /// 友好名（HA 卡片显示名）。
    #[serde(default = "default_friendly_name")]
    pub friendly_name: String,
    /// 型号。
    #[serde(default = "default_model")]
    pub model: String,
    /// 厂商。
    #[serde(default = "default_manufacturer")]
    pub manufacturer: String,
    /// 建议区域（HA 首次添加时的默认区域）。
    #[serde(default)]
    pub suggested_area: String,
    /// 是否广播 mDNS `_esphomelib._tcp`（便于 HA 自动发现；失败只告警，可 IP 手动添加）。
    #[serde(default = "default_mdns")]
    pub mdns: bool,
    /// 是否向客户端宣告 `bluetooth_proxy` 能力（Dot 2 当 BLE 代理）。
    ///
    /// **默认开启**，但实际生效还需 `[echomuse].ble_proxy` 也为真——那个才是设备侧
    /// BLE 扫描/连接的总闸，桥只是把已经打开的能力如实宣告出去。二者任一为假时
    /// 桥不宣告，避免客户端点进来才发现设备没开 BLE（§7：不谎报能力）。
    #[serde(default = "default_ble_proxy")]
    pub ble_proxy: bool,
}

impl Default for EspHomeConfig {
    fn default() -> Self {
        Self {
            engine: default_engine(),
            enabled: false,
            port: default_port(),
            encryption_key: String::new(),
            device_name: default_device_name(),
            friendly_name: default_friendly_name(),
            model: default_model(),
            manufacturer: default_manufacturer(),
            suggested_area: String::new(),
            mdns: default_mdns(),
            ble_proxy: default_ble_proxy(),
        }
    }
}

impl EspHomeConfig {
    /// 配置规范化（幂等）：端口/名称非法时回落默认，密钥去空白，引擎别名收敛。
    ///
    /// `encryption_key` 的 base64 合法性不在这里校验——由 `server::serve` 启动前
    /// 用 crate 的 `build()` 探测（同样口径，避免两处各写一套 base64 规则）。
    pub fn normalize(&mut self) {
        // 纯底座构建（未编 native-plugins）即使复用了带 `[esp_home].enabled=true`
        // 的数据库也保持惰性：桥依赖 EchoMuse 设备入口，底座里它不存在。
        if cfg!(not(feature = "native-plugins")) {
            self.enabled = false;
        }
        self.engine = canonical_engine(&self.engine, ESPHOME_ENGINES, DEFAULT_ESPHOME_ENGINE);
        if self.port == 0 {
            tracing::warn!("[esp_home].port=0 非法（无法监听），已回落到 6053");
            self.port = default_port();
        }
        self.device_name = self.device_name.trim().to_string();
        if self.device_name.is_empty() {
            self.device_name = default_device_name();
        }
        self.friendly_name = self.friendly_name.trim().to_string();
        if self.friendly_name.is_empty() {
            self.friendly_name = default_friendly_name();
        }
        self.encryption_key = self.encryption_key.trim().to_string();
    }

    /// 规范化后的实现 id（未知值原样小写，交由注册表 fail-loud 校验）。
    pub fn engine_id(&self) -> String {
        canonical_engine(&self.engine, ESPHOME_ENGINES, DEFAULT_ESPHOME_ENGINE)
    }
}

fn default_engine() -> String {
    "native".into()
}
fn default_port() -> u16 {
    6053
}
fn default_device_name() -> String {
    "xiaozhi-dot2".into()
}
fn default_friendly_name() -> String {
    "EchoMuse Dot 2".into()
}
fn default_model() -> String {
    "EchoMuse Dot 2".into()
}
fn default_manufacturer() -> String {
    "xiaozhi".into()
}
fn default_mdns() -> bool {
    true
}
fn default_ble_proxy() -> bool {
    true
}
