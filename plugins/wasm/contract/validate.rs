//! 装载期校验：manifest / registration / endpoint / subscription / service。
//!
//! 口径：**拒绝而非静默容忍**。宿主不认识业务语义，但必须挡住形状非法、
//! 越权配额、可被注入路由键的声明。

use anyhow::{anyhow, bail, Result};
use std::collections::HashSet;

use super::{
    compare_versions, json_type_name, ManifestDto, RegistrationDto, MAX_UI_JSON_BYTES,
};

/// 资源配额上限（防插件声明夸张值把宿主拖垮）。
const MAX_MEMORY_BYTES: u64 = 1024 * 1024 * 1024;
const MAX_FUEL_PER_CALL: u64 = 100_000_000_000;
const MAX_EPOCH_MS_PER_CALL: u64 = 60_000;
const MAX_CONNECTIONS: u16 = 4096;
const MAX_QUEUED_EVENTS: u16 = 65_535;
const MAX_OPEN_FILES: u16 = 1024;

/// 允许绑定的地址（其余一律拒绝，避免插件把宿主暴露到可路由网段之外）。
const ALLOWED_BINDS: [&str; 4] = ["0.0.0.0", "127.0.0.1", "::", "::1"];

/// 配置字段允许的控件类型（与前端渲染器同源，漂移双侧同步改）。
pub const CONFIG_FIELD_KINDS: [&str; 9] = [
    "str", "password", "int", "float", "bool", "enum", "path", "dir", "text",
];

fn valid_version(v: &str) -> bool {
    compare_versions(v, "0").is_some()
}

fn valid_id_shape(id: &str) -> bool {
    !id.is_empty()
        && id.split('.').all(|seg| {
            !seg.is_empty()
                && seg
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
        })
}

/// topic 形状：非空、≤128、只允许 ASCII 可见字符、通配只能在尾部且形如 `.*`。
fn valid_topic(topic: &str) -> Result<()> {
    if topic.is_empty() || topic.len() > 128 {
        bail!("订阅 topic 长度非法：{:?}（1..=128 字节）", topic);
    }
    // 通配形状先判（S3m 勘误：此前字符白名单先行，尾部 `.*` 永远无法通过声明
    // ——文档承诺的通配订阅是死路）。只允许尾部通配且形如 `.*`。
    let wildcard = topic.ends_with(".*");
    if topic.contains('*') && !wildcard {
        bail!(
            "订阅 topic 通配只允许尾部 `.*`：{:?}（禁止中间通配）",
            topic
        );
    }
    // 字符白名单对去除尾部 `.*` 后的前缀检查（`*` 只在尾部通配语境合法）。
    let prefix = topic.trim_end_matches(".*");
    if prefix.is_empty() {
        bail!("订阅 topic 通配前缀不能为空：{:?}", topic);
    }
    if !prefix
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-')
    {
        bail!(
            "订阅 topic 含非法字符：{:?}（仅允许字母数字与 . _ -）",
            topic
        );
    }
    if prefix.ends_with('.') || prefix.contains("..") {
        bail!("订阅 topic 段非法（空段）：{:?}", topic);
    }
    Ok(())
}

