//! 涂鸦云 OpenAPI 客户端（**仅目录同步用**——控制与状态读取纯局域网，零云依赖）。
//!
//! ## 签名（simple_sign 模式，HMAC-SHA256 hex）
//!
//! - token 请求：`sign = HMAC-SHA256(secret, client_id + t)`（无 access_token，按空串拼接）；
//! - 业务请求：`sign = HMAC-SHA256(secret, client_id + access_token + t)`；
//! - 统一头：`client_id / t(13 位毫秒) / sign_mode=simple_sign`（业务请求另有 `access_token`）。
//!   simple_sign 不参与 body/url 计算。
//!
//! ## 待真机验证项（集中在本文件常量，勿散落多处）
//!
//! - 端点 host：`openapi.tuya{cn,us,eu,in}.com`（架构文档记作 `openapi.tuya{,aus,eu,in}`，
//!   按官方公开文档修正为 `tuyacn`——见 [`region_host`]）；
//! - 设备清单端点 `/v1.0/iot-01/associated-users/actions/authorized-devices` 的翻页参数
//!   （`page_size` + `last_row_id` 游标）与回包形状（`result.list` / 裸数组两种都解析）；
//! - token 失效错误码 1010/1011 → 重取一次重试（重试判定见 `is_token_failure`）。
//!
//! 测试（`cloud_tests.rs`）用 [`HttpSender`] 假实现驱动：签名向量、请求形状、分页、
//! 1010 重试全覆盖，**不发真网络请求**。

use std::future::Future;
use std::pin::Pin;
use std::time::{Duration, Instant};

use hmac::Hmac;
// hmac 0.13：`new_from_slice` 来自 digest 树的 KeyInit，`update`/`finalize` 来自
// digest 的 `Mac` trait（hmac 以 `pub use digest` 转出，二者都必须在作用域内）
use hmac::digest::{KeyInit, Mac};
use serde_json::Value;
use sha2::Sha256;

/// 云侧设备目录一行（authorized-devices 回包投影）。
#[derive(Debug, Clone, PartialEq)]
pub struct CloudDevice {
    pub device_id: String,
    pub name: String,
    pub model: String,
    pub local_key: String,
    pub online: bool,
    pub ip: String,
    pub product_id: String,
}

/// 云调用失败的三归类（同步编排据此写 sync_history）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CloudError {
    /// 凭证/鉴权问题（签名错、token 拿不到、账号无权）→ cloud_auth_failed。
    Auth(String),
    /// 网络/超时 → network_error。
    Network(String),
    /// 云端返回了非鉴权类业务错误 → network_error 同类处理（重试无益，但不是凭证问题）。
    Api(String),
}

impl CloudError {
    /// 是否为凭证/鉴权问题（同步编排与测试共用的归类判据；生产路径按变体 match）。
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn is_auth(&self) -> bool {
        matches!(self, CloudError::Auth(_))
    }

    /// 日志安全文本（不含 secret；错误消息本身也不含）。
    pub fn message(&self) -> String {
        match self {
            CloudError::Auth(m) | CloudError::Network(m) | CloudError::Api(m) => m.clone(),
        }
    }
}

// ---- 协议常量（涂鸦云 OpenAPI；待真机验证项集中在此，勿散落）----

/// token 端点（grant_type=1 = 简单模式 token）。
pub(crate) const PATH_TOKEN: &str = "/v1/token?grant_type=1";
/// 账号授权设备清单端点（免 uid，含 local_key）。
pub(crate) const PATH_DEVICES: &str = "/v1.0/iot-01/associated-users/actions/authorized-devices";
/// 分页大小（架构口径 50）。
pub(crate) const PAGE_SIZE: u32 = 50;
/// 翻页硬上限（5000 台设备的家，护住"服务端忽略游标"场景下的死循环）。
pub(crate) const MAX_PAGES: u32 = 100;
/// token 默认有效期 7200s，提前 300s 刷新。
const TOKEN_REFRESH_MARGIN_SECS: i64 = 300;

/// 区域 → OpenAPI host。未识别区域由调用方 normalize 收敛，这里兜底 cn。
pub(crate) fn region_host(region: &str) -> String {
    match region {
        "us" => "https://openapi.tuyaus.com".into(),
        "eu" => "https://openapi.tuyaeu.com".into(),
        "in" => "https://openapi.tuyain.com".into(),
        _ => "https://openapi.tuyacn.com".into(),
    }
}

