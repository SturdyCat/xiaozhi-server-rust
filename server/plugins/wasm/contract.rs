//! WIT 契约宿主绑定与投影 DTO。
//!
//! `bindgen!` 以 `server/wit/plugin.wit` 为唯一源生成宿主侧类型与链接器胶水；
//! 本文件把这些 WIT 类型转成 serde DTO（`/api/plugins` 投影与内部记录用），
//! 并提供 manifest 的装载期校验（契约主版本、id 形状、扩展点声明形状）。

use anyhow::{anyhow, bail, Result};

wasmtime::component::bindgen!({
    // 直接指向契约文件（目录形式会被当作 deps/ 包布局解析而失败）
    path: "wit/plugin.wit",
    world: "plugin",
});

// bindgen 产物**直接展开在本模块**（无 bindings:: 包装）：
// - `contract::Plugin`：world 实例（lifecycle 导出的访问器）
// - `contract::xiaozhi::plugin::host::Host`：宿主服务 trait（log / now-ms）
// - `contract::xiaozhi::plugin::types::*`：契约记录
// 下面统一 re-export，模块树其余部分只经 contract 引用。

use self::xiaozhi::plugin::types as wit;

// ============================================================
// 投影 DTO（serde；WIT 类型不过 serde，边界处转换）
// ============================================================

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct ManifestDto {
    pub id: String,
    pub version: String,
    pub api_version: String,
    pub title: String,
    pub description: String,
    pub permissions: Vec<String>,
    pub implements: Vec<String>,
    pub official: bool,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct MenuEntryDto {
    pub slot: String,
    pub id: String,
    pub label: String,
    pub group: String,
    pub order: u16,
    pub icon: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct FieldSchemaDto {
    pub key: String,
    pub label: String,
    pub kind: String,
    pub default_hint: String,
    pub help: String,
    pub hot: String,
    pub secret: bool,
    pub required: bool,
    pub enum_values: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct ActionDescDto {
    pub id: String,
    pub label: String,
    pub danger: bool,
    pub help: String,
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct RegistrationDto {
    pub menu: Vec<MenuEntryDto>,
    pub config_fields: Vec<FieldSchemaDto>,
    pub actions: Vec<ActionDescDto>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct TestResultDto {
    pub name: String,
    pub ok: bool,
    pub detail: String,
}

// ============================================================
// WIT → DTO 转换
// ============================================================

impl From<wit::PluginManifest> for ManifestDto {
    fn from(m: wit::PluginManifest) -> Self {
        Self {
            id: m.id,
            version: m.version,
            api_version: m.api_version,
            title: m.title,
            description: m.description,
            permissions: m.permissions,
            implements: m.implements,
            official: m.official,
        }
    }
}

fn hot_str(h: wit::Hot) -> &'static str {
    match h {
        wit::Hot::Live => "live",
        wit::Hot::NextSession => "next_session",
        wit::Hot::RestartOnly => "restart_only",
    }
}

impl From<wit::Registration> for RegistrationDto {
    fn from(r: wit::Registration) -> Self {
        Self {
            menu: r
                .menu
                .into_iter()
                .map(|m| MenuEntryDto {
                    slot: m.slot,
                    id: m.id,
                    label: m.label,
                    group: m.group,
                    order: m.order,
                    icon: m.icon,
                })
                .collect(),
            config_fields: r
                .config_fields
                .into_iter()
                .map(|f| FieldSchemaDto {
                    key: f.key,
                    label: f.label,
                    kind: f.kind,
                    default_hint: f.default_hint,
                    help: f.help,
                    hot: hot_str(f.hot).to_string(),
                    secret: f.secret,
                    required: f.required,
                    enum_values: f.enum_values,
                })
                .collect(),
            actions: r
                .actions
                .into_iter()
                .map(|a| ActionDescDto {
                    id: a.id,
                    label: a.label,
                    danger: a.danger,
                    help: a.help,
                })
                .collect(),
        }
    }
}

impl From<wit::TestResult> for TestResultDto {
    fn from(t: wit::TestResult) -> Self {
        Self {
            name: t.name,
            ok: t.ok,
            detail: t.detail,
        }
    }
}

// ============================================================
// 装载期校验
// ============================================================

/// manifest 校验：拒绝而非静默容忍（对齐静态注册表「未知 engine 不静默回退」的口径）。
pub fn validate_manifest(m: &ManifestDto) -> Result<()> {
    if m.id.is_empty() {
        bail!("manifest.id 不能为空");
    }
    // id 形状：`<域>.<名>`，段内仅小写字母数字与 `-`/`_`，防槽位键与配置段注入
    for seg in m.id.split('.') {
        if seg.is_empty()
            || !seg
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
        {
            bail!(
                "manifest.id 非法：{:?}（应为 `<域>.<名>`，段内仅小写字母数字与 -/_）",
                m.id
            );
        }
    }
    // 契约主版本：`api_version` 形如 "0" 或 "0.1"，取首段比对
    let major = m
        .api_version
        .split('.')
        .next()
        .unwrap_or("")
        .parse::<u32>()
        .map_err(|_| anyhow!("manifest.api-version 非法：{:?}", m.api_version))?;
    if major != crate::plugins::wasm::CONTRACT_MAJOR_VERSION {
        bail!(
            "契约主版本不匹配：插件 {} 声明 {}，宿主 {}（拒绝激活；请改用匹配的 SDK 构建）",
            m.id,
            m.api_version,
            crate::plugins::wasm::CONTRACT_MAJOR_VERSION
        );
    }
    if m.title.is_empty() {
        bail!("manifest.title 不能为空（管理台清单展示需要）");
    }
    for p in &m.implements {
        if p.is_empty() || !p.contains('.') {
            bail!(
                "manifest.implements 扩展点形状非法：{:?}（应为 `asr.provider` 形）",
                p
            );
        }
    }
    Ok(())
}

/// 注册包校验：菜单槽位 id 唯一、槽位名在已知集合内；扩展点与 manifest 声明一致由 resolver 保证。
pub fn validate_registration(reg: &RegistrationDto) -> Result<()> {
    let known_slots = ["nav.main", "config.section", "workflow.node-menu"];
    let mut seen = std::collections::HashSet::new();
    for m in &reg.menu {
        if !known_slots.contains(&m.slot.as_str()) {
            bail!(
                "菜单槽位 {:?} 未知（entry id={}）：已知 {:?}",
                m.slot,
                m.id,
                known_slots
            );
        }
        if !seen.insert((m.slot.clone(), m.id.clone())) {
            bail!("菜单槽位 {:?} 内 entry id {:?} 重复", m.slot, m.id);
        }
        if m.label.is_empty() {
            bail!("菜单 entry {} label 为空", m.id);
        }
    }
    let mut keys = std::collections::HashSet::new();
    let kinds = [
        "str", "password", "int", "float", "bool", "enum", "path", "dir", "text",
    ];
    for f in &reg.config_fields {
        if f.key.is_empty() || f.key.contains('.') || f.key.contains('/') {
            bail!(
                "配置字段 key 非法：{:?}（`[plugins.<id>.config]` 段内的裸键）",
                f.key
            );
        }
        if !keys.insert(f.key.clone()) {
            bail!("配置字段 key 重复：{:?}", f.key);
        }
        if !kinds.contains(&f.kind.as_str()) {
            bail!(
                "配置字段 {} 的 kind 非法：{:?}（合法 {:?}）",
                f.key,
                f.kind,
                kinds
            );
        }
        if f.kind == "enum" && f.enum_values.is_empty() {
            bail!("配置字段 {} 声明 enum 但 enum-values 为空", f.key);
        }
    }
    let mut actions = std::collections::HashSet::new();
    for a in &reg.actions {
        if a.id.is_empty() || a.label.is_empty() {
            bail!("动作声明非法：{:?}（id 与 label 必填）", a.id);
        }
        if !actions.insert(a.id.clone()) {
            bail!("动作 id 重复：{:?}", a.id);
        }
    }
    Ok(())
}