/// manifest 校验：拒绝而非静默容忍（对齐「未知 engine 不静默回退」的口径）。
pub fn validate_manifest(m: &ManifestDto) -> Result<()> {
    if !valid_id_shape(&m.id) {
        bail!(
            "manifest.id 非法：{:?}（应为 `<域>.<名>`，段内仅小写字母数字与 -/_）",
            m.id
        );
    }

    // 契约版本：主版本必须相等，次版本必须等于宿主当前次版本（0.2）。
    let mut segs = m.api_version.split('.');
    let major = segs
        .next()
        .unwrap_or("")
        .parse::<u32>()
        .map_err(|_| anyhow!("manifest.api-version 非法：{:?}", m.api_version))?;
    let minor = segs.next().unwrap_or("0").parse::<u32>().unwrap_or(0);
    let host_major = crate::plugins::wasm::CONTRACT_MAJOR_VERSION;
    let host_minor = crate::plugins::wasm::CONTRACT_MINOR_VERSION;
    if major != host_major || minor != host_minor {
        bail!(
            "契约版本不匹配：插件 {} 声明 {}，宿主 {}.{}（拒绝激活；请用 xiaozhi-plugin-sdk {}.{} 重新构建后上传）",
            m.id,
            m.api_version,
            host_major,
            host_minor,
            host_major,
            host_minor
        );
    }

    if m.title.is_empty() {
        bail!("manifest.title 不能为空（管理台清单展示需要）");
    }
    if !valid_version(&m.version) {
        bail!(
            "manifest.version 非法：{:?}（应为数字点分段，如 1.2.3）",
            m.version
        );
    }
    if m.config_namespace != m.id {
        bail!(
            "manifest.config-namespace 必须等于插件 id：声明 {:?}，id {}",
            m.config_namespace,
            m.id
        );
    }

    let mut seen_requires = HashSet::new();
    for r in &m.requires {
        if !valid_id_shape(&r.id) {
            bail!("manifest.requires[].id 非法：{:?}", r.id);
        }
        if r.id == m.id {
            bail!("manifest.requires 自引用：{} 不能依赖自身", r.id);
        }
        if !seen_requires.insert(r.id.clone()) {
            bail!("manifest.requires 重复依赖：{}", r.id);
        }
        if !valid_version(&r.min_version) {
            bail!(
                "manifest.requires[{}].min-version 非法：{:?}（数字点分段）",
                r.id,
                r.min_version
            );
        }
    }

    for p in &m.implements {
        if p.is_empty() || !p.contains('.') {
            bail!(
                "manifest.implements 扩展点形状非法：{:?}（应为 `asr.provider` 形）",
                p
            );
        }
    }

    for s in &m.subscriptions {
        valid_topic(&s.topic)?;
        if let Some(filter) = &s.filter_json {
            let parsed: serde_json::Value = serde_json::from_str(filter)
                .map_err(|e| anyhow!("订阅 {} 的 filter-json 非法：{e}", s.topic))?;
            if !parsed.is_object() {
                bail!(
                    "订阅 {} 的 filter-json 顶层必须是 JSON 对象（得到 {}）",
                    s.topic,
                    json_type_name(&parsed)
                );
            }
        }
    }

    let mut endpoint_names = HashSet::new();
    for e in &m.endpoints {
        if e.name.is_empty() || e.name.len() > 64 {
            bail!("endpoint.name 非法：{:?}（1..=64 字节）", e.name);
        }
        if !e
            .name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            bail!("endpoint.name 含非法字符：{:?}", e.name);
        }
        if !endpoint_names.insert(e.name.clone()) {
            bail!("endpoint.name 重复：{:?}", e.name);
        }
        if !ALLOWED_BINDS.contains(&e.bind.as_str()) {
            bail!(
                "endpoint {} 的 bind 非法：{:?}（允许 {:?}）",
                e.name,
                e.bind,
                ALLOWED_BINDS
            );
        }
        if let Some(path) = &e.path {
            if !path.starts_with('/') {
                bail!("endpoint {} 的 path 必须以 / 开头：{:?}", e.name, path);
            }
        }
        // http / ws 必须有 path（默认 "/" 由插件显式声明，宿主不猜）
        if e.protocol.is_message_oriented() && e.path.is_none() {
            bail!(
                "endpoint {} 为 {} 协议，必须声明 path（可用 \"/\" 表示全部）",
                e.name,
                e.protocol.as_str()
            );
        }
    }

    let r = &m.resources;
    if r.memory_bytes > MAX_MEMORY_BYTES {
        bail!(
            "manifest.resources.memory-bytes 超上限：{} > {MAX_MEMORY_BYTES}",
            r.memory_bytes
        );
    }
    if r.fuel_per_call > MAX_FUEL_PER_CALL {
        bail!(
            "manifest.resources.fuel-per-call 超上限：{} > {MAX_FUEL_PER_CALL}",
            r.fuel_per_call
        );
    }
    if r.epoch_ms_per_call > MAX_EPOCH_MS_PER_CALL {
        bail!(
            "manifest.resources.epoch-ms-per-call 超上限：{} > {MAX_EPOCH_MS_PER_CALL}",
            r.epoch_ms_per_call
        );
    }
    if r.max_connections > MAX_CONNECTIONS {
        bail!(
            "manifest.resources.max-connections 超上限：{} > {MAX_CONNECTIONS}",
            r.max_connections
        );
    }
    if r.max_queued_events > MAX_QUEUED_EVENTS {
        bail!(
            "manifest.resources.max-queued-events 超上限：{} > {MAX_QUEUED_EVENTS}",
            r.max_queued_events
        );
    }
    if r.max_open_files > MAX_OPEN_FILES {
        bail!(
            "manifest.resources.max-open-files 超上限：{} > {MAX_OPEN_FILES}",
            r.max_open_files
        );
    }

    let mut services = HashSet::new();
    for s in &m.provides {
        validate_service(s, &mut services)?;
    }
    Ok(())
}

