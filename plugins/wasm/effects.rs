//! 效果模型 + 权限门 + 执行器。
//!
//! 插件不直接做 I/O：一切副作用都是声明式 [`Effect`]，宿主逐效果过权限门
//! （manifest.permissions 声明 ⊆ 采用时批准）后执行。http-get 是**两段式**：
//! 执行器发起请求（[`HttpFetcher`]，测试注入假实现），结果由 registry 以
//! `http.response` 事件回灌插件（深度上限见 registry）。speak / device-frames /
//! schedule 在对应阶段落地，执行器对其返回 [`EffectError::Unsupported`]。

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use anyhow::Result;

use super::kv::KvStore;

/// 领域效果（WIT `types.effect` 的宿主镜像；variant 变体随宿主服务落地扩员）
#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    Log {
        level: String,
        message: String,
    },
    KvPut {
        key: String,
        value: Vec<u8>,
    },
    KvDelete {
        key: String,
    },
    HttpRequest {
        request_id: String,
        method: String,
        url: String,
        headers: Vec<(String, String)>,
        body: Vec<u8>,
        timeout_ms: u32,
    },
    LanStatus {
        request_id: String,
        device_handle: String,
    },
    LanSetDp {
        request_id: String,
        device_handle: String,
        dp: String,
        value_json: String,
    },
}

impl Effect {
    pub fn kind(&self) -> &'static str {
        match self {
            Effect::Log { .. } => "log",
            Effect::KvPut { .. } => "kv-put",
            Effect::KvDelete { .. } => "kv-delete",
            Effect::HttpRequest { .. } => "http-request",
            Effect::LanStatus { .. } => "lan-status",
            Effect::LanSetDp { .. } => "lan-set-dp",
        }
    }
}

/// 权限集合：从 manifest.permissions 解析。
///
/// 0.2 形状：`kv` / `config` / `pub` / `services` / `net` / `timer` / `fs` / `mdns`
/// （`log` 免声明）；`http-out:<host>` / `speak` / `device-frames` / `schedule` /
/// `lan-socket` 为旧效果通道，保留解析以兼容已装插件。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Permissions {
    pub kv: bool,
    /// 插件配置命名空间读写（`config-get` / `config-secret` / `config-set`）。
    pub config: bool,
    /// 发布自定义 topic（`publish`，仍限 `<plugin-id>.*`）。
    pub publish: bool,
    /// 调用其他插件的服务（`call-service`）。
    pub services: bool,
    /// socket 监听与出站（`listen` / `net-send` / `udp-send` 等）。
    pub net: bool,
    /// 定时器（`set-timeout` / `set-interval`）。
    pub timer: bool,
    /// 插件私有文件沙箱。
    pub fs: bool,
    /// mDNS 广播。
    pub mdns: bool,
    pub http_out_hosts: Vec<String>,
    pub speak: bool,
    pub device_frames: bool,
    pub schedule: bool,
    /// 高层 LAN connector 效果；不等于 raw socket 权限。
    pub lan_socket: bool,
}

impl Permissions {
    pub fn parse(declared: &[String]) -> Result<Self> {
        let mut p = Self::default();
        for d in declared {
            match d.as_str() {
                "kv" => p.kv = true,
                "config" => p.config = true,
                "pub" => p.publish = true,
                "services" => p.services = true,
                "net" => p.net = true,
                "timer" => p.timer = true,
                "fs" => p.fs = true,
                "mdns" => p.mdns = true,
                "speak" => p.speak = true,
                "device-frames" => p.device_frames = true,
                "schedule" => p.schedule = true,
                "lan-socket" => p.lan_socket = true,
                other => {
                    if let Some(host) = other.strip_prefix("http-out:") {
                        if host.is_empty() || host.contains(' ') {
                            anyhow::bail!("http-out 权限的 host 非法：{other:?}");
                        }
                        p.http_out_hosts.push(host.to_string());
                    } else {
                        anyhow::bail!(
                            "未知权限：{other:?}（合法：kv/config/pub/services/net/timer/fs/mdns/\
                             speak/device-frames/schedule/lan-socket/http-out:<host>）"
                        );
                    }
                }
            }
        }
        Ok(p)
    }
}

