//! HTTP/1.1 监听：宿主只做报文分帧，路由与业务语义在插件侧。
//!
//! 请求 → `io.http.request` 事件 → 插件回调 `http-respond` → 宿主写回。
//! 不支持的：HTTP/2、TLS、chunked 请求体、SSE/长轮询（返回 501）。

use std::sync::atomic::AtomicU64;
use std::sync::Arc;

use anyhow::{bail, Result};
use base64::Engine as _;
use serde_json::json;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

use super::{next_request_id, HttpReply, NetManager};
use crate::plugins::wasm::contract::EndpointSpecDto;
use crate::plugins::wasm::event_bus::Envelope;

/// 请求头上限。
const MAX_HEAD_BYTES: usize = 16 * 1024;
/// 请求体上限（可按 resources 调整，本版固定 1 MiB）。
const MAX_BODY_BYTES: usize = 1024 * 1024;
/// 等待插件响应的时间。
const RESPONSE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);
/// 单连接读超时。
const READ_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);

/// 启动 HTTP 监听；返回实际端口与关闭信号。
pub(super) fn spawn(
    net: &Arc<NetManager>,
    _endpoint_id: u64,
    plugin_id: &str,
    spec: &EndpointSpecDto,
) -> Result<(u16, tokio::sync::oneshot::Sender<()>)> {
    let addr = format!("{}:{}", spec.bind, spec.port);
    let listener = std::net::TcpListener::bind(&addr)
        .map_err(|e| anyhow::anyhow!("HTTP 端点绑定失败 {addr}：{e}"))?;
    listener.set_nonblocking(true)?;
    let port = listener.local_addr()?.port();
    let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel();

    let net = Arc::clone(net);
    let plugin = plugin_id.to_string();
    let prefix = spec.path.clone().unwrap_or_else(|| "/".to_string());
    let Some(handle) = net.runtime().cloned() else {
        anyhow::bail!("宿主未绑定 tokio 运行时，无法启动监听循环");
    };
    handle.spawn(async move {
        let Ok(listener) = tokio::net::TcpListener::from_std(listener) else {
            return;
        };
        let counter = Arc::new(AtomicU64::new(1));
        loop {
            tokio::select! {
                _ = &mut shutdown_rx => break,
                accepted = listener.accept() => {
                    let Ok((stream, peer)) = accepted else { continue };
                    let net = Arc::clone(&net);
                    let plugin = plugin.clone();
                    let prefix = prefix.clone();
                    let counter = Arc::clone(&counter);
                    tokio::spawn(async move {
                        let _ = serve_conn(&net, &plugin, &prefix, stream, &peer.to_string(), &counter).await;
                    });
                }
            }
        }
    });

    Ok((port, shutdown_tx))
}

/// 处理单个 HTTP 连接（一问一答，`Connection: close`）。
async fn serve_conn(
    net: &Arc<NetManager>,
    plugin_id: &str,
    path_prefix: &str,
    stream: tokio::net::TcpStream,
    peer: &str,
    counter: &AtomicU64,
) -> Result<()> {
    let (read_half, mut write_half) = stream.into_split();
    let mut reader = BufReader::new(read_half);

    let Some(req) = read_request(&mut reader).await? else {
        return Ok(());
    };

    // 路径前缀匹配（"/" = 全收）
    if path_prefix != "/" && !req.path.starts_with(path_prefix) {
        let body = b"not found";
        let reply = HttpReply {
            status: 404,
            headers: vec![("content-type".into(), "text/plain".into())],
            body: body.to_vec(),
        };
        write_reply(&mut write_half, &reply).await?;
        return Ok(());
    }

    let request_id = next_request_id(counter);
    let payload = json!({
        "id": request_id,
        "method": req.method,
        "path": req.path,
        "query": req.query,
        "headers": req.headers,
        "body": base64_engine().encode(&req.body),
        "peer": peer,
    });
    net.deliver(
        plugin_id,
        Envelope {
            topic: "io.http.request".to_string(),
            correlation_id: Some(request_id.clone()),
            payload: payload.to_string().into_bytes(),
        },
    );

    let rx = net.wait_http(&request_id)?;
    let reply = match tokio::time::timeout(RESPONSE_TIMEOUT, rx).await {
        Ok(Ok(reply)) => reply,
        _ => HttpReply {
            status: 504,
            headers: vec![("content-type".into(), "text/plain".into())],
            body: b"plugin timeout".to_vec(),
        },
    };
    write_reply(&mut write_half, &reply).await
}