/// HMAC-SHA256 → 小写 hex（simple_sign 的 sign 值口径）。
pub(crate) fn sign_hmac_hex(secret: &str, payload: &str) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).expect("HMAC 接受任意长度密钥");
    mac.update(payload.as_bytes());
    hex_lower(&mac.finalize().into_bytes())
}

fn hex_lower(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

/// 13 位毫秒时间戳（字符串，直接进头）。
pub(crate) fn timestamp_ms() -> String {
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    format!("{ms}")
}

/// 构造 token 请求（纯函数：t 由调用方传入，测试可断言确定性）。
pub(crate) fn build_token_request(
    region: &str,
    access_id: &str,
    access_secret: &str,
    t: &str,
) -> CloudRequest {
    let sign = sign_hmac_hex(access_secret, &format!("{access_id}{t}"));
    CloudRequest {
        method: "GET",
        url: format!("{}{PATH_TOKEN}", region_host(region)),
        headers: vec![
            ("client_id".into(), access_id.to_string()),
            ("t".into(), t.to_string()),
            ("sign_mode".into(), "simple_sign".into()),
            ("sign".into(), sign),
        ],
    }
}

/// 构造设备清单请求（带游标翻页）。
pub(crate) fn build_devices_request(
    region: &str,
    access_id: &str,
    access_secret: &str,
    access_token: &str,
    t: &str,
    last_row_id: &str,
) -> CloudRequest {
    let sign = sign_hmac_hex(access_secret, &format!("{access_id}{access_token}{t}"));
    let mut url = format!(
        "{}{PATH_DEVICES}?page_size={PAGE_SIZE}",
        region_host(region)
    );
    if !last_row_id.is_empty() {
        url.push_str("&last_row_id=");
        url.push_str(last_row_id);
    }
    CloudRequest {
        method: "GET",
        url,
        headers: vec![
            ("client_id".into(), access_id.to_string()),
            ("access_token".into(), access_token.to_string()),
            ("t".into(), t.to_string()),
            ("sign_mode".into(), "simple_sign".into()),
            ("sign".into(), sign),
        ],
    }
}

// ---- HTTP 抽象（真实 reqwest 实现 + 测试假实现）----

/// 一个待发送的云请求（方法/URL/头；simple_sign 无 body）。
#[derive(Debug, Clone, PartialEq)]
pub struct CloudRequest {
    pub method: &'static str,
    pub url: String,
    pub headers: Vec<(String, String)>,
}

/// 云响应（状态码 + 原始 body 文本）。
#[derive(Debug, Clone, PartialEq)]
pub struct CloudResponse {
    pub status: u16,
    pub body: String,
}

/// HTTP 发送抽象：真实实现走 reqwest；测试用 [`cloud_tests::FakeSender`] 脚本应答。
pub trait HttpSender: Send + Sync {
    fn send<'a>(
        &'a self,
        req: CloudRequest,
    ) -> Pin<Box<dyn Future<Output = Result<CloudResponse, CloudError>> + Send + 'a>>;
}

/// reqwest 实现（10s 超时；rustls，与项目依赖栈一致）。
pub(crate) struct ReqwestSender {
    client: reqwest::Client,
}

impl ReqwestSender {
    pub(crate) fn new() -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .unwrap_or_default();
        ReqwestSender { client }
    }
}

impl HttpSender for ReqwestSender {
    fn send<'a>(
        &'a self,
        req: CloudRequest,
    ) -> Pin<Box<dyn Future<Output = Result<CloudResponse, CloudError>> + Send + 'a>> {
        Box::pin(async move {
            let mut r = self
                .client
                .request(req.method.parse().unwrap_or(reqwest::Method::GET), &req.url);
            for (k, v) in &req.headers {
                r = r.header(k, v);
            }
            match r.send().await {
                Ok(resp) => {
                    let status = resp.status().as_u16();
                    let body = resp
                        .text()
                        .await
                        .map_err(|e| CloudError::Network(format!("读取云响应失败: {e}")))?;
                    Ok(CloudResponse { status, body })
                }
                Err(e) => {
                    if e.is_timeout() {
                        Err(CloudError::Network("云请求超时（10s）".into()))
                    } else {
                        Err(CloudError::Network(format!("云请求失败: {e}")))
                    }
                }
            }
        })
    }
}

// ---- 回包解析（纯函数，测试直驱）----

