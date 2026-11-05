//! `cloud.rs` 合约测试：签名向量、请求形状、回包解析、翻页与 1010 重试。
//! 全部走 `FakeSender` 脚本应答——**不发真实网络请求、不 sleep**。

use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use super::cloud::*;

/// 脚本化假 sender：按序弹出应答；脚本为空则报错（防意外真请求）。
struct FakeSender {
    script: Mutex<VecDeque<Result<CloudResponse, CloudError>>>,
    seen: Mutex<Vec<CloudRequest>>,
}

impl FakeSender {
    fn new(responses: Vec<Result<CloudResponse, CloudError>>) -> Self {
        FakeSender {
            script: Mutex::new(responses.into()),
            seen: Mutex::new(Vec::new()),
        }
    }

    fn requests(&self) -> Vec<CloudRequest> {
        self.seen.lock().unwrap().clone()
    }
}

impl HttpSender for FakeSender {
    fn send<'a>(
        &'a self,
        req: CloudRequest,
    ) -> Pin<Box<dyn Future<Output = Result<CloudResponse, CloudError>> + Send + 'a>> {
        Box::pin(async move {
            self.seen.lock().unwrap().push(req);
            self.script
                .lock()
                .unwrap()
                .pop_front()
                .expect("脚本应答耗尽——测试不应发出额外请求")
        })
    }
}

/// `Box<Arc<FakeSender>>` 也能当 sender：engine 持有 `Box<dyn HttpSender>`，
/// 测试侧经 Arc 克隆保留句柄读 `seen` 请求记录。
impl HttpSender for Arc<FakeSender> {
    fn send<'a>(
        &'a self,
        req: CloudRequest,
    ) -> Pin<Box<dyn Future<Output = Result<CloudResponse, CloudError>> + Send + 'a>> {
        HttpSender::send(&**self, req)
    }
}

fn ok(body: String) -> Result<CloudResponse, CloudError> {
    Ok(CloudResponse { status: 200, body })
}

// ---- 签名 ----

#[test]
fn hmac_sha256_matches_known_vector() {
    // 公开测试向量：验证 HMAC-SHA256 + 大写 hex 编码链路本身正确
    assert_eq!(
        sign_hmac_hex("key", "The quick brown fox jumps over the lazy dog"),
        "F7BC83F430538424B13298E6AA6FB143EF4D59A14946175997479DBC2D1A3CD8"
    );
    // 官方签名的 Content-SHA256 段：空 body = sha256("")，**小写** hex
    assert_eq!(
        sha256_hex_lower(""),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
}

#[test]
fn official_sign_matches_offline_golden_vector() {
    // 2026-11-03 与官方算法逐字节对拍的黄金向量（python 参照实现计算）：
    // message = aid + "" + t + "GET\n" + sha256("")小写 + "\n\n" + path?query
    let sign = official_sign("asec", "aid", "", "1700000000000", "GET", PATH_TOKEN, "");
    assert_eq!(sign, "42AD3292C57626B9ED5B83615A5F3735CBFA4C9BC9F9F512C8A22A24BDE0C0E2");
}

#[test]
fn token_request_uses_official_signing_and_headers() {
    let req = build_token_request("cn", "aid", "asec", "1700000000000");
    assert_eq!(req.url, "https://openapi.tuyacn.com/v1.0/token?grant_type=1");
    assert_eq!(req.body, "");
    let header = |k: &str| req.headers.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone());
    assert_eq!(header("client_id").as_deref(), Some("aid"));
    assert_eq!(header("access_token").as_deref(), Some(""));
    assert_eq!(header("t").as_deref(), Some("1700000000000"));
    assert_eq!(header("sign_method").as_deref(), Some("HMAC-SHA256"));
    assert_eq!(header("lang").as_deref(), Some("zh"));
    // 旧 mode=simple 头已废弃（涂鸦调试台 cors 签名是平台私有口径，公开 API 不用）
    assert_eq!(header("mode"), None);
    // sign = HMAC(secret, aid + "" + t + "GET\n" + sha256("") + "\n\n" + path?query)
    assert_eq!(
        header("sign").as_deref(),
        Some("42AD3292C57626B9ED5B83615A5F3735CBFA4C9BC9F9F512C8A22A24BDE0C0E2")
    );
}

