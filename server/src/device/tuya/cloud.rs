//! 涂鸦云 OpenAPI 客户端（**仅目录同步用**——控制与状态读取纯局域网，零云依赖）。
//!
//! ## 签名（涂鸦官方 v1.0 口径，2026-11-03 真凭证联调**全链路验证通过**）
//!
//! 与官方 SDK（tuya-connector-python `_calculate_sign`）逐字节一致：
//!
//! ```text
//! str_to_sign = METHOD + "\n" + sha256(body).小写hex + "\n" + "\n" + path?排序后query
//! sign        = HMAC-SHA256(secret, client_id + access_token + t + str_to_sign).大写hex
//! ```
//!
//! - body 为空（GET）时 sha256 取空串（`e3b0c442…`）；两段 `\n` 之间是**预留的空
//!   Header 段，必须保留**——2026-11-03 之前所有 1004 sign invalid 的根因就是这里
//!   少了一段换行，**不是凭证错**（调试台黄金样本复核确认）；
//! - query 参数按字典序拼接（`k=v&k=v`）——设备清单翻页 `last_row_key` 排在 `size` 前；
//! - 统一头：`client_id / access_token(token 请求传空) / t(13 位毫秒) /
//!   sign_method=HMAC-SHA256 / sign / lang`。涂鸦调试台自用的 `mode=cors` +
//!   `signVersion=2.0` 是平台前端私有签名（掺会话），公开 API 不适用，勿模仿。
//!
//! ## 端点（2026-11-03 cn 区真机验证：token ✓ 设备详情 ✓ 授权设备清单 ✓ 4 台）
//!
//! - token：`GET /v1.0/token?grant_type=1`（7200s，提前 300s 刷新）；
//! - 设备清单：`GET /v1.0/iot-01/associated-users/devices?size=50`，回包
//!   `result.{devices[], has_more, last_row_key, total}`，翻页带 `last_row_key`；
//!   旧路径 `…/actions/authorized-devices` 恒 1108（cn 网关不存在，已废弃）；
//! - 错误判读：1004=签名/secret 错；1108=路径不存在；28841003=API 组未订阅；
//!   1010/1011=token 失效 → 重取一次重试（`is_token_failure`）。
//!
//! 管理面调试代理：`CloudClient::proxy_request`（`POST /api/tuya/cloud/proxy` 用），
//! 管理台/curl 可直接签名转发任意 `/v1.` 路径，联调免写临时脚本。
//!
//! 测试（`cloud_tests.rs`）用 [`HttpSender`] 假实现驱动：签名向量、请求形状、分页、
//! 1010 重试全覆盖，**不发真网络请求**。

use std::time::{Duration, Instant};

use super::cloud_transport::ReqwestSender;
pub use super::cloud_transport::{CloudRequest, CloudResponse, HttpSender};

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

/// token 端点（grant_type=1 = 简单模式 token；`/v1/token` 会 1108）。
pub(crate) const PATH_TOKEN: &str = "/v1.0/token?grant_type=1";
/// 账号授权设备清单端点（免 uid，含 local_key；2026-11-03 cn 区真机验证通过）。
pub(crate) const PATH_DEVICES: &str = "/v1.0/iot-01/associated-users/devices";
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

/// HMAC-SHA256 → **大写** hex（涂鸦 sign 值口径）。
pub(crate) fn sign_hmac_hex(secret: &str, payload: &str) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).expect("HMAC 接受任意长度密钥");
    mac.update(payload.as_bytes());
    hex_upper(&mac.finalize().into_bytes())
}

