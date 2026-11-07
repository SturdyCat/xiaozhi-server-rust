//! 静态注册包构建器：manifest / registration 的类型化构建 + guest 侧前置校验。
//!
//! 校验规则与宿主 `plugins/wasm/contract.rs::validate_*` **同源**
//! （规则漂移 = 双侧都该改）：形状非法在构建期报 [`Error::Invalid`]，而不是装载期
//! 被宿主拒绝后只剩一行日志。静态数据用 expect 语义（见 crate 文档分界表）。
//!
//! ```ignore
//! let manifest = ManifestBuilder::new("weather.official", "天气")?
//!     .description("NMC 天气查询")
//!     .permission("http-out:www.nmc.cn")?
//!     .implements("skill.weather")?
//!     .official(true)
//!     .build()?;
//! ```

use crate::error::{Error, Result};
use crate::xiaozhi::plugin::types as wit;
use crate::API_VERSION;

// ============================================================
// manifest
// ============================================================

/// 已知权限名（与宿主 `effects::Permissions::parse` 同源；漂移双侧同步改）。
const KNOWN_BARE_PERMISSIONS: [&str; 5] = [
    "kv",
    "speak",
    "device-frames",
    "schedule",
    "lan-socket",
];
/// 已知菜单槽位（与宿主 `contract::validate_registration` 同源）。
pub const KNOWN_MENU_SLOTS: [&str; 3] = ["nav.main", "config.section", "workflow.node-menu"];
/// 已知配置字段 kind（同源）。
const KNOWN_FIELD_KINDS: [&str; 9] = [
    "str", "password", "int", "float", "bool", "enum", "path", "dir", "text",
];

fn valid_plugin_id(id: &str) -> bool {
    !id.is_empty()
        && id.split('.').all(|seg| {
            !seg.is_empty()
                && seg
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
        })
}

fn valid_permission(p: &str) -> bool {
    if KNOWN_BARE_PERMISSIONS.contains(&p) {
        return true;
    }
    match p.strip_prefix("http-out:") {
        Some(host) => !host.is_empty() && !host.contains(' '),
        None => false,
    }
}

/// manifest 构建器（链式；`new` 即校验 id/title，`build` 产出 WIT 类型）。
#[derive(Debug)]
pub struct ManifestBuilder {
    id: String,
    version: String,
    api_version: String,
    title: String,
    description: String,
    permissions: Vec<String>,
    implements: Vec<String>,
    official: bool,
}

impl ManifestBuilder {
    pub fn new(id: &str, title: &str) -> Result<Self> {
        if !valid_plugin_id(id) {
            return Err(Error::Invalid(format!(
                "插件 id {id:?}（应为 `<域>.<名>`，段内仅小写字母数字与 -/_）"
            )));
        }
        if title.is_empty() {
            return Err(Error::Invalid("manifest.title 不能为空".into()));
        }
        Ok(Self {
            id: id.to_string(),
            version: "0.1.0".to_string(),
            api_version: API_VERSION.to_string(),
            title: title.to_string(),
            description: String::new(),
            permissions: Vec::new(),
            implements: Vec::new(),
            official: false,
        })
    }

    pub fn version(mut self, v: &str) -> Self {
        self.version = v.to_string();
        self
    }

    /// 覆盖契约版本（默认 [`API_VERSION`]；主版本不匹配宿主会拒绝激活）。
    pub fn api_version(mut self, v: &str) -> Self {
        self.api_version = v.to_string();
        self
    }

    pub fn description(mut self, d: &str) -> Self {
        self.description = d.to_string();
        self
    }

    /// 声明权限（已知集合内、不重复；未知权限宿主装载期整体拒绝）。
    pub fn permission(mut self, p: &str) -> Result<Self> {
        if !valid_permission(p) {
            return Err(Error::Invalid(format!(
                "未知权限 {p:?}（合法：{KNOWN_BARE_PERMISSIONS:?} 与 http-out:<host>）"
            )));
        }
        if self.permissions.iter().any(|x| x == p) {
            return Err(Error::Duplicate(format!("权限 {p:?}")));
        }
        self.permissions.push(p.to_string());
        Ok(self)
    }

