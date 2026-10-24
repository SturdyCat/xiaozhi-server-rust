//! DLNA/UPnP 媒体服务器接入。
//!
//! 职责分工：**SSDP 发现、设备描述、SOAP 动作交给 [`rupnp`]**（成熟实现：多网卡接口
//! 枚举、正确的 M-SEARCH 收发、roxmltree 解析描述、hyper 发 SOAP）——自研简化版只查
//! `MediaServer:1` 单条 ST（漏掉只声明 `:2`/`:3` 的服务器）且组播不枚举接口，实测扫不
//! 到任何服务器，已废弃。本模块只保留 **DIDL 解析**（内置极简 scanner，不引 XML 依赖）
//! 与 URL/时长工具。
//!
//! 数据流：`discover` 并发搜 `MediaServer:1/2/3` 三个 ST 版本（很多现代服务器只声明
//! `:2`/`:3`）→ `browse` 用 `Device::from_url` 从设备描述地址重建设备、找到
//! `ContentDirectory` 服务后 SOAP `Browse` 递归列出音频条目 → 音频直链由
//! [`crate::device::echomuse::music`] 下载、解码为 48k mono 后，经既有音乐通道
//! （`frame_music` + `MediaState::enqueue_music`）实时推流到设备（与语音混音）。

use std::collections::HashSet;
use std::time::Duration;

use rupnp::http::Uri;
use rupnp::ssdp::{SearchTarget, URN};
use serde::Serialize;

/// ContentDirectory 服务 URN（1/2/3 版本逐个尝试，设备声明哪个用哪个）。
const CONTENT_DIRECTORY: &str = "schemas-upnp-org";
const CONTENT_DIRECTORY_SERVICE: &str = "ContentDirectory";

/// 发现的 DLNA 媒体服务器（友好名 + 设备描述地址）。
#[derive(Debug, Clone, Serialize)]
pub struct DlnaServer {
    pub name: String,
    pub location: String,
}

/// 一个可播放的音频条目（序列化给管理台选择用）。
#[derive(Debug, Clone, Serialize)]
pub struct DlnaItem {
    pub title: String,
    pub artist: String,
    pub album: String,
    #[serde(rename = "url")]
    pub url: String,
    /// 时长（秒），未知为 0。
    #[serde(rename = "duration")]
    pub duration: u64,
}

/// 发现局域网内 DLNA 媒体服务器（`MediaServer`）。`timeout` 控制 SSDP 等待窗口。
///
/// 三个 MediaServer 版本的搜索**并发**进行（同一窗口内各自收响应），按描述地址去重。
pub async fn discover(timeout: Duration) -> Vec<DlnaServer> {
    let mut handles = Vec::new();
    for ver in [1u32, 2, 3] {
        handles.push(tokio::spawn(discover_one(
            URN::device(CONTENT_DIRECTORY, "MediaServer", ver),
            timeout,
        )));
    }
    let mut devices = Vec::new();
    for h in handles {
        match h.await {
            Ok(v) => devices.extend(v),
            Err(e) => tracing::warn!("[dlna] SSDP 搜索任务失败：{e}"),
        }
    }
    let mut seen = HashSet::new();
    let mut servers = Vec::new();
    for d in devices {
        let location = d.url().to_string();
        if !seen.insert(location.clone()) {
            continue;
        }
        let name = d.friendly_name().trim();
        let name = if name.is_empty() {
            host_of(&location).to_string()
        } else {
            name.to_string()
        };
        servers.push(DlnaServer { name, location });
    }
    if servers.is_empty() {
        tracing::info!("[dlna] 未发现 DLNA 媒体服务器");
    } else {
        tracing::info!("[dlna] 发现 {} 个 DLNA 媒体服务器", servers.len());
    }
    servers
}

/// 搜一个 ST 版本，收割全部响应（单台设备描述拉取失败只跳过该台，不中断整轮）。
async fn discover_one(target: URN, timeout: Duration) -> Vec<rupnp::Device> {
    let label = format!("{target:?}");
    let search = SearchTarget::URN(target);
    let Ok(stream) = rupnp::discover(&search, timeout, None).await else {
        tracing::warn!("[dlna] SSDP 发现启动失败（ST={label}）");
        return Vec::new();
    };
    let mut stream = std::pin::pin!(stream);
    let mut out = Vec::new();
    while let Some(item) = futures_util::StreamExt::next(&mut stream).await {
        match item {
            Ok(d) => out.push(d),
            Err(e) => tracing::debug!("[dlna] 跳过一台描述拉取失败的设备：{e}"),
        }
    }
    out
}