/// SHA-256 → **小写** hex（涂鸦签名字符串里的 Content-SHA256 段口径）。
pub(crate) fn sha256_hex_lower(data: &str) -> String {
    use sha2::Digest;
    let mut hash = Sha256::new();
    hash.update(data.as_bytes());
    hash.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// 涂鸦官方 v1.0 签名（与 tuya-connector-python `_calculate_sign` 逐字节一致）。
///
/// `path_query` 必须与实际发送的 URL 路径+query **完全一致**（query 字典序）；
/// `body` 为原始 JSON 文本（GET 传空串）。
pub(crate) fn official_sign(
    secret: &str,
    access_id: &str,
    access_token: &str,
    t: &str,
    method: &str,
    path_query: &str,
    body: &str,
) -> String {
    let content_sha = sha256_hex_lower(body);
    let message = format!("{access_id}{access_token}{t}{method}\n{content_sha}\n\n{path_query}");
    sign_hmac_hex(secret, &message)
}

fn hex_upper(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push_str(&format!("{b:02X}"));
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
    let sign = official_sign(access_secret, access_id, "", t, "GET", PATH_TOKEN, "");
    CloudRequest {
        method: "GET",
        url: format!("{}{PATH_TOKEN}", region_host(region)),
        body: String::new(),
        headers: vec![
            ("client_id".into(), access_id.to_string()),
            ("access_token".into(), String::new()),
            ("t".into(), t.to_string()),
            ("sign_method".into(), "HMAC-SHA256".into()),
            ("sign".into(), sign),
            ("lang".into(), "zh".into()),
        ],
    }
}

/// 设备清单请求的 path?query（纯函数，签名与 URL 共用同一串，测试直驱）。
/// query 字典序：`last_row_key` 在 `size` 前；无游标时只有 `size`。
pub(crate) fn devices_path_query(last_row_key: &str) -> String {
    if last_row_key.is_empty() {
        format!("{PATH_DEVICES}?size={PAGE_SIZE}")
    } else {
        format!("{PATH_DEVICES}?last_row_key={last_row_key}&size={PAGE_SIZE}")
    }
}

/// 构造设备清单请求（带游标翻页）。
pub(crate) fn build_devices_request(
    region: &str,
    access_id: &str,
    access_secret: &str,
    access_token: &str,
    t: &str,
    last_row_key: &str,
) -> CloudRequest {
    let path_query = devices_path_query(last_row_key);
    let sign = official_sign(
        access_secret,
        access_id,
        access_token,
        t,
        "GET",
        &path_query,
        "",
    );
    CloudRequest {
        method: "GET",
        url: format!("{}{}", region_host(region), path_query),
        body: String::new(),
        headers: vec![
            ("client_id".into(), access_id.to_string()),
            ("access_token".into(), access_token.to_string()),
            ("t".into(), t.to_string()),
            ("sign_method".into(), "HMAC-SHA256".into()),
            ("sign".into(), sign),
            ("lang".into(), "zh".into()),
        ],
    }
}

/// 调试代理请求构造（纯函数）：官方签名统一入口，签名与发送共用同一串 body。
pub(crate) fn build_proxy_request(
    region: &str,
    access_id: &str,
    access_secret: &str,
    access_token: &str,
    method: &'static str,
    path_query: &str,
    body_json: &str,
    t: &str,
) -> CloudRequest {
    let sign = official_sign(
        access_secret,
        access_id,
        access_token,
        t,
        method,
        path_query,
        body_json,
    );
    let mut headers = vec![
        ("client_id".into(), access_id.to_string()),
        ("access_token".into(), access_token.to_string()),
        ("t".into(), t.to_string()),
        ("sign_method".into(), "HMAC-SHA256".into()),
        ("sign".into(), sign),
        ("lang".into(), "zh".into()),
    ];
    if !body_json.is_empty() {
        headers.push(("Content-Type".into(), "application/json".into()));
    }
    CloudRequest {
        method,
        url: format!("{}{}", region_host(region), path_query),
        body: body_json.to_string(),
        headers,
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
/// 真实形状（2026-11-03 cn 区真机验证）：`result.{devices[], has_more, last_row_key,
/// total}`。防御式兼容旧文档形状 `result.list`/`last_row_id` 与裸数组（满页近似
/// has_more）。
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
    let (list, has_more) = if let Some(arr) = result.get("devices").and_then(Value::as_array) {
        let more = result
            .get("has_more")
            .and_then(Value::as_bool)
            .unwrap_or(arr.len() as u32 >= PAGE_SIZE);
        (arr.clone(), more)
    } else if let Some(arr) = result.get("list").and_then(Value::as_array) {
        let more = result
            .get("has_more")
            .and_then(Value::as_bool)
            .unwrap_or(arr.len() as u32 >= PAGE_SIZE);
        (arr.clone(), more)
    } else if let Some(arr) = result.as_array() {
        (arr.clone(), arr.len() as u32 >= PAGE_SIZE)
    } else {
        (Vec::new(), false)
    };
    // 游标：真机形状 last_row_key；兼容旧文档 last_row_id
    let cursor = ["last_row_key", "last_row_id"]
        .iter()
        .find_map(|k| result.get(*k).and_then(Value::as_str))
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

    /// 调试代理：按官方签名把任意请求转发到涂鸦云（`POST /api/tuya/cloud/proxy` 用）。
    ///
    /// `method` 限 GET/POST/DELETE（管理面已校验）；`path_query` 必须以 `/v1.` 开头
    /// 且与签名一致；`body_json` 为原始 JSON 文本（非 POST 传空串）。token 失效
    /// （1010/1011）自动重取重试一次；云回包**原样透传**，联调所见即所得。
    pub(crate) async fn proxy_request(
        &mut self,
        region: &str,
        access_id: &str,
        access_secret: &str,
        method: &str,
        path_query: &str,
        body_json: &str,
    ) -> Result<CloudResponse, CloudError> {
        let static_method: &'static str = match method {
            "POST" => "POST",
            "DELETE" => "DELETE",
            _ => "GET",
        };
        // token/refresh-token 端点**签名字符串与 access_token 头都留空**（网关按无
        // token 复算；2026-11-03 代理实测：带 token 段恒 1004）。业务路径用缓存 token。
        let is_token_path = path_query.starts_with("/v1.0/token");
        let token = self.ensure_token(region, access_id, access_secret).await?;
        let bearer = if is_token_path { String::new() } else { token };
        let resp = self
            .send_proxy_once(
                region,
                access_id,
                access_secret,
                &bearer,
                static_method,
                path_query,
                body_json,
            )
            .await?;
        if resp_body_code(&resp.body).map_or(false, |c| c == 1010 || c == 1011) {
            tracing::info!("调试代理遇 token 失效，重取后重试一次");
            self.invalidate();
            let fresh = self.ensure_token(region, access_id, access_secret).await?;
            return self
                .send_proxy_once(
                    region,
                    access_id,
                    access_secret,
                    &fresh,
                    static_method,
                    path_query,
                    body_json,
                )
                .await;
        }
        Ok(resp)
    }

    /// 代理单次发送（构造签名请求 → 发送；token 失效重试在 [`Self::proxy_request`]）。
    async fn send_proxy_once(
        &mut self,
        region: &str,
        access_id: &str,
        access_secret: &str,
        token: &str,
        method: &'static str,
        path_query: &str,
        body_json: &str,
    ) -> Result<CloudResponse, CloudError> {
        let req = build_proxy_request(
            region,
            access_id,
            access_secret,
            token,
            method,
            path_query,
            body_json,
            &timestamp_ms(),
        );
        self.sender.send(req).await
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

/// 云回包 body 里的业务错误码（`{"code":N,...}`），非 JSON 或无 code → None。
fn resp_body_code(body: &str) -> Option<i64> {
    serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|v| v.get("code").and_then(Value::as_i64))
}
