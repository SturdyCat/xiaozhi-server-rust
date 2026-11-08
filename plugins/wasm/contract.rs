//! WIT 契约宿主绑定与投影 DTO。
//!
//! `bindgen!` 以 `server/wit/plugin.wit` 为唯一源生成宿主侧类型与链接器胶水；
//! 本文件把这些 WIT 类型转成 serde DTO（`/api/plugins` 投影与内部记录用）。
//! 装载期校验在 [`validate`] 子模块（manifest / registration / endpoint / subscription）。

use anyhow::anyhow;

wasmtime::component::bindgen!({
    // 直接指向契约文件（目录形式会被当作 deps/ 包布局解析而失败）
    path: "wit/plugin.wit",
    world: "plugin",
});

// bindgen 产物**直接展开在本模块**（无 bindings:: 包装）：
// - `contract::Plugin`：world 实例（lifecycle 导出的访问器）
// - `contract::xiaozhi::plugin::host_core::Host` 等：四个宿主服务 trait
// - `contract::xiaozhi::plugin::types::*`：契约记录
// 下面统一 re-export，模块树其余部分只经 contract 引用。
pub use self::xiaozhi::plugin::types as wit;

mod validate;
pub use validate::{validate_manifest, validate_registration};

// ============================================================
// 投影 DTO（serde；WIT 类型不过 serde，边界处转换）
// ============================================================

/// 事件订阅投影。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct SubscriptionDto {
    pub topic: String,
    #[serde(default)]
    pub filter_json: Option<String>,
}

/// 端点协议（与 WIT `protocol` 同义）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProtocolDto {
    Http,
    Tcp,
    Udp,
    Ws,
}

impl ProtocolDto {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Http => "http",
            Self::Tcp => "tcp",
            Self::Udp => "udp",
            Self::Ws => "ws",
        }
    }

    /// 该协议是否由宿主做 HTTP 报文分帧（其余给原始字节）。
    pub fn is_message_oriented(&self) -> bool {
        matches!(self, Self::Http | Self::Ws)
    }
}

/// 端点声明投影。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct EndpointSpecDto {
    pub name: String,
    pub protocol: ProtocolDto,
    pub bind: String,
    pub port: u16,
    #[serde(default)]
    pub path: Option<String>,
    pub required: bool,
}

/// 资源配额投影（0 表示用宿主默认值）。
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct ResourceLimitsDto {
    pub fuel_per_call: u64,
    pub epoch_ms_per_call: u64,
    pub memory_bytes: u64,
    pub max_connections: u16,
    pub max_queued_events: u16,
    pub max_open_files: u16,
}

/// 服务方法投影。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct ServiceMethodDto {
    pub name: String,
    #[serde(default)]
    pub request_schema_json: String,
    #[serde(default)]
    pub response_schema_json: String,
}

/// 服务声明投影。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct ServiceDeclDto {
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub methods: Vec<ServiceMethodDto>,
}

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
    /// 依赖的其他插件（激活前置；装载/注册不判定）。
    #[serde(default)]
    pub requires: Vec<RequirementDto>,
    /// 事件订阅；宿主按 topic 路由，不解释语义。
    #[serde(default)]
    pub subscriptions: Vec<SubscriptionDto>,
    /// 要宿主监听的端点。
    #[serde(default)]
    pub endpoints: Vec<EndpointSpecDto>,
    /// 资源配额。
    #[serde(default)]
    pub resources: ResourceLimitsDto,
    /// 配置命名空间（必须等于 id）。
    #[serde(default)]
    pub config_namespace: String,
    /// 提供的服务声明。
    #[serde(default)]
    pub provides: Vec<ServiceDeclDto>,
}

/// 依赖声明投影（`requires`）。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct RequirementDto {
    pub id: String,
    pub min_version: String,
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

/// 插件声明的 JSON UI 页面（A2UI 最小闭环）。宿主只透传：`schema_json` 校验为
/// 单个 JSON 对象后按 `schema` 键原样投影，不解释、不渲染。
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct RegistrationDto {
    pub menu: Vec<MenuEntryDto>,
    pub config_fields: Vec<FieldSchemaDto>,
    pub actions: Vec<ActionDescDto>,
    /// 通用 JSON UI 描述（A2UI 最小闭环）：`Some(文本)` 必须是单个 JSON 对象；
    /// 宿主只校验并透传投影（组件树 / actions / config bindings 由插件自组织）。
    pub ui_json: Option<String>,
    /// 运行时补充的服务声明（与 manifest.provides 合并）。
    #[serde(default)]
    pub services: Vec<ServiceDeclDto>,
}

