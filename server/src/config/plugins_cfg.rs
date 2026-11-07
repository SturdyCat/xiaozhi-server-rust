//! WASM 插件平台配置段（`[plugins]`）：平台总闸 + 每插件激活意图/钉版/签名。
//!
//! 热生效档位：RestartOnly（装载/引导都在启动期；激活/停用端点即时写盘 +
//! 立即装载/卸载实例）。每插件条目键 = 插件 id（id 含点号，TOML 用引号键：
//! `[plugins.entries."weather.official"]`）。**缺省不激活**（2026-11-07 生命周期
//! 语义）：`enabled` = 激活意图，上传只登记为 Staged、注册后仍不运行——激活端点
//! 写 true 并立即装载，停用端点写 false 并卸载实例。

use serde::{Deserialize, Serialize};

/// WASM 插件平台配置段（`[plugins]`）。热生效档位：RestartOnly（装载/引导都在启动期；
/// 激活/停用端点即时写盘 + 立即装载/卸载实例）。
///
/// 每插件条目键 = 插件 id（id 含点号，TOML 用引号键：`[plugins.entries."weather.official"]`）。
/// **缺省不激活**（2026-11-07 生命周期语义）：`enabled` = 激活意图，上传只登记为
/// Staged、注册后仍不运行——激活端点写 true 并立即装载，停用端点写 false 并卸载实例。
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(default)]
pub struct PluginsConfig {
    /// 平台总闸：false = 完全跳过产物装载与启动引导（宿主空表照常可启动）。
    pub enabled: bool,
    /// 严格完整性模式：true 时未配置 artifact SHA-256 pin 的产物不装载/不引导。
    /// 默认 false 保持 P3 以前的旧配置兼容语义。
    pub require_artifact_pin: bool,
    /// Ed25519 公钥（标准 base64）；配置后每个启用条目都必须提供 attestation。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attestation_public_key: Option<String>,
    /// 每插件条目（键 = 插件 id）。
    pub entries: std::collections::HashMap<String, PluginEntryConfig>,
}

impl Default for PluginsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            require_artifact_pin: false,
            attestation_public_key: None,
            entries: std::collections::HashMap::new(),
        }
    }
}

impl PluginsConfig {
    /// 该插件是否处于「激活意图」（平台总闸 ∧ 条目显式 enabled=true；缺省条目 = false）。
    pub fn allows(&self, id: &str) -> bool {
        self.enabled && self.entries.get(id).map(|e| e.enabled).unwrap_or(false)
    }

    /// 老部署迁移判定：缺省启用时代的条目（缺失 = 旧默认 true；显式 true 同）。
    /// boot 用它识别「产物存在、无注册标记、旧语义 enabled」→ 自动补注册+激活一次。
    pub fn legacy_enabled(&self, id: &str) -> bool {
        self.enabled && self.entries.get(id).map(|e| e.enabled).unwrap_or(true)
    }
}

/// 单插件条目。
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(default)]
pub struct PluginEntryConfig {
    /// 激活意图：true = 引导激活（缺省 false——上传/注册不自动运行）。
    pub enabled: bool,
    /// 可选的内容摘要钉版（SHA-256 小写十六进制）。缺省表示兼容旧配置，不校验摘要。
    pub artifact: Option<String>,
    /// 可选的 Ed25519 detached attestation（标准 base64，签名覆盖 artifact 摘要文本）。
    pub attestation: Option<String>,
}

impl Default for PluginEntryConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            artifact: None,
            attestation: None,
        }
    }
}

impl PluginEntryConfig {
    /// 仅接受标准 SHA-256 文本，避免拼写错误把配置变成永远无法装载的隐性禁用。
    pub fn valid_artifact_pin(pin: &str) -> bool {
        pin.len() == 64 && pin.bytes().all(|b| b.is_ascii_hexdigit())
    }

    /// `None` 保持 P3 之前的兼容语义：只按 enabled 判断，不增加摘要门。
    pub fn artifact_matches(&self, digest: &str) -> bool {
        self.artifact
            .as_deref()
            .map(|pin| pin.eq_ignore_ascii_case(digest))
            .unwrap_or(true)
    }
}