#[test]
fn devices_request_uses_sorted_query_and_official_signing() {
    let req = build_devices_request("eu", "aid", "asec", "tok", "1700000000001", "row-9");
    // 真机验证的路径 + query 字典序（last_row_key 在 size 前）
    assert_eq!(
        req.url,
        "https://openapi.tuyaeu.com/v1.0/iot-01/associated-users/devices?last_row_key=row-9&size=50"
    );
    let header = |k: &str| req.headers.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone());
    assert_eq!(header("access_token").as_deref(), Some("tok"));
    assert_eq!(
        header("sign").as_deref(),
        Some("1BC6D2A8FDAC9A06F1F31430187F139D3E2FFCC63CD2642875C11B010EB9A357")
    );
}

#[test]
fn devices_first_page_has_size_only() {
    let pq = devices_path_query("");
    assert_eq!(pq, "/v1.0/iot-01/associated-users/devices?size=50");
    let sign = official_sign("asec", "aid", "tok", "1700000000001", "GET", &pq, "");
    assert_eq!(sign, "DBAFB409120A180DF90C1321611DEE26AA345BA7240BB4756D162A2E7DC7D225");
}

#[test]
fn region_hosts_map_to_official_endpoints() {
    assert_eq!(region_host("cn"), "https://openapi.tuyacn.com");
    assert_eq!(region_host("us"), "https://openapi.tuyaus.com");
    assert_eq!(region_host("eu"), "https://openapi.tuyaeu.com");
    assert_eq!(region_host("in"), "https://openapi.tuyain.com");
    // 未知区域兜底 cn（normalize 已前置收敛，这里只验不 panic）
    assert_eq!(region_host("mars"), "https://openapi.tuyacn.com");
}

// ---- 回包解析 ----

#[test]
fn parse_token_response_extracts_token_and_expiry() {
    let (token, expire) = parse_token_response(
        r#"{"success":true,"t":1,"result":{"access_token":"tok-1","expire_time":7200}}"#,
    )
    .expect("parse");
    assert_eq!(token, "tok-1");
    assert_eq!(expire, 7200);
}

#[test]
fn parse_token_response_auth_failure() {
    let err = parse_token_response(r#"{"success":false,"code":1001,"msg":"sign invalid"}"#)
        .expect_err("must fail");
    assert!(err.is_auth(), "签名/凭证类错误应归 Auth");
}

#[test]
fn parse_devices_page_real_shape_devices_key() {
    // 2026-11-03 cn 区真机形状：result.{devices[], has_more, last_row_key, total}
    let body = r#"{"success":true,"result":{"devices":[
        {"id":"d1","name":"插座","model":"m1","local_key":"k1","online":true,"ip":"192.168.1.5","product_id":"p1"},
        {"id":"d2","name":"灯","model":"m2","local_key":"k2","online":false,"ip":"","product_id":"p2"}
    ],"has_more":false,"last_row_key":"8053..F208","total":2}}"#;
    let (devices, has_more, cursor) = parse_devices_page(body).expect("parse");
    assert_eq!(devices.len(), 2);
    assert_eq!(
        devices[0],
        CloudDevice {
            device_id: "d1".into(),
            name: "插座".into(),
            model: "m1".into(),
            local_key: "k1".into(),
            online: true,
            ip: "192.168.1.5".into(),
            product_id: "p1".into(),
        }
    );
    assert!(!has_more);
    assert_eq!(cursor, "8053..F208");
}

#[test]
fn parse_devices_page_legacy_list_shape_compat() {
    // 防御式兼容：旧文档形状 result.{list, last_row_id} 仍可解析
    let body = r#"{"success":true,"result":{"list":[
        {"id":"d1","local_key":"k1"}
    ],"has_more":true,"last_row_id":"d1"}}"#;
    let (devices, has_more, cursor) = parse_devices_page(body).expect("parse");
    assert_eq!(devices.len(), 1);
    assert!(has_more);
    assert_eq!(cursor, "d1");
}