/// LAN connector 的高层命令；句柄是不透明标识，命令不携带密钥、IP、报文或 socket。
#[derive(Debug, Clone, PartialEq)]
pub enum LanSocketCommand {
    Status {
        /// Internal routing metadata; never crosses the WIT boundary.
        plugin_id: String,
        request_id: String,
        device_handle: String,
    },
    SetDp {
        /// Internal routing metadata; never crosses the WIT boundary.
        plugin_id: String,
        request_id: String,
        device_handle: String,
        dp: String,
        value_json: String,
    },
}

/// 有界 LAN 命令队列：宿主可在 native Tuya runtime 中异步 drain，队列满则显式失败。
pub struct LanSocketQueue {
    inner: Mutex<VecDeque<LanSocketCommand>>,
    capacity: usize,
}

impl LanSocketQueue {
    pub const DEFAULT_CAPACITY: usize = 64;
    pub const MAX_REQUEST_ID_BYTES: usize = 128;
    pub const MAX_HANDLE_BYTES: usize = 128;
    pub const MAX_DP_BYTES: usize = 32;
    pub const MAX_VALUE_JSON_BYTES: usize = 4096;

    pub fn new(capacity: usize) -> Self {
        Self {
            inner: Mutex::new(VecDeque::new()),
            capacity: capacity.clamp(1, Self::DEFAULT_CAPACITY),
        }
    }

    pub fn len(&self) -> usize {
        self.inner.lock().unwrap_or_else(|e| e.into_inner()).len()
    }

    pub fn drain(&self, max: usize) -> Vec<LanSocketCommand> {
        let mut queue = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        (0..max.min(queue.len()))
            .filter_map(|_| queue.pop_front())
            .collect()
    }

    pub fn push(&self, command: LanSocketCommand) -> Result<(), String> {
        validate_lan_command(&command)?;
        let mut queue = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if queue.len() >= self.capacity {
            return Err(format!("lan-socket 命令队列已满（上限 {}）", self.capacity));
        }
        queue.push_back(command);
        Ok(())
    }
}

impl Default for LanSocketQueue {
    fn default() -> Self {
        Self::new(Self::DEFAULT_CAPACITY)
    }
}

fn validate_lan_command(command: &LanSocketCommand) -> Result<(), String> {
    let (plugin_id, request_id, handle, dp, value_len) = match command {
        LanSocketCommand::Status {
            plugin_id,
            request_id,
            device_handle,
        } => (plugin_id, request_id, device_handle, None, None),
        LanSocketCommand::SetDp {
            plugin_id,
            request_id,
            device_handle,
            dp,
            value_json,
        } => (
            plugin_id,
            request_id,
            device_handle,
            Some(dp),
            Some(value_json.len()),
        ),
    };
    if plugin_id.is_empty()
        || plugin_id.len() > LanSocketQueue::MAX_HANDLE_BYTES
        || plugin_id.chars().any(char::is_control)
    {
        return Err("lan-socket plugin_id 为空、超长或含控制字符".to_string());
    }
    if request_id.is_empty() || request_id.len() > LanSocketQueue::MAX_REQUEST_ID_BYTES {
        return Err("lan-socket request_id 为空或超长".to_string());
    }
    if handle.is_empty()
        || handle.len() > LanSocketQueue::MAX_HANDLE_BYTES
        || handle.chars().any(char::is_control)
    {
        return Err("lan-socket device_handle 为空、超长或含控制字符".to_string());
    }
    if let Some(dp) = dp {
        if dp.is_empty()
            || dp.len() > LanSocketQueue::MAX_DP_BYTES
            || !dp.bytes().all(|b| b.is_ascii_digit())
        {
            return Err("lan-socket dp 必须是有界数字标识".to_string());
        }
    }
    if value_len.is_some_and(|len| len > LanSocketQueue::MAX_VALUE_JSON_BYTES) {
        return Err("lan-socket value-json 超长".to_string());
    }
    if let LanSocketCommand::SetDp { value_json, .. } = command {
        serde_json::from_str::<serde_json::Value>(value_json)
            .map_err(|e| format!("lan-socket value-json 非法：{e}"))?;
    }
    Ok(())
}