    /// 声明实现的扩展点（`asr.provider` 形；不重复）。
    pub fn implements(mut self, point: &str) -> Result<Self> {
        if point.is_empty() || !point.contains('.') {
            return Err(Error::Invalid(format!(
                "扩展点 {point:?} 形状非法（应为 `asr.provider` 形）"
            )));
        }
        if self.implements.iter().any(|x| x == point) {
            return Err(Error::Duplicate(format!("扩展点 {point:?}")));
        }
        self.implements.push(point.to_string());
        Ok(self)
    }

    /// 官方默认实现标记（随内核分发、不可卸载、被回退器兜底）。
    pub fn official(mut self, official: bool) -> Self {
        self.official = official;
        self
    }

    pub fn build(self) -> Result<wit::PluginManifest> {
        if self.api_version.is_empty() {
            return Err(Error::Invalid("api-version 为空".into()));
        }
        Ok(wit::PluginManifest {
            id: self.id,
            version: self.version,
            api_version: self.api_version,
            title: self.title,
            description: self.description,
            permissions: self.permissions,
            implements: self.implements,
            official: self.official,
        })
    }
}

// ============================================================
// registration
// ============================================================

/// registration 构建器（菜单/配置字段/动作；重复与形状错误在 push 时报）。
#[derive(Default)]
pub struct RegistrationBuilder {
    menu: Vec<wit::MenuEntry>,
    config_fields: Vec<wit::FieldSchema>,
    actions: Vec<wit::ActionDesc>,
}

impl RegistrationBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    /// 注册菜单槽位项（槽位已知、槽内 id 唯一、label 非空）。
    pub fn menu_entry(
        mut self,
        slot: &str,
        id: &str,
        label: &str,
        group: &str,
        order: u16,
        icon: &str,
    ) -> Result<Self> {
        if !KNOWN_MENU_SLOTS.contains(&slot) {
            return Err(Error::Invalid(format!(
                "菜单槽位 {slot:?} 未知（entry {id:?}；已知 {KNOWN_MENU_SLOTS:?}）"
            )));
        }
        if self.menu.iter().any(|m| m.slot == slot && m.id == id) {
            return Err(Error::Duplicate(format!(
                "菜单槽位 {slot:?} 内 entry {id:?}"
            )));
        }
        if label.is_empty() {
            return Err(Error::Invalid(format!("菜单 entry {id:?} label 为空")));
        }
        self.menu.push(wit::MenuEntry {
            slot: slot.to_string(),
            id: id.to_string(),
            label: label.to_string(),
            group: group.to_string(),
            order,
            icon: icon.to_string(),
        });
        Ok(self)
    }

    /// 注册配置字段（[`FieldSpec`] 构建后传入；key 唯一）。
    pub fn config_field(mut self, spec: wit::FieldSchema) -> Result<Self> {
        if spec.key.is_empty() || spec.key.contains('.') || spec.key.contains('/') {
            return Err(Error::Invalid(format!(
                "配置字段 key {:?} 非法（`[plugins.<id>.config]` 段内的裸键）",
                spec.key
            )));
        }
        if self.config_fields.iter().any(|f| f.key == spec.key) {
            return Err(Error::Duplicate(format!("配置字段 key {:?}", spec.key)));
        }
        if !KNOWN_FIELD_KINDS.contains(&spec.kind.as_str()) {
            return Err(Error::Invalid(format!(
                "配置字段 {} 的 kind {:?} 非法（合法 {KNOWN_FIELD_KINDS:?}）",
                spec.key, spec.kind
            )));
        }
        if spec.kind == "enum" && spec.enum_values.is_empty() {
            return Err(Error::Invalid(format!(
                "配置字段 {} 声明 enum 但 enum-values 为空",
                spec.key
            )));
        }
        self.config_fields.push(spec);
        Ok(self)
    }

    /// 注册管理动作（id/label 非空、id 唯一）。
    pub fn action(mut self, id: &str, label: &str, danger: bool, help: &str) -> Result<Self> {
        if id.is_empty() || label.is_empty() {
            return Err(Error::Invalid(format!(
                "动作声明非法：{id:?}（id 与 label 必填）"
            )));
        }
        if self.actions.iter().any(|a| a.id == id) {
            return Err(Error::Duplicate(format!("动作 id {id:?}")));
        }
        self.actions.push(wit::ActionDesc {
            id: id.to_string(),
            label: label.to_string(),
            danger,
            help: help.to_string(),
        });
        Ok(self)
    }

    pub fn build(self) -> wit::Registration {
        wit::Registration {
            menu: self.menu,
            config_fields: self.config_fields,
            actions: self.actions,
        }
    }
}