/// 浏览某服务器（设备描述 `location`）下全部音频条目（递归进容器）。
pub async fn browse(location: &str) -> Result<Vec<DlnaItem>, String> {
    let uri: Uri = location
        .parse()
        .map_err(|_| format!("location 不是合法 URL：{location}"))?;
    let device = rupnp::Device::from_url(uri)
        .await
        .map_err(|e| format!("拉取设备描述失败: {e}"))?;
    let service = [1u32, 2, 3].iter().find_map(|v| {
        device.find_service(&URN::service(
            CONTENT_DIRECTORY,
            CONTENT_DIRECTORY_SERVICE,
            *v,
        ))
    });
    let Some(service) = service else {
        return Err("该设备没有 ContentDirectory 服务".to_string());
    };
    let mut items = Vec::new();
    if let Err(e) = browse_container(&device, service, "0", 0, &mut items).await {
        if items.is_empty() {
            return Err(e);
        }
        tracing::warn!("[dlna] 浏览中断（已收集 {} 项）：{e}", items.len());
    }
    Ok(items)
}

// ─────────────────────────────── ContentDirectory 浏览 ───────────────────────────────

/// 递归浏览某个容器（ObjectID），把音频条目追加到 `out`，容器则下钻。
async fn browse_container(
    device: &rupnp::Device,
    service: &rupnp::Service,
    object_id: &str,
    depth: u32,
    out: &mut Vec<DlnaItem>,
) -> Result<(), String> {
    if depth > 5 || out.len() >= 3000 {
        return Ok(());
    }
    let didl = soap_browse(device, service, object_id).await?;
    for e in parse_didl(&didl) {
        if e.is_container {
            // 递归 async fn 必须显式 Box::pin 引入间接层，否则 future 尺寸无穷递归。
            Box::pin(browse_container(device, service, &e.id, depth + 1, out)).await?;
        } else if !e.res_url.is_empty() && is_audio(&e.class) {
            out.push(DlnaItem {
                title: if e.title.is_empty() {
                    e.res_url.clone()
                } else {
                    e.title
                },
                artist: e.artist,
                album: e.album,
                url: resolve_url(&device.url().to_string(), &e.res_url),
                duration: parse_duration(&e.duration),
            });
        }
    }
    Ok(())
}

/// 仅保留明确音频类，或没有 class 信息（很多服务器不填）也放行。
fn is_audio(class: &str) -> bool {
    let c = class.to_lowercase();
    c.is_empty() || c.contains("audio") || c.contains("music") || c.contains("song")
}

/// 发 SOAP `Browse`（参数与 SOAPACTION 头由 rupnp 按服务描述自动封装），返回 `Result`
/// 里的 DIDL 文本。
async fn soap_browse(
    device: &rupnp::Device,
    service: &rupnp::Service,
    object_id: &str,
) -> Result<String, String> {
    let args = format!(
        "<ObjectID>{object_id}</ObjectID>\
         <BrowseFlag>BrowseDirectChildren</BrowseFlag>\
         <Filter>*</Filter>\
         <StartingIndex>0</StartingIndex>\
         <RequestedCount>0</RequestedCount>\
         <SortCriteria></SortCriteria>"
    );
    let resp = service
        .action(device.url(), "Browse", &args)
        .await
        .map_err(|e| format!("Browse 请求失败: {e}"))?;
    Ok(resp
        .get("Result")
        .map(|s| s.as_str())
        .unwrap_or("")
        .to_string())
}

#[derive(Debug, Default)]
struct DidlEntry {
    id: String,
    class: String,
    title: String,
    artist: String,
    album: String,
    res_url: String,
    res_mime: String,
    duration: String,
    is_container: bool,
}

/// 解析 DIDL-Lite：收集 `item`/`container`；`res` 的文本是直链，属性带时长与 MIME。
fn parse_didl(xml: &str) -> Vec<DidlEntry> {
    let events = scan_xml(xml);
    let mut entries = Vec::new();
    let mut cur: Option<DidlEntry> = None;
    let mut capture: Option<String> = None;
    for ev in &events {
        match ev {
            XEv::Start(name, attrs) => {
                let l = local(name);
                match l.as_str() {
                    "item" | "container" => {
                        cur = Some(DidlEntry {
                            id: attr(attrs, "id"),
                            is_container: l == "container",
                            ..Default::default()
                        });
                    }
                    "title" | "class" | "creator" | "album" | "res" => {
                        capture = Some(l.clone());
                        if l == "res" {
                            if let Some(c) = cur.as_mut() {
                                c.duration = attr(attrs, "duration");
                                c.res_mime = protocol_mime(&attr(attrs, "protocolinfo"));
                            }
                        }
                    }
                    _ => {}
                }
            }
            XEv::Text(t) => {
                if let (Some(c), Some(cap)) = (cur.as_mut(), capture.as_ref()) {
                    match cap.as_str() {
                        "title" => c.title = t.clone(),
                        "class" => c.class = t.clone(),
                        "creator" => c.artist = t.clone(),
                        "album" => c.album = t.clone(),
                        "res" => {
                            if c.res_url.is_empty() {
                                c.res_url = t.clone();
                            }
                        }
                        _ => {}
                    }
                }
            }
            XEv::End(name) => {
                let l = local(name);
                match l.as_str() {
                    "item" | "container" => {
                        if let Some(e) = cur.take() {
                            entries.push(e);
                        }
                        capture = None;
                    }
                    "title" | "class" | "creator" | "album" | "res" => {
                        capture = None;
                    }
                    _ => {}
                }
            }
        }
    }
    entries
}