/// 权限拒绝（审计事件，不是 bug）
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum PermissionError {
    #[error("效果 {kind} 需要权限 {perm}（manifest 未声明）")]
    Denied {
        kind: &'static str,
        perm: &'static str,
    },
}

/// 效果执行错误
#[derive(Debug, thiserror::Error)]
pub enum EffectError {
    #[error(transparent)]
    Permission(#[from] PermissionError),
    #[error("效果 {kind} 的宿主服务尚未实现（按阶段落地，当前拒绝而非静默丢弃）")]
    Unsupported { kind: String },
    #[error("效果执行失败：{0}")]
    Failed(#[from] anyhow::Error),
}

/// 权限门：逐效果校验。返回被放行的效果（顺序保持），首个越权即报错。
pub fn enforce_permissions(
    perms: &Permissions,
    effects: Vec<Effect>,
) -> Result<Vec<Effect>, PermissionError> {
    for e in &effects {
        match e {
            Effect::Log { .. } => {} // 结构化日志免权限
            Effect::KvPut { .. } | Effect::KvDelete { .. } => {
                if !perms.kv {
                    return Err(PermissionError::Denied {
                        kind: e.kind(),
                        perm: "kv",
                    });
                }
            }
            Effect::HttpRequest { url, .. } => match url_host(url) {
                Some(host) if perms.http_out_hosts.iter().any(|h| h == &host) => {}
                _ => {
                    return Err(PermissionError::Denied {
                        kind: e.kind(),
                        perm: "http-out（URL host 不在白名单或 URL 非法）",
                    })
                }
            },
            Effect::LanStatus { .. } | Effect::LanSetDp { .. } => {
                if !perms.lan_socket {
                    return Err(PermissionError::Denied {
                        kind: e.kind(),
                        perm: "lan-socket",
                    });
                }
            }
        }
    }
    Ok(effects)
}

/// 从绝对 URL 提取 host（`scheme://host[:port]/...`）。手写最小解析：效果面只接受
/// http(s) 绝对地址，不值得为此引入 url 依赖；非法形状返回 None（权限门拒绝）。
fn url_host(url: &str) -> Option<String> {
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))?;
    let host_port = rest.split(['/', '?', '#']).next()?;
    let host = host_port.rsplit_once(':').map_or(host_port, |(h, p)| {
        // 仅剥端口（数字段）；IPv6 字面量含多个 ':' 时保守不剥
        if p.chars().all(|c| c.is_ascii_digit()) && !p.is_empty() && !h.is_empty() {
            h
        } else {
            host_port
        }
    });
    if host.is_empty() || host.contains(' ') {
        None
    } else {
        Some(host.to_string())
    }
}

// ============================================================
// HTTP 执行抽象（生产 reqwest；测试脚本化假实现，零真实网络）
// ============================================================

/// 效果执行器发起 HTTP 的抽象（对齐 tuya/cloud 的 HttpSender 模式）。
pub trait HttpFetcher: Send + Sync {
    /// GET 一个 URL。成功 = (状态码, 响应体)；失败 = 错误消息（回灌给插件自行降级）。
    fn fetch(&self, url: &str) -> std::result::Result<(u16, String), String>;

    /// 通用请求（`effect-http-request`）。默认实现只接受「无 body 的 GET」，
    /// 其余形态返回错误由插件自行降级——避免宿主静默降级成语义不同的请求。
    fn fetch_request(
        &self,
        req: &HttpRequestSpec,
    ) -> std::result::Result<(u16, String), String> {
        if req.method.eq_ignore_ascii_case("GET") && req.body.is_empty() {
            self.fetch(&req.url)
        } else {
            Err(format!(
                "当前 HttpFetcher 实现不支持 {} {}（仅 GET 无 body）",
                req.method, req.url
            ))
        }
    }
}

/// 一次 HTTP 请求的宿主侧描述（`effect-http-request` 的 Rust 形态）。
#[derive(Debug, Clone)]
pub struct HttpRequestSpec {
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    pub timeout_ms: u32,
}