fn validate_service(
    s: &super::ServiceDeclDto,
    seen: &mut HashSet<String>,
) -> Result<()> {
    if !valid_id_shape(&s.name) {
        bail!("服务名非法：{:?}（应为 `<域>.<名>`）", s.name);
    }
    if !seen.insert(s.name.clone()) {
        bail!("服务名重复：{:?}", s.name);
    }
    if !valid_version(&s.version) {
        bail!("服务 {} 的 version 非法：{:?}", s.name, s.version);
    }
    let mut methods = HashSet::new();
    for m in &s.methods {
        if m.name.is_empty()
            || !m
                .name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            bail!("服务 {} 的方法名非法：{:?}", s.name, m.name);
        }
        if !methods.insert(m.name.clone()) {
            bail!("服务 {} 的方法名重复：{:?}", s.name, m.name);
        }
    }
    Ok(())
}

/// 注册包校验：槽位形状（插件可自定义槽位，宿主只按约定渲染已知槽位）、
/// 配置字段、动作、服务、A2UI 透传守门。
pub fn validate_registration(reg: &RegistrationDto) -> Result<()> {
    let mut seen = HashSet::new();
    for m in &reg.menu {
        if m.slot.is_empty() || !m.slot.contains('.') || m.slot.len() > 64 {
            bail!(
                "菜单槽位形状非法：{:?}（应为 `nav.main` 形，1..=64 字节）",
                m.slot
            );
        }
        if m.id.is_empty() || m.id.len() > 64 {
            bail!("菜单 entry id 非法：{:?}（1..=64 字节）", m.id);
        }
        if !seen.insert((m.slot.clone(), m.id.clone())) {
            bail!("菜单槽位 {:?} 内 entry id {:?} 重复", m.slot, m.id);
        }
        if m.label.is_empty() {
            bail!("菜单 entry {} label 为空", m.id);
        }
    }

    let mut keys = HashSet::new();
    for f in &reg.config_fields {
        if f.key.is_empty() || f.key.contains('.') || f.key.contains('/') || f.key.len() > 64 {
            bail!(
                "配置字段 key 非法：{:?}（插件命名空间内的裸键，1..=64 字节）",
                f.key
            );
        }
        if !keys.insert(f.key.clone()) {
            bail!("配置字段 key 重复：{:?}", f.key);
        }
        if !CONFIG_FIELD_KINDS.contains(&f.kind.as_str()) {
            bail!(
                "配置字段 {} 的 kind 非法：{:?}（合法 {:?}）",
                f.key,
                f.kind,
                CONFIG_FIELD_KINDS
            );
        }
        if f.kind == "enum" && f.enum_values.is_empty() {
            bail!("配置字段 {} 声明 enum 但 enum-values 为空", f.key);
        }
    }

    let mut actions = HashSet::new();
    for a in &reg.actions {
        if a.id.is_empty() || a.label.is_empty() {
            bail!("动作声明非法：{:?}（id 与 label 必填）", a.id);
        }
        if !actions.insert(a.id.clone()) {
            bail!("动作 id 重复：{:?}", a.id);
        }
    }

    let mut services = HashSet::new();
    for s in &reg.services {
        validate_service(s, &mut services)?;
    }

    validate_ui_json(reg.ui_json.as_deref())
}

/// UI 描述校验（A2UI 透传管道的守门）：可选字段；`Some` 时必须是
/// 「单个 JSON 对象」文本且不超大小上限。规则与 guest SDK
/// `RegistrationBuilder::ui_json` 同源（漂移双侧同步改）。
fn validate_ui_json(ui_json: Option<&str>) -> Result<()> {
    let Some(text) = ui_json else {
        return Ok(());
    };
    if text.len() > MAX_UI_JSON_BYTES {
        bail!("UI 描述超上限：{} > {MAX_UI_JSON_BYTES} 字节", text.len());
    }
    let parsed: serde_json::Value =
        serde_json::from_str(text).map_err(|e| anyhow!("UI 描述不是合法 JSON：{e}"))?;
    if !parsed.is_object() {
        bail!(
            "UI 描述顶层必须是 JSON 对象（得到 {}）",
            json_type_name(&parsed)
        );
    }
    Ok(())
}