/// `protocolInfo` 形如 `http-get:*:audio/mpeg:*` → 第 3 段是 MIME 类型。
fn protocol_mime(pi: &str) -> String {
    pi.split(':').nth(2).unwrap_or("").to_string()
}

/// 时长字符串 `H:MM:SS.mmm` 或 `MM:SS` → 秒。
fn parse_duration(s: &str) -> u64 {
    let s = s.trim();
    if s.is_empty() {
        return 0;
    }
    let parts: Vec<&str> = s.split(':').collect();
    let mut secs: f64 = 0.0;
    for (i, p) in parts.iter().enumerate().rev() {
        let val: f64 = p.parse().unwrap_or(0.0);
        secs += val * 60f64.powi(i as i32);
    }
    secs as u64
}

// ─────────────────────────────── URL 工具 ───────────────────────────────

/// 把相对直链 URL 解析成绝对地址（基于设备描述的 base；`http(s):` 原样返回）。
fn resolve_url(base: &str, rel: &str) -> String {
    let rel = rel.trim();
    if rel.starts_with("http://") || rel.starts_with("https://") {
        return rel.to_string();
    }
    if let Some(scheme_end) = base.find("//") {
        if let Some(auth_end) = base[scheme_end + 2..].find('/') {
            let root = &base[..scheme_end + 2 + auth_end];
            if rel.starts_with('/') {
                return format!("{root}{rel}");
            }
            let dir = &base[..base.rfind('/').unwrap_or(base.len())];
            return format!("{dir}/{rel}");
        }
    }
    rel.to_string()
}

/// 取 URL 的 host（用于设备名兜底）。
fn host_of(url: &str) -> &str {
    url.split("//")
        .nth(1)
        .and_then(|r| r.split('/').next())
        .unwrap_or(url)
}

/// 下载音频直链为字节（DLNA 推流第一步）。带超时，避免卡死。
pub(crate) async fn http_get_bytes(url: &str) -> Result<Vec<u8>, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(60))
        .build()
        .map_err(|e| format!("HTTP 客户端创建失败: {e}"))?;
    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("音频下载失败: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("音频下载 HTTP {}", resp.status()));
    }
    resp.bytes()
        .await
        .map(|b| b.to_vec())
        .map_err(|e| format!("读取音频流失败: {e}"))
}

// ─────────────────────────────── 极简 XML scanner ───────────────────────────────
// 只服务规整的 DIDL：按标签名（忽略命名空间前缀）+ 属性遍历，不校验结构。

#[derive(Debug)]
enum XEv<'a> {
    Start(&'a str, Vec<(&'a str, String)>),
    End(&'a str),
    Text(String),
}