impl HttpRequestSpec {
    pub fn get(url: impl Into<String>) -> Self {
        Self {
            method: "GET".to_string(),
            url: url.into(),
            headers: Vec::new(),
            body: Vec::new(),
            timeout_ms: 0,
        }
    }
}

/// 生产实现：reqwest 阻塞客户端**懒创建**（不能在 async 上下文建——同 weather 的
/// OnceLock 纪律；效果执行本就要求调用方在 spawn_blocking/同步上下文）。
pub struct ReqwestFetcher {
    timeout_secs: u64,
    client: std::sync::OnceLock<reqwest::blocking::Client>,
}

impl ReqwestFetcher {
    pub fn new(timeout_secs: u64) -> Self {
        Self {
            timeout_secs,
            client: std::sync::OnceLock::new(),
        }
    }
}

impl Default for ReqwestFetcher {
    fn default() -> Self {
        Self::new(8)
    }
}

impl HttpFetcher for ReqwestFetcher {
    fn fetch(&self, url: &str) -> std::result::Result<(u16, String), String> {
        self.fetch_request(&HttpRequestSpec::get(url))
    }

    fn fetch_request(
        &self,
        req: &HttpRequestSpec,
    ) -> std::result::Result<(u16, String), String> {
        let client = self.client.get_or_init(|| {
            reqwest::blocking::Client::builder()
                .timeout(std::time::Duration::from_secs(self.timeout_secs))
                .user_agent(concat!("xiaozhi-server-rust/", env!("CARGO_PKG_VERSION")))
                .build()
                .unwrap_or_else(|_| reqwest::blocking::Client::new())
        });
        let method = reqwest::Method::from_bytes(req.method.to_uppercase().as_bytes())
            .map_err(|e| format!("非法 HTTP 方法 {}：{e}", req.method))?;
        let mut builder = client.request(method, &req.url);
        for (k, v) in &req.headers {
            builder = builder.header(k.as_str(), v.as_str());
        }
        if !req.body.is_empty() {
            builder = builder.body(req.body.clone());
        }
        if req.timeout_ms > 0 {
            builder = builder.timeout(std::time::Duration::from_millis(req.timeout_ms as u64));
        }
        let resp = builder.send().map_err(|e| e.to_string())?;
        let status = resp.status().as_u16();
        let body = resp.text().map_err(|e| e.to_string())?;
        Ok((status, body))
    }
}

/// 单次 fetch 的回灌载荷（`http.responses` 批量事件的元素形状，见 WIT 注释）。
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct HttpFeedback {
    pub id: String,
    pub status: u16,
    pub body: String,
    pub error: Option<String>,
}

impl HttpFeedback {
    pub fn to_payload(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| {
            // 序列化只可能因非字符串键/控制字符失败：给出可对账的最小错误体
            format!(
                "{{\"id\":\"{}\",\"status\":0,\"body\":\"\",\"error\":\"payload 序列化失败\"}}",
                self.id
            )
        })
    }
}

/// 按步批量回灌：一步产生的全部请求合成一个 `http.responses` 事件（数组 payload）。
/// 深度按**步**计而非请求数——天气冷路径一步并发拉全部省份列表（35+ 请求）是合法用法。
pub fn feedback_batch_payload(batch: &[HttpFeedback]) -> String {
    serde_json::to_string(batch).unwrap_or_else(|_| "[]".to_string())
}

/// 执行一批已放行的效果。逐效果执行、逐效果记日志；失败聚合上报不中断其余效果。
/// http-get 效果同步执行并返回回灌事件载荷（registry 决定是否继续回灌）。
pub fn execute_effects(
    plugin_id: &str,
    effects: &[Effect],
    kv: &Arc<KvStore>,
    fetcher: &Arc<dyn HttpFetcher>,
) -> (Vec<EffectError>, Vec<HttpFeedback>) {
    execute_effects_with_lan_queue(plugin_id, effects, kv, fetcher, None)
}

