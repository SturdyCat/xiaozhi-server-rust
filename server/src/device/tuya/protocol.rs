//! 涂鸦云 OpenAPI **纯协议语义单源**（阶段三 S3b 证据切片）。
//!
//! 本文件只含可独立编译的协议事实：常量、官方 v1.0 签名、请求构造与
//! 请求/响应形状。**刻意不依赖宿主**——不 import `crate::`、不带 reqwest/tokio，
//! 仅 hmac + sha2（纯 Rust，可编译到 wasm32）。宿主 `cloud` 模块与 guest
//! 插件（`#[path]` 单源引用，见 tools/wasm-fixtures）共用同一份源码：
//! 「协议语义在插件侧可执行」以**逐字节黄金向量对拍**钉死
//! （`plugins/wasm/tests/protocol.rs`），而不是停在纸面承诺。
//!
//! 边界（与 tuya 策略 seam 的铁律一致）：`access_secret` 属于凭据，
//! **永不跨 WASM 边界**——guest 侧本文件只用于证明语义可共享；生产签名调用方
//! 仍是持有凭证的 native `CloudClient`，待契约 config-read 落地后插件才可
//! 以自身配置命名空间承接完整签名链路。

use hmac::digest::{KeyInit, Mac};
use hmac::Hmac;
use serde_json::Value;
use sha2::Sha256;

// ---- 协议常量（涂鸦云 OpenAPI；待真机验证项集中在此，勿散落）----

/// token 端点（grant_type=1 = 简单模式 token；`/v1/token` 会 1108）。
pub(crate) const PATH_TOKEN: &str = "/v1.0/token?grant_type=1";
/// 账号授权设备清单端点（免 uid，含 local_key；2026-11-03 cn 区真机验证通过）。
pub(crate) const PATH_DEVICES: &str = "/v1.0/iot-01/associated-users/devices";
/// 分页大小（架构口径 50）。
pub(crate) const PAGE_SIZE: u32 = 50;
/// 翻页硬上限（5000 台设备的家，护住"服务端忽略游标"场景下的死循环）。
pub(crate) const MAX_PAGES: u32 = 100;

/// 一个待发送的云请求（方法/URL/头/原始 body；GET 的 body 为空串）。
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CloudRequest {
    pub method: &'static str,
    pub url: String,
    pub headers: Vec<(String, String)>,
    /// 原始 JSON 文本——签名与发送用**同一串**，保证 Content-SHA256 一致。
    pub body: String,
}

/// 云响应（状态码 + 原始 body 文本）。
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CloudResponse {
    pub status: u16,
    pub body: String,
}

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

/// 从涂鸦 token 回包提取 `(access_token, expire_time)`（成功形态；阶段三 S3c 插件侧
/// 云签名链路用）。失败形态返回 `code=<n>: <msg>` 业务性错误文本——宿主侧的
/// Auth/Api 错误**分类**是业务语义，仍在 `cloud.rs::api_error`（不进单源）。
pub(crate) fn parse_token_ok(body: &str) -> Result<(String, i64), String> {
    let v: Value = serde_json::from_str(body).map_err(|e| format!("云响应不是 JSON: {e}"))?;
    if v.get("success").and_then(Value::as_bool) != Some(true) {
        let code = v.get("code").and_then(Value::as_i64).unwrap_or(0);
        let msg = v.get("msg").and_then(Value::as_str).unwrap_or("unknown");
        return Err(format!("code={code}: {msg}"));
    }
    let result = v.get("result").ok_or("token 回包缺 result")?;
    let token = result
        .get("access_token")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or("token 回包缺 access_token")?
        .to_string();
    // expire_time = 剩余有效秒数；缺省按 7200
    let expire = result
        .get("expire_time")
        .and_then(Value::as_i64)
        .unwrap_or(7200);
    Ok((token, expire))
}

/// 解析涂鸦设备清单回包的**页形状**（S3d 单源扩展）→ (设备行, has_more, 游标)。
///
/// 设备行保持原始 JSON Value——字段投影（id/name/local_key/online… → `CloudDevice`）
/// 是消费方业务语义，宿主侧见 `cloud.rs::parse_devices_page`。形状事实在单源：
/// 真机形状 `result.devices`，兼容旧文档 `result.list` 与裸数组；`has_more` 缺省按
/// 「满页近似」；游标真机 `last_row_key`，兼容旧文档 `last_row_id`。
pub(crate) fn parse_devices_page_ok(body: &str) -> Result<(Vec<Value>, bool, String), String> {
    let v: Value = serde_json::from_str(body).map_err(|e| format!("云响应不是 JSON: {e}"))?;
    if v.get("success").and_then(Value::as_bool) != Some(true) {
        let code = v.get("code").and_then(Value::as_i64).unwrap_or(0);
        let msg = v.get("msg").and_then(Value::as_str).unwrap_or("unknown");
        return Err(format!("code={code}: {msg}"));
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
    let cursor = ["last_row_key", "last_row_id"]
        .iter()
        .find_map(|k| result.get(*k).and_then(Value::as_str))
        .unwrap_or("")
        .to_string();
    Ok((list, has_more, cursor))
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