/// 把 XML 文本切成开始/结束/文本事件（借用输入切片，文本转成 owned）。
fn scan_xml(input: &str) -> Vec<XEv<'_>> {
    let bytes = input.as_bytes();
    let n = bytes.len();
    let mut i = 0;
    let mut events = Vec::new();
    while i < n {
        if bytes[i] == b'<' {
            if i + 1 < n && bytes[i + 1] == b'/' {
                // 结束标签
                let start = i + 2;
                let mut k = start;
                while k < n && bytes[k] != b'>' {
                    k += 1;
                }
                let name = String::from_utf8_lossy(&bytes[start..k])
                    .trim()
                    .to_ascii_lowercase();
                events.push(XEv::End(Box::leak(name.into_boxed_str())));
                i = if k < n { k + 1 } else { n };
            } else if i + 1 < n && (bytes[i + 1] == b'?' || bytes[i + 1] == b'!') {
                // 声明 / 注释：跳到 '>'
                let mut k = i + 2;
                while k < n && bytes[k] != b'>' {
                    k += 1;
                }
                i = if k < n { k + 1 } else { n };
            } else {
                // 开始标签（可能自闭合）
                let start = i + 1;
                let mut k = start;
                while k < n
                    && bytes[k] != b'>'
                    && bytes[k] != b'/'
                    && !bytes[k].is_ascii_whitespace()
                {
                    k += 1;
                }
                let name = String::from_utf8_lossy(&bytes[start..k])
                    .trim()
                    .to_ascii_lowercase();
                let mut attrs: Vec<(&str, String)> = Vec::new();
                let mut p = k;
                let mut self_close = false;
                'attrs: while p < n {
                    while p < n && bytes[p].is_ascii_whitespace() {
                        p += 1;
                    }
                    if p >= n {
                        break;
                    }
                    if bytes[p] == b'>' {
                        p += 1;
                        break 'attrs;
                    }
                    if bytes[p] == b'/' {
                        self_close = true;
                        p += 1;
                        if p < n && bytes[p] == b'>' {
                            p += 1;
                        }
                        break 'attrs;
                    }
                    let an = p;
                    while p < n
                        && bytes[p] != b'='
                        && !bytes[p].is_ascii_whitespace()
                        && bytes[p] != b'>'
                    {
                        p += 1;
                    }
                    let attr_name = String::from_utf8_lossy(&bytes[an..p])
                        .trim()
                        .to_ascii_lowercase();
                    // 跳过空白与 '='
                    while p < n && (bytes[p].is_ascii_whitespace() || bytes[p] == b'=') {
                        p += 1;
                    }
                    let mut val = String::new();
                    if p < n && (bytes[p] == b'"' || bytes[p] == b'\'') {
                        let q = bytes[p];
                        p += 1;
                        let vs = p;
                        while p < n && bytes[p] != q {
                            p += 1;
                        }
                        val = unescape_str(&String::from_utf8_lossy(&bytes[vs..p]));
                        p += 1; // 跳过闭合引号
                    }
                    let an_str: &str = Box::leak(attr_name.into_boxed_str());
                    attrs.push((an_str, val));
                }
                let name_str: &'static str = Box::leak(name.into_boxed_str());
                let attrs_owned: Vec<(&str, String)> = attrs
                    .into_iter()
                    .map(|(k, v)| {
                        let kk: &'static str = Box::leak(k.to_string().into_boxed_str());
                        (kk, v)
                    })
                    .collect();
                events.push(XEv::Start(name_str, attrs_owned));
                if self_close {
                    events.push(XEv::End(name_str));
                }
                i = p;
            }
        } else {
            // 文本直到下一个 '<'
            let start = i;
            while i < n && bytes[i] != b'<' {
                i += 1;
            }
            let text = unescape_str(&String::from_utf8_lossy(&bytes[start..i]))
                .trim()
                .to_string();
            if !text.is_empty() {
                events.push(XEv::Text(text));
            }
        }
    }
    events
}

/// 取元素/属性的本地名（去掉命名空间前缀，如 `upnp:class` → `class`），并转小写。
fn local(name: &str) -> String {
    name.rsplit(':').next().unwrap_or(name).to_ascii_lowercase()
}

/// 在属性表里按本地名查值。
fn attr<'a>(attrs: &'a [(&'a str, String)], name: &str) -> String {
    let target = name.to_ascii_lowercase();
    attrs
        .iter()
        .find(|(k, _)| local(k) == target)
        .map(|(_, v)| v.clone())
        .unwrap_or_default()
}

/// 还原常见 XML 实体（`&amp; &lt; &gt; &quot; &apos; &#nnn;`）。
fn unescape_str(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let bytes = s.as_bytes();
    let n = bytes.len();
    let mut out = String::with_capacity(n);
    let mut i = 0;
    while i < n {
        if bytes[i] == b'&' {
            if let Some(end) = s[i..].find(';') {
                let ent = &s[i + 1..i + end];
                let ch = match ent {
                    "amp" => '&',
                    "lt" => '<',
                    "gt" => '>',
                    "quot" => '"',
                    "apos" => '\'',
                    _ => {
                        if let Some(code) =
                            ent.strip_prefix('#').and_then(|c| c.parse::<u32>().ok())
                        {
                            if let Some(c) = char::from_u32(code) {
                                out.push(c);
                                i = i + end + 1;
                                continue;
                            }
                        }
                        '&'
                    }
                };
                out.push(ch);
                i = i + end + 1;
                continue;
            }
            out.push('&');
            i += 1;
        } else {
            let next = s[i..].find('&').map(|d| i + d).unwrap_or(n);
            out.push_str(&s[i..next]);
            i = next;
        }
    }
    out
}

#[cfg(test)]
#[path = "dlna_tests.rs"]
mod tests;