/// 执行效果并可选地将高层 LAN 命令放入有界队列。
/// `None` 保持旧调用方的显式 Unsupported 语义；宿主已绑定队列时才入队。
pub fn execute_effects_with_lan_queue(
    plugin_id: &str,
    effects: &[Effect],
    kv: &Arc<KvStore>,
    fetcher: &Arc<dyn HttpFetcher>,
    lan_queue: Option<&LanSocketQueue>,
) -> (Vec<EffectError>, Vec<HttpFeedback>) {
    let mut errs = Vec::new();
    let mut feedback = Vec::new();
    for e in effects {
        let res = match e {
            Effect::Log { level, message } => {
                match level.as_str() {
                    "warn" => tracing::warn!(plugin = plugin_id, "{message}"),
                    "error" => tracing::error!(plugin = plugin_id, "{message}"),
                    "debug" => tracing::debug!(plugin = plugin_id, "{message}"),
                    _ => tracing::info!(plugin = plugin_id, "{message}"),
                }
                Ok(())
            }
            Effect::KvPut { key, value } => kv.put(plugin_id, key, value).map_err(Into::into),
            Effect::KvDelete { key } => {
                let existed = kv.delete(plugin_id, key);
                tracing::debug!(plugin = plugin_id, key, existed, "kv-delete 效果");
                Ok(())
            }
            Effect::HttpRequest {
                request_id,
                method,
                url,
                headers,
                body,
                timeout_ms,
            } => {
                let spec = HttpRequestSpec {
                    method: method.clone(),
                    url: url.clone(),
                    headers: headers.clone(),
                    body: body.clone(),
                    timeout_ms: *timeout_ms,
                };
                match fetcher.fetch_request(&spec) {
                    Ok((status, body)) => feedback.push(HttpFeedback {
                        id: request_id.clone(),
                        status,
                        body,
                        error: None,
                    }),
                    Err(err) => feedback.push(HttpFeedback {
                        id: request_id.clone(),
                        status: 0,
                        body: String::new(),
                        error: Some(err),
                    }),
                }
                Ok(())
            }
            Effect::LanStatus {
                request_id,
                device_handle,
            } => match lan_queue {
                Some(queue) => queue
                    .push(LanSocketCommand::Status {
                        plugin_id: plugin_id.to_string(),
                        request_id: request_id.clone(),
                        device_handle: device_handle.clone(),
                    })
                    .map_err(anyhow::Error::msg),
                None => Err(anyhow::anyhow!("lan-socket 宿主服务未绑定")),
            },
            Effect::LanSetDp {
                request_id,
                device_handle,
                dp,
                value_json,
            } => match lan_queue {
                Some(queue) => queue
                    .push(LanSocketCommand::SetDp {
                        plugin_id: plugin_id.to_string(),
                        request_id: request_id.clone(),
                        device_handle: device_handle.clone(),
                        dp: dp.clone(),
                        value_json: value_json.clone(),
                    })
                    .map_err(anyhow::Error::msg),
                None => Err(anyhow::anyhow!("lan-socket 宿主服务未绑定")),
            },
        };
        if let Err(err) = res {
            tracing::warn!(plugin = plugin_id, kind = e.kind(), %err, "效果执行失败");
            errs.push(EffectError::Failed(err));
        }
    }
    (errs, feedback)
}

// ============================================================
// WIT ↔ 领域转换
// ============================================================

use super::contract::xiaozhi::plugin::types as wit;

impl From<wit::Effect> for Effect {
    fn from(e: wit::Effect) -> Self {
        match e {
            wit::Effect::Log(l) => Effect::Log {
                level: l.level,
                message: l.message,
            },
            wit::Effect::KvPut(k) => Effect::KvPut {
                key: k.key,
                value: k.value,
            },
            wit::Effect::KvDelete(k) => Effect::KvDelete { key: k.key },
            wit::Effect::HttpRequest(h) => Effect::HttpRequest {
                request_id: h.request_id,
                method: h.method,
                url: h.url,
                headers: h.headers,
                body: h.body,
                timeout_ms: h.timeout_ms as u32,
            },
            wit::Effect::LanStatus(l) => Effect::LanStatus {
                request_id: l.request_id,
                device_handle: l.device_handle,
            },
            wit::Effect::LanSetDp(l) => Effect::LanSetDp {
                request_id: l.request_id,
                device_handle: l.device_handle,
                dp: l.dp,
                value_json: l.value_json,
            },
        }
    }
}