// ============================================================
// 配置字段
// ============================================================

/// 配置字段构建器（kind 由构造器定死，杜绝拼写错误；默认 hot=live、非 secret、非必填）。
pub struct FieldSpec {
    key: String,
    label: String,
    kind: &'static str,
    default_hint: String,
    help: String,
    hot: wit::Hot,
    secret: bool,
    required: bool,
    enum_values: Vec<String>,
}

impl FieldSpec {
    fn new(key: &str, label: &str, kind: &'static str) -> Self {
        Self {
            key: key.to_string(),
            label: label.to_string(),
            kind,
            default_hint: String::new(),
            help: String::new(),
            hot: wit::Hot::Live,
            secret: false,
            required: false,
            enum_values: Vec::new(),
        }
    }

    pub fn str(key: &str, label: &str) -> Self {
        Self::new(key, label, "str")
    }
    pub fn password(key: &str, label: &str) -> Self {
        Self::new(key, label, "password")
    }
    pub fn int(key: &str, label: &str) -> Self {
        Self::new(key, label, "int")
    }
    pub fn float(key: &str, label: &str) -> Self {
        Self::new(key, label, "float")
    }
    pub fn bool(key: &str, label: &str) -> Self {
        Self::new(key, label, "bool")
    }
    pub fn path(key: &str, label: &str) -> Self {
        Self::new(key, label, "path")
    }
    pub fn dir(key: &str, label: &str) -> Self {
        Self::new(key, label, "dir")
    }
    pub fn text(key: &str, label: &str) -> Self {
        Self::new(key, label, "text")
    }
    /// 枚举字段（values 非空由 [`RegistrationBuilder::config_field`] 校验）。
    pub fn enum_(key: &str, label: &str, values: &[&str]) -> Self {
        let mut s = Self::new(key, label, "enum");
        s.enum_values = values.iter().map(|v| v.to_string()).collect();
        s
    }

    /// 管理台展示的默认值提示（展示用途，不是运行默认值——同静态注册表口径）。
    pub fn hint(mut self, hint: &str) -> Self {
        self.default_hint = hint.to_string();
        self
    }
    pub fn help(mut self, help: &str) -> Self {
        self.help = help.to_string();
        self
    }
    pub fn hot(mut self, hot: wit::Hot) -> Self {
        self.hot = hot;
        self
    }
    pub fn secret(mut self) -> Self {
        self.secret = true;
        self
    }
    pub fn required(mut self) -> Self {
        self.required = true;
        self
    }

    pub fn build(self) -> wit::FieldSchema {
        wit::FieldSchema {
            key: self.key,
            label: self.label,
            kind: self.kind.to_string(),
            default_hint: self.default_hint,
            help: self.help,
            hot: self.hot,
            secret: self.secret,
            required: self.required,
            enum_values: self.enum_values,
        }
    }
}