/// 解析 token 回包 → (access_token, 有效期秒)。
pub(crate) fn parse_token_response(body: &str) -> Result<(String, i64), CloudError> {
    let v: Value =
        serde_json::from_str(body).map_err(|e| CloudError::Api(format!("云响应不是 JSON: {e}")))?;
    if v.get("success").and_then(Value::as_bool) != Some(true) {
        return Err(api_error(&v));
    }
    let result = v
        .get("result")
        .ok_or_else(|| CloudError::Api("token 回包缺 result".into()))?;
    let token = result
        .get("access_token")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| CloudError::Api("token 回包缺 access_token".into()))?
        .to_string();
    // expire_time = 剩余有效秒数；缺省按 7200
    let expire = result
        .get("expire_time")
        .and_then(Value::as_i64)
        .unwrap_or(7200);
    Ok((token, expire))
}

/// 解析一页设备清单 → (设备, 是否还有更多, 游标)。
///
/// 回包形状防御式解析：`result` 为对象（`list` + `has_more`/`last_row_id`）或裸数组
/// （has_more 以"满页"近似）——真实形状以真机验收为准（见模块头"待真机验证"）。
pub(crate) fn parse_devices_page(
    body: &str,
) -> Result<(Vec<CloudDevice>, bool, String), CloudError> {
    let v: Value =
        serde_json::from_str(body).map_err(|e| CloudError::Api(format!("云响应不是 JSON: {e}")))?;
    if v.get("success").and_then(Value::as_bool) != Some(true) {
        return Err(api_error(&v));
    }
    let Some(result) = v.get("result") else {
        return Ok((Vec::new(), false, String::new()));
    };
    let (list, has_more) = if let Some(arr) = result.as_array() {
        (arr.clone(), arr.len() as u32 >= PAGE_SIZE)
    } else if let Some(arr) = result.get("list").and_then(Value::as_array) {
        let more = result
            .get("has_more")
            .and_then(Value::as_bool)
            .unwrap_or(arr.len() as u32 >= PAGE_SIZE);
        (arr.clone(), more)
    } else {
        (Vec::new(), false)
    };
    let cursor = result
        .get("last_row_id")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let devices = list
        .iter()
        .filter_map(|d| {
            let device_id = d.get("id").and_then(Value::as_str)?.to_string();
            Some(CloudDevice {
                device_id,
                name: d
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
                model: d
                    .get("model")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
                local_key: d
                    .get("local_key")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
                online: d.get("online").and_then(Value::as_bool).unwrap_or(false),
                ip: d
                    .get("ip")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
                product_id: d
                    .get("product_id")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
            })
        })
        .collect();
    Ok((devices, has_more, cursor))
}

/// `{success:false}` 回包 → 错误归类：凭证/token 类错误码归 Auth，其余归 Api。
///
/// Auth 码表（涂鸦 OpenAPI 常见凭证类）：1001 签名错误、1004 token 无效、
/// 1010/1011 token 失效/过期——这些重试或改凭证才可能解决，归 cloud_auth_failed。
fn api_error(v: &Value) -> CloudError {
    let code = v.get("code").and_then(Value::as_i64).unwrap_or(0);
    let msg = v
        .get("msg")
        .and_then(Value::as_str)
        .or_else(|| v.get("message").and_then(Value::as_str))
        .unwrap_or("")
        .to_string();
    if matches!(code, 1001 | 1004 | 1010 | 1011) {
        CloudError::Auth(format!("云鉴权失败（code {code}）: {msg}"))
    } else {
        CloudError::Api(format!("云接口错误（code {code}）: {msg}"))
    }
}

// ---- 客户端（token 缓存 + 1010/1011 重试）----

struct TokenCache {
    token: String,
    /// 拿到 token 的时刻；到期 = fetched_at + (expire - margin)。
    valid_until: Instant,
}

/// 涂鸦云客户端：token 缓存 + 自动翻页拉全量设备清单。
///
/// `Mutex` 由调用方（engine）持有为 `tokio::sync::Mutex`——本类型自身只做逻辑。
pub(crate) struct CloudClient {
    sender: Box<dyn HttpSender>,
    token: Option<TokenCache>,
}

impl CloudClient {
    /// 真实客户端（reqwest）。
    pub(crate) fn new() -> Self {
        CloudClient {
            sender: Box::new(ReqwestSender::new()),
            token: None,
        }
    }

    /// 测试注入假 sender。
    #[cfg(test)]
    pub(crate) fn with_sender(sender: Box<dyn HttpSender>) -> Self {
        CloudClient {
            sender,
            token: None,
        }
    }

    /// 丢弃 token 缓存（凭证指纹变化 / token 失效重试前）。
    pub(crate) fn invalidate(&mut self) {
        self.token = None;
    }