struct ParsedRequest {
    method: String,
    path: String,
    query: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

/// 读一个 HTTP/1.1 请求。
async fn read_request<R: tokio::io::AsyncRead + Unpin>(
    reader: &mut BufReader<R>,
) -> Result<Option<ParsedRequest>> {
    let mut head = Vec::new();
    loop {
        let mut line = String::new();
        let read = tokio::time::timeout(READ_TIMEOUT, reader.read_line(&mut line))
            .await
            .map_err(|_| anyhow::anyhow!("HTTP 头读取超时"))??;
        if read == 0 {
            return Ok(None);
        }
        head.extend_from_slice(line.as_bytes());
        if head.len() > MAX_HEAD_BYTES {
            bail!("HTTP 头超上限（{MAX_HEAD_BYTES} 字节）");
        }
        if line == "\r\n" || line == "\n" {
            break;
        }
        // 抹掉行尾便于后续切分
        while head.last() == Some(&b'\n') || head.last() == Some(&b'\r') {
            head.pop();
        }
        head.push(b'\n');
    }

    let text = String::from_utf8_lossy(&head).to_string();
    let mut lines = text.lines().filter(|l| !l.is_empty());
    let Some(request_line) = lines.next() else {
        return Ok(None);
    };
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("").to_uppercase();
    let target = parts.next().unwrap_or("/");
    if method.is_empty() {
        bail!("HTTP 请求行非法：{request_line:?}");
    }

    let (path, query) = match target.split_once('?') {
        Some((p, q)) => (p.to_string(), q.to_string()),
        None => (target.to_string(), String::new()),
    };

    let mut headers = Vec::new();
    let mut content_length = 0usize;
    let mut chunked = false;
    for line in lines {
        let Some((k, v)) = line.split_once(':') else {
            continue;
        };
        let key = k.trim().to_ascii_lowercase();
        let value = v.trim().to_string();
        if key == "content-length" {
            content_length = value.parse::<usize>().unwrap_or(0);
        }
        if key == "transfer-encoding" && value.to_ascii_lowercase().contains("chunked") {
            chunked = true;
        }
        headers.push((key, value));
    }
    if chunked {
        bail!("不支持 chunked 请求体（请用 Content-Length）");
    }
    if content_length > MAX_BODY_BYTES {
        bail!("请求体超上限：{content_length} > {MAX_BODY_BYTES}");
    }

    let mut body = vec![0u8; content_length];
    if content_length > 0 {
        reader.read_exact(&mut body).await?;
    }

    Ok(Some(ParsedRequest {
        method,
        path,
        query,
        headers,
        body,
    }))
}

/// 写响应。
async fn write_reply<W: tokio::io::AsyncWrite + Unpin>(w: &mut W, reply: &HttpReply) -> Result<()> {
    let reason = status_reason(reply.status);
    let mut head = format!("HTTP/1.1 {} {}\r\n", reply.status, reason);
    let mut has_type = false;
    for (k, v) in &reply.headers {
        if k.eq_ignore_ascii_case("content-length") {
            continue;
        }
        if k.eq_ignore_ascii_case("content-type") {
            has_type = true;
        }
        head.push_str(&format!("{k}: {v}\r\n"));
    }
    if !has_type {
        head.push_str("content-type: text/plain\r\n");
    }
    head.push_str(&format!("content-length: {}\r\n", reply.body.len()));
    head.push_str("connection: close\r\n\r\n");
    w.write_all(head.as_bytes()).await?;
    w.write_all(&reply.body).await?;
    w.flush().await?;
    Ok(())
}

fn status_reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        201 => "Created",
        204 => "No Content",
        301 => "Moved Permanently",
        302 => "Found",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        409 => "Conflict",
        413 => "Payload Too Large",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        501 => "Not Implemented",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        504 => "Gateway Timeout",
        _ => "Unknown",
    }
}

fn base64_engine() -> base64::engine::general_purpose::GeneralPurpose {
    base64::engine::general_purpose::STANDARD
}