#[test]
fn parse_devices_page_bare_array_shape() {
    // 防御式：result 直接是数组（部分网关/版本形状）——满页近似 has_more
    let mut body = String::from(r#"{"success":true,"result":["#);
    for i in 0..50 {
        body.push_str(&format!(r#"{{"id":"d{i}","local_key":"k{i}"}},"#));
    }
    body.pop();
    body.push_str("]}");
    let (devices, has_more, _) = parse_devices_page(&body).expect("parse");
    assert_eq!(devices.len(), 50);
    assert!(has_more, "裸数组满页按还有更多处理");
}

#[test]
fn credential_error_codes_classify_as_auth_others_as_api() {
    // 凭证类错误码（1001 签名错 / 1010/1011 token 失效）→ Auth；业务码 → Api
    for code in [1001i64, 1010, 1011] {
        let body = format!(r#"{{"success":false,"code":{code},"msg":"x"}}"#);
        let err = parse_devices_page(&body).expect_err("must fail");
        assert!(err.is_auth(), "code {code} 应归 Auth");
    }
    let err = parse_devices_page(r#"{"success":false,"code":2001,"msg":"limit"}"#)
        .expect_err("must fail");
    assert!(!err.is_auth(), "业务码不应归 Auth");
}

// ---- 客户端流程（假 sender）----

#[test]
fn authorized_devices_paginates_until_has_more_false() {
    let sender = FakeSender::new(vec![
        ok(r#"{"success":true,"result":{"access_token":"tok","expire_time":7200}}"#.into()),
        ok(r#"{"success":true,"result":{"devices":[{"id":"d1","local_key":"k1"}],"has_more":true,"last_row_key":"d1"}}"#.into()),
        ok(r#"{"success":true,"result":{"devices":[{"id":"d2","local_key":"k2"}],"has_more":false}}"#.into()),
    ]);
    let mut client = CloudClient::with_sender(Box::new(sender));
    let (devices, err) = tokio::runtime::Runtime::new()
        .expect("rt")
        .block_on(client.authorized_devices_all("cn", "aid", "asec"));
    assert!(err.is_none(), "全页成功不应有错误");
    assert_eq!(devices.len(), 2);
    assert_eq!(devices[1].device_id, "d2");
}

#[test]
fn partial_page_failure_keeps_fetched_devices() {
    // 第 1 页成功、第 2 页网络失败 → 返回第 1 页数据 + 错误（同步落库为 partial）
    let sender = FakeSender::new(vec![
        ok(r#"{"success":true,"result":{"access_token":"tok","expire_time":7200}}"#.into()),
        ok(r#"{"success":true,"result":{"devices":[{"id":"d1","local_key":"k1"}],"has_more":true,"last_row_key":"d1"}}"#.into()),
        Err(CloudError::Network("第 2 页超时".into())),
    ]);
    let mut client = CloudClient::with_sender(Box::new(sender));
    let (devices, err) = tokio::runtime::Runtime::new()
        .expect("rt")
        .block_on(client.authorized_devices_all("cn", "aid", "asec"));
    assert!(err.is_some());
    assert!(!err.as_ref().expect("err").is_auth());
    assert_eq!(devices.len(), 1, "已成功的页不丢");
}

#[test]
fn token_error_1010_retries_once_with_fresh_token() {
    // 首个 token 有效；第 1 页回 1010 → 重取 token（第 2 个 token 响应）→ 重试成功
    let sender = Arc::new(FakeSender::new(vec![
        ok(r#"{"success":true,"result":{"access_token":"tok-a","expire_time":7200}}"#.into()),
        ok(r#"{"success":false,"code":1010,"msg":"token invalid"}"#.into()),
        ok(r#"{"success":true,"result":{"access_token":"tok-b","expire_time":7200}}"#.into()),
        ok(r#"{"success":true,"result":{"devices":[{"id":"d1","local_key":"k1"}],"has_more":false}}"#.into()),
    ]));
    let mut client = CloudClient::with_sender(Box::new(sender.clone()));
    let (devices, err) = tokio::runtime::Runtime::new()
        .expect("rt")
        .block_on(client.authorized_devices_all("cn", "aid", "asec"));
    assert!(err.is_none());
    assert_eq!(devices.len(), 1);

    let reqs = sender.requests();
    assert_eq!(reqs.len(), 4, "token→page→token→page");
    let header = |r: &CloudRequest, k: &str| {
        r.headers
            .iter()
            .find(|(n, _)| n == k)
            .map(|(_, v)| v.clone())
    };
    assert_eq!(header(&reqs[1], "access_token").as_deref(), Some("tok-a"));
    assert_eq!(
        header(&reqs[3], "access_token").as_deref(),
        Some("tok-b"),
        "重试必须换新 token"
    );
}

#[test]
fn bad_credentials_fail_as_auth_without_retry_loop() {
    // token 一直 1010：重取一次仍失败 → Auth 错误（不无限重试）
    let sender = FakeSender::new(vec![
        ok(r#"{"success":true,"result":{"access_token":"tok-a","expire_time":7200}}"#.into()),
        ok(r#"{"success":false,"code":1011,"msg":"token expired"}"#.into()),
        ok(r#"{"success":true,"result":{"access_token":"tok-b","expire_time":7200}}"#.into()),
        ok(r#"{"success":false,"code":1011,"msg":"token expired"}"#.into()),
    ]);
    let mut client = CloudClient::with_sender(Box::new(sender));
    let (devices, err) = tokio::runtime::Runtime::new()
        .expect("rt")
        .block_on(client.authorized_devices_all("cn", "aid", "asec"));
    let err = err.expect("must fail");
    assert!(err.is_auth());
    assert!(devices.is_empty(), "凭证错误不该有设备数据");
}

#[test]
fn network_failure_classified_as_network() {
    let sender = FakeSender::new(vec![Err(CloudError::Network("超时".into()))]);
    let mut client = CloudClient::with_sender(Box::new(sender));
    let rt = tokio::runtime::Runtime::new().expect("rt");
    let (_, err) = rt.block_on(client.authorized_devices_all("cn", "aid", "asec"));
    let err = err.expect("must fail");
    assert!(!err.is_auth());
}

#[test]
fn cursor_loop_is_bounded_when_server_repeats_page() {
    // 服务端忽略游标：每页都回同样的 d1 + has_more=true → stale 判定终止（≤ MAX_PAGES 次请求）
    let page = ok(r#"{"success":true,"result":{"devices":[{"id":"d1","local_key":"k1"}],"has_more":true,"last_row_key":"d1"}}"#.into());
    let script: Vec<Result<CloudResponse, CloudError>> =
        std::iter::once(ok(r#"{"success":true,"result":{"access_token":"tok","expire_time":7200}}"#.into()))
            .chain(std::iter::repeat(page).take(MAX_PAGES as usize))
            .collect();
    let sender = FakeSender::new(script);
    let mut client = CloudClient::with_sender(Box::new(sender));
    let (devices, err) = tokio::runtime::Runtime::new()
        .expect("rt")
        .block_on(client.authorized_devices_all("cn", "aid", "asec"));
    assert!(err.is_none(), "must terminate with ok");
    assert_eq!(devices.len(), 1, "重复页不产生重复设备");
}

// ---- 调试代理（proxy_request）----

#[test]
fn proxy_get_forwards_with_signature_headers() {
    let sender = Arc::new(FakeSender::new(vec![
        ok(r#"{"success":true,"result":{"access_token":"tok","expire_time":7200}}"#.into()),
        ok(r#"{"success":true,"result":{"online":true}}"#.into()),
    ]));
    let mut client = CloudClient::with_sender(Box::new(sender.clone()));
    let rt = tokio::runtime::Runtime::new().expect("rt");
    let resp = rt.block_on(client.proxy_request("cn", "aid", "asec", "GET", "/v1.0/devices/d1/status", ""))
        .expect("proxy ok");
    assert_eq!(resp.body, r#"{"success":true,"result":{"online":true}}"#);

    let reqs = sender.requests();
    assert_eq!(reqs.len(), 2, "token → 业务请求");
    assert_eq!(reqs[1].url, "https://openapi.tuyacn.com/v1.0/devices/d1/status");
    assert_eq!(reqs[1].method, "GET");
    let header = |r: &CloudRequest, k: &str| r.headers.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone());
    assert_eq!(header(&reqs[1], "access_token").as_deref(), Some("tok"));
    assert_eq!(header(&reqs[1], "sign_method").as_deref(), Some("HMAC-SHA256"));
    // 签名可复算：官方公式对同一输入确定（t 取自请求头）
    let t = header(&reqs[1], "t").expect("t");
    let expect = official_sign("asec", "aid", "tok", &t, "GET", "/v1.0/devices/d1/status", "");
    assert_eq!(header(&reqs[1], "sign").as_deref(), Some(expect.as_str()));
}

#[test]
fn proxy_post_signs_and_sends_same_body() {
    let sender = Arc::new(FakeSender::new(vec![
        ok(r#"{"success":true,"result":{"access_token":"tok","expire_time":7200}}"#.into()),
        ok(r#"{"success":true,"result":true}"#.into()),
    ]));
    let mut client = CloudClient::with_sender(Box::new(sender.clone()));
    let rt = tokio::runtime::Runtime::new().expect("rt");
    let body = r#"{"commands":[{"code":"switch_1","value":true}]}"#;
    let resp = rt.block_on(client.proxy_request("cn", "aid", "asec", "POST", "/v1.0/devices/d1/commands", body))
        .expect("proxy ok");
    assert!(resp.body.contains("success"));

    let reqs = sender.requests();
    let req = &reqs[1];
    assert_eq!(req.method, "POST");
    assert_eq!(req.body, body, "签名与发送必须是同一串 body");
    let header = |k: &str| req.headers.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone());
    assert_eq!(header("Content-Type").as_deref(), Some("application/json"));
    let t = header("t").expect("t");
    let expect = official_sign("asec", "aid", "tok", &t, "POST", "/v1.0/devices/d1/commands", body);
    assert_eq!(header("sign").as_deref(), Some(expect.as_str()));
}

#[test]
fn proxy_retries_once_on_stale_token() {
    let sender = Arc::new(FakeSender::new(vec![
        ok(r#"{"success":true,"result":{"access_token":"tok-a","expire_time":7200}}"#.into()),
        ok(r#"{"success":false,"code":1010,"msg":"token invalid"}"#.into()),
        ok(r#"{"success":true,"result":{"access_token":"tok-b","expire_time":7200}}"#.into()),
        ok(r#"{"success":true,"result":{}}"#.into()),
    ]));
    let mut client = CloudClient::with_sender(Box::new(sender.clone()));
    let rt = tokio::runtime::Runtime::new().expect("rt");
    let resp = rt.block_on(client.proxy_request("cn", "aid", "asec", "GET", "/v1.0/devices", ""))
        .expect("proxy ok after retry");
    assert!(resp.body.contains("success"));

    let reqs = sender.requests();
    assert_eq!(reqs.len(), 4, "token → 1010 → 重取 token → 重试");
    let header = |r: &CloudRequest, k: &str| r.headers.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone());
    assert_eq!(header(&reqs[1], "access_token").as_deref(), Some("tok-a"));
    assert_eq!(header(&reqs[3], "access_token").as_deref(), Some("tok-b"), "重试必须换新 token");
}

#[test]
fn proxy_token_path_signs_without_access_token() {
    // 网关要求 token 端点签名不含 access_token 段（2026-11-03 代理实测带段恒 1004）
    let sender = Arc::new(FakeSender::new(vec![
        ok(r#"{"success":true,"result":{"access_token":"tok-a","expire_time":7200}}"#.into()),
        ok(r#"{"success":true,"result":{"access_token":"tok-x","expire_time":7200}}"#.into()),
    ]));
    let mut client = CloudClient::with_sender(Box::new(sender.clone()));
    let rt = tokio::runtime::Runtime::new().expect("rt");
    let resp = rt.block_on(client.proxy_request("cn", "aid", "asec", "GET", "/v1.0/token?grant_type=1", ""))
        .expect("proxy ok");
    assert!(resp.body.contains("tok-x"));

    let reqs = sender.requests();
    let header = |r: &CloudRequest, k: &str| r.headers.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone());
    assert_eq!(header(&reqs[1], "access_token").as_deref(), Some(""), "token 端点 access_token 头留空");
    let t = header(&reqs[1], "t").expect("t");
    let expect = official_sign("asec", "aid", "", &t, "GET", "/v1.0/token?grant_type=1", "");
    assert_eq!(header(&reqs[1], "sign").as_deref(), Some(expect.as_str()), "签名字符串不含 token 段");
}