    /// 拉取账号下全部授权设备（自动翻页 + token 失效重试一次）。
    ///
    /// 返回 (已取到的设备, 中途错误)。**中途页失败不丢已取数据**——同步编排据此把
    /// 本次落库为 `partial`（部分成功）而不是 network_error（全失败）。
    pub(crate) async fn authorized_devices_all(
        &mut self,
        region: &str,
        access_id: &str,
        access_secret: &str,
    ) -> (Vec<CloudDevice>, Option<CloudError>) {
        let mut all: Vec<CloudDevice> = Vec::new();
        let mut cursor = String::new();
        for _page in 0..MAX_PAGES {
            let (devices, has_more, next_cursor) = match self
                .fetch_page_with_retry(region, access_id, access_secret, &cursor)
                .await
            {
                Ok(page) => page,
                Err(e) => return (all, Some(e)),
            };
            let stale = !devices.is_empty()
                && devices
                    .iter()
                    .all(|d| all.iter().any(|p| p.device_id == d.device_id));
            if stale {
                // 服务端忽略游标把同一页又发了一遍——不落重复数据，按"已到末页"处理
                tracing::warn!("涂鸦云设备清单翻页未前进（游标被服务端忽略），按末页处理");
                break;
            }
            all.extend(devices);
            if !has_more || next_cursor.is_empty() || next_cursor == cursor {
                break;
            }
            cursor = next_cursor;
        }
        (all, None)
    }

    /// 拉一页：token 失效（1010/1011）→ 丢弃缓存重取一次再试。
    async fn fetch_page_with_retry(
        &mut self,
        region: &str,
        access_id: &str,
        access_secret: &str,
        cursor: &str,
    ) -> Result<(Vec<CloudDevice>, bool, String), CloudError> {
        match self
            .fetch_page(region, access_id, access_secret, cursor)
            .await
        {
            Err(CloudError::Auth(msg)) if is_token_failure(&msg) => {
                tracing::info!("涂鸦云 token 失效，重取后重试一次");
                self.invalidate();
                self.fetch_page(region, access_id, access_secret, cursor)
                    .await
            }
            other => other,
        }
    }

    async fn fetch_page(
        &mut self,
        region: &str,
        access_id: &str,
        access_secret: &str,
        cursor: &str,
    ) -> Result<(Vec<CloudDevice>, bool, String), CloudError> {
        let token = self.ensure_token(region, access_id, access_secret).await?;
        let t = timestamp_ms();
        let req = build_devices_request(region, access_id, access_secret, &token, &t, cursor);
        let resp = self.sender.send(req).await?;
        if resp.status == 401 || resp.status == 403 {
            return Err(CloudError::Auth(format!(
                "云请求被拒（HTTP {}）",
                resp.status
            )));
        }
        if resp.status >= 500 {
            return Err(CloudError::Network(format!(
                "云服务暂不可用（HTTP {}）",
                resp.status
            )));
        }
        parse_devices_page(&resp.body)
    }

    /// 取有效 token（缓存未到期直接用；否则重新获取）。
    async fn ensure_token(
        &mut self,
        region: &str,
        access_id: &str,
        access_secret: &str,
    ) -> Result<String, CloudError> {
        if let Some(cache) = &self.token {
            if cache.valid_until > Instant::now() {
                return Ok(cache.token.clone());
            }
        }
        let t = timestamp_ms();
        let req = build_token_request(region, access_id, access_secret, &t);
        let resp = self.sender.send(req).await?;
        if resp.status == 401 || resp.status == 403 {
            return Err(CloudError::Auth(
                "token 请求被拒：检查 access_id/access_secret 与区域".into(),
            ));
        }
        let (token, expire) = parse_token_response(&resp.body)
            .map_err(|e| CloudError::Auth(format!("获取 token 失败: {}", e.message())))?;
        let ttl = (expire - TOKEN_REFRESH_MARGIN_SECS).max(60) as u64;
        self.token = Some(TokenCache {
            token: token.clone(),
            valid_until: Instant::now() + Duration::from_secs(ttl),
        });
        Ok(token)
    }
}

/// `fetch_page` 报的 Auth 错误里是否为"token 失效"（1010/1011 → 重取一次重试；
/// 1001/1004 是签名/token 无效类，重试也无益）。`parse_*` 已把错误码嵌进文本。
fn is_token_failure(msg: &str) -> bool {
    msg.contains("code 1010") || msg.contains("code 1011")
}