/// UI 描述大小上限（与 guest SDK builder 同源；漂移双侧同步改）：
/// 防 manifest 膨胀拖累清单投影。
pub const MAX_UI_JSON_BYTES: usize = 256 * 1024;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct TestResultDto {
    pub name: String,
    pub ok: bool,
    pub detail: String,
}

// ============================================================
// WIT → DTO 转换
// ============================================================

fn protocol_from(p: wit::Protocol) -> ProtocolDto {
    match p {
        wit::Protocol::Http => ProtocolDto::Http,
        wit::Protocol::Tcp => ProtocolDto::Tcp,
        wit::Protocol::Udp => ProtocolDto::Udp,
        wit::Protocol::Ws => ProtocolDto::Ws,
    }
}

fn service_decl_from(s: wit::ServiceDecl) -> ServiceDeclDto {
    ServiceDeclDto {
        name: s.name,
        version: s.version,
        methods: s
            .methods
            .into_iter()
            .map(|m| ServiceMethodDto {
                name: m.name,
                request_schema_json: m.request_schema_json,
                response_schema_json: m.response_schema_json,
            })
            .collect(),
    }
}

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
            requires: m
                .requires
                .into_iter()
                .map(|r| RequirementDto {
                    id: r.id,
                    min_version: r.min_version,
                })
                .collect(),
            subscriptions: m
                .subscriptions
                .into_iter()
                .map(|s| SubscriptionDto {
                    topic: s.topic,
                    filter_json: s.filter_json,
                })
                .collect(),
            endpoints: m
                .endpoints
                .into_iter()
                .map(|e| EndpointSpecDto {
                    name: e.name,
                    protocol: protocol_from(e.protocol),
                    bind: e.bind,
                    port: e.port,
                    path: e.path,
                    required: e.required,
                })
                .collect(),
            resources: ResourceLimitsDto {
                fuel_per_call: m.resources.fuel_per_call,
                epoch_ms_per_call: m.resources.epoch_ms_per_call,
                memory_bytes: m.resources.memory_bytes,
                max_connections: m.resources.max_connections,
                max_queued_events: m.resources.max_queued_events,
                max_open_files: m.resources.max_open_files,
            },
            config_namespace: m.config_namespace,
            provides: m.provides.into_iter().map(service_decl_from).collect(),
        }
    }
}

/// 数字点分段版本比较（`1.2.3`；缺段补 0；含非数字段 → None）。
/// 供升级判定（新>旧）与依赖满足判定（运行 ≥ min）共用同一口径。
pub fn compare_versions(a: &str, b: &str) -> Option<std::cmp::Ordering> {
    fn segments(v: &str) -> Option<Vec<u64>> {
        if v.is_empty() {
            return None;
        }
        v.split('.')
            .map(|seg| seg.parse::<u64>().ok())
            .collect::<Option<Vec<_>>>()
    }
    let (a, b) = (segments(a)?, segments(b)?);
    let len = a.len().max(b.len());
    let mut order = std::cmp::Ordering::Equal;
    for i in 0..len {
        let (x, y) = (
            a.get(i).copied().unwrap_or(0),
            b.get(i).copied().unwrap_or(0),
        );
        order = order.then(x.cmp(&y));
        if order != std::cmp::Ordering::Equal {
            break;
        }
    }
    Some(order)
}

/// `version >= min`（数字点分段；任一侧不可比较 = false）。
pub fn version_at_least(version: &str, min: &str) -> bool {
    compare_versions(version, min) != Some(std::cmp::Ordering::Less)
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
            ui_json: r.ui_json,
            services: r.services.into_iter().map(service_decl_from).collect(),
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

/// UI 描述的只读投影（`/api/plugins` registration.ui 与
/// `GET /api/plugins/{id}/ui` 共用同一出口）：文本解析为 JSON 对象；
/// 未声明 → `null`（装载期已校验，此处只为投影兜底）。
pub fn ui_json_value(ui_json: Option<&str>) -> serde_json::Value {
    ui_json
        .and_then(|text| serde_json::from_str(text).ok())
        .unwrap_or(serde_json::Value::Null)
}

/// 构造契约错误（供 host 实现复用统一文案）。
pub fn contract_err(msg: impl Into<String>) -> anyhow::Error {
    anyhow!(msg.into())
}

/// JSON 值类型名（错误文案用）。
fn json_type_name(v: &serde_json::Value) -> &'static str {
    match v {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "bool",
        serde_json::Value::Number(_) => "number",
        serde_json::Value::String(_) => "string",
        serde_json::Value::Array(_) => "array",
        serde_json::Value::Object(_) => "object",
    }
}
