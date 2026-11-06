//! 涂鸦云 HTTP 传输层：请求/响应形状 + 发送抽象（自 cloud.rs 拆出，2026-11-03）。
//!
//! 签名与协议逻辑在 [`super::cloud`]；本文件只管"怎么发"。真实实现走 reqwest
//! （10s 超时、rustls）；测试用假 sender 脚本应答，不发真网络请求。

use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use super::cloud::CloudError;

/// 一个待发送的云请求（方法/URL/头/原始 body；GET 的 body 为空串）。
#[derive(Debug, Clone, PartialEq)]
pub struct CloudRequest {
    pub method: &'static str,
    pub url: String,
    pub headers: Vec<(String, String)>,
    /// 原始 JSON 文本——签名与发送用**同一串**，保证 Content-SHA256 一致。
    pub body: String,
}

/// 云响应（状态码 + 原始 body 文本）。
#[derive(Debug, Clone, PartialEq)]
pub struct CloudResponse {
    pub status: u16,
    pub body: String,
}

/// HTTP 发送抽象：真实实现走 reqwest；测试用假 sender 脚本应答。
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
            if !req.body.is_empty() {
                r = r.header("Content-Type", "application/json").body(req.body);
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
