//! DLNA/UPnP 媒体服务器接入（简化实现，纯 Rust，仅依赖 `symphonia` 解码）。
//!
//! 数据流：`discover` 用 SSDP M-SEARCH 组播 `239.255.255.250:1900` 发现
//! `MediaServer` → `fetch_description` 拉设备描述取 `ContentDirectory` 控制 URL →
//! `browse` 经 SOAP `Browse` 递归列出音频条目（标题/艺术家/专辑/直链 URL/时长）。
//! 音频直链由 [`crate::device::echomuse::music`] 下载、解码为 48k mono 后，经既有
//! 音乐通道（`frame_music` + `MediaState::enqueue_music`）实时推流到设备（与语音混音）。
//!
//! 设计取舍（"简化"优先）：
//! - XML 用内置极简 scanner 解析（设备描述/SOAP/DIDL 皆规整），不另引 XML 依赖。
//! - `<res>` 直链直接 HTTP GET，不预先 `GetMediaInfo`（常见服务器均支持直接取流）。
//! - 递归深度与时长上限封顶，避免超大媒体库一次性拖垮服务端。

use std::time::Duration;

use serde::Serialize;

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
pub async fn discover(timeout: Duration) -> Vec<DlnaServer> {
    let locations = ssdp_search(timeout).await;
    let mut servers = Vec::new();
    for loc in locations {
        if let Ok(desc) = fetch_description(&loc).await {
            if desc.content_directory.is_some() {
                let name = if desc.friendly_name.is_empty() {
                    host_of(&loc).to_string()
                } else {
                    desc.friendly_name
                };
                servers.push(DlnaServer {
                    name,
                    location: loc,
                });
            }
        }
    }
    if servers.is_empty() {
        tracing::info!("[dlna] 未发现 DLNA 媒体服务器");
    } else {
        tracing::info!("[dlna] 发现 {} 个 DLNA 媒体服务器", servers.len());
    }
    servers
}

/// 浏览某服务器（设备描述 `location`）下全部音频条目（递归进容器）。
pub async fn browse(location: &str) -> Result<Vec<DlnaItem>, String> {
    let desc = fetch_description(location).await?;
    let ctrl = desc
        .content_directory
        .ok_or_else(|| "该设备没有 ContentDirectory 服务".to_string())?;
    let mut items = Vec::new();
    if let Err(e) = browse_container(&ctrl, "0", 0, &mut items).await {
        if items.is_empty() {
            return Err(e);
        }
        tracing::warn!("[dlna] 浏览中断（已收集 {} 项）：{e}", items.len());
    }
    Ok(items)
}

// ─────────────────────────────── SSDP 发现 ───────────────────────────────

/// 组播 M-SEARCH，收集响应里的 `LOCATION`（设备描述地址），去重返回。
async fn ssdp_search(timeout: Duration) -> Vec<String> {
    let socket = match tokio::net::UdpSocket::bind("0.0.0.0:0").await {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!("[dlna] SSDP 套接字创建失败（无法发现）：{e}");
            return Vec::new();
        }
    };
    // 只问 MediaServer，减少无关响应。
    let msearch = "M-SEARCH * HTTP/1.1\r\n\
        HOST: 239.255.255.250:1900\r\n\
        MAN: \"ssdp:discover\"\r\n\
        MX: 2\r\n\
        ST: urn:schemas-upnp-org:device:MediaServer:1\r\n\
        \r\n";
    if socket
        .send_to(msearch.as_bytes(), "239.255.255.250:1900")
        .await
        .is_err()
    {
        return Vec::new();
    }
    let mut locations: Vec<String> = Vec::new();
    let mut buf = [0u8; 8192];
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            break;
        }
        match tokio::time::timeout(remaining, socket.recv_from(&mut buf)).await {
            Ok(Ok((n, _addr))) => {
                let text = String::from_utf8_lossy(&buf[..n]);
                if let Some(loc) = parse_ssdp_location(&text) {
                    if !locations.iter().any(|l| l == &loc) {
                        locations.push(loc);
                    }
                }
            }
            _ => break,
        }
    }
    locations
}

/// 从 SSDP 响应文本里取 `LOCATION:` 头（大小写不敏感）。
fn parse_ssdp_location(resp: &str) -> Option<String> {
    for line in resp.lines() {
        if let Some((k, v)) = line.split_once(':') {
            if k.trim().eq_ignore_ascii_case("LOCATION") {
                return Some(v.trim().to_string());
            }
        }
    }
    None
}

// ─────────────────────────────── 设备描述 ───────────────────────────────

struct DeviceDesc {
    friendly_name: String,
    content_directory: Option<String>,
}

/// 拉设备描述 XML，解析友好名 + ContentDirectory 控制 URL（相对地址按 location 补齐）。
async fn fetch_description(location: &str) -> Result<DeviceDesc, String> {
    let xml = http_get_text(location, Duration::from_secs(3)).await?;
    parse_device_description(&xml, location)
}

fn parse_device_description(xml: &str, base: &str) -> Result<DeviceDesc, String> {
    let events = scan_xml(xml);
    let mut friendly = String::new();
    let mut cd_ctrl: Option<String> = None;
    let mut in_service = false;
    let mut svc_type = String::new();
    let mut svc_ctrl = String::new();
    let mut capture: Option<String> = None;
    for ev in &events {
        match ev {
            XEv::Start(name, _attrs) => {
                let l = local(name);
                if l == "service" {
                    in_service = true;
                    svc_type.clear();
                    svc_ctrl.clear();
                }
                if matches!(l.as_str(), "friendlyname" | "servicetype" | "controlurl") {
                    capture = Some(l);
                }
            }
            XEv::Text(t) => {
                if let Some(cap) = &capture {
                    match cap.as_str() {
                        "friendlyname" => friendly = t.clone(),
                        "servicetype" => svc_type = t.clone(),
                        "controlurl" => svc_ctrl = t.clone(),
                        _ => {}
                    }
                }
            }
            XEv::End(name) => {
                let l = local(name);
                if l == "service" {
                    if in_service
                        && svc_type.to_lowercase().contains("contentdirectory")
                        && cd_ctrl.is_none()
                    {
                        cd_ctrl = Some(resolve_url(base, &svc_ctrl));
                    }
                    in_service = false;
                }
                capture = None;
            }
        }
    }
    Ok(DeviceDesc {
        friendly_name: friendly,
        content_directory: cd_ctrl,
    })
}

// ─────────────────────────────── ContentDirectory 浏览 ───────────────────────────────

/// 递归浏览某个容器（ObjectID），把音频条目追加到 `out`，容器则下钻。
async fn browse_container(
    ctrl: &str,
    object_id: &str,
    depth: u32,
    out: &mut Vec<DlnaItem>,
) -> Result<(), String> {
    if depth > 5 || out.len() >= 3000 {
        return Ok(());
    }
    let didl = soap_browse(ctrl, object_id).await?;
    for e in parse_didl(&didl) {
        if e.is_container {
            // 递归 async fn 必须显式 Box::pin 引入间接层，否则 future 尺寸无穷递归。
            Box::pin(browse_container(ctrl, &e.id, depth + 1, out)).await?;
        } else if !e.res_url.is_empty() && is_audio(&e.class) {
            out.push(DlnaItem {
                title: if e.title.is_empty() {
                    e.res_url.clone()
                } else {
                    e.title
                },
                artist: e.artist,
                album: e.album,
                url: resolve_url(ctrl, &e.res_url),
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

/// 发 SOAP `Browse` 到 ContentDirectory 控制 URL，返回 `Result` 里的 DIDL 文本。
async fn soap_browse(ctrl: &str, object_id: &str) -> Result<String, String> {
    let body = format!(
        r#"<?xml version="1.0" encoding="utf-8"?>
<s:Envelope xmlns:s="http://schemas.xmlsoap.org/soap/envelope/" s:encodingStyle="http://schemas.xmlsoap.org/soap/encoding/">
<s:Body>
<u:Browse xmlns:u="urn:schemas-upnp-org:service:ContentDirectory:1">
<ObjectID>{oid}</ObjectID>
<BrowseFlag>BrowseDirectChildren</BrowseFlag>
<Filter>*</Filter>
<StartingIndex>0</StartingIndex>
<RequestedCount>0</RequestedCount>
<SortCriteria></SortCriteria>
</u:Browse>
</s:Body>
</s:Envelope>"#,
        oid = object_id
    );
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| format!("SOAP 客户端创建失败: {e}"))?;
    let resp = client
        .post(ctrl)
        .header("Content-Type", "text/xml; charset=\"utf-8\"")
        .header(
            "SOAPACTION",
            "\"urn:schemas-upnp-org:service:ContentDirectory:1#Browse\"",
        )
        .body(body)
        .send()
        .await
        .map_err(|e| format!("Browse 请求失败: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("Browse HTTP {}", resp.status()));
    }
    let text = resp
        .text()
        .await
        .map_err(|e| format!("读取 Browse 响应失败: {e}"))?;
    parse_soap_result(&text).ok_or_else(|| "Browse 响应缺少 Result".to_string())
}

/// 从 SOAP 响应里抽取 `<Result>` 文本（已 XML 转义，scanner 会还原成 DIDL）。
fn parse_soap_result(xml: &str) -> Option<String> {
    let events = scan_xml(xml);
    let mut capture = false;
    let mut result = String::new();
    for ev in &events {
        match ev {
            XEv::Start(name, _attrs) => {
                if local(name) == "result" {
                    capture = true;
                }
            }
            XEv::Text(t) => {
                if capture {
                    result.push_str(t);
                }
            }
            XEv::End(name) => {
                if local(name) == "result" {
                    capture = false;
                }
            }
        }
    }
    if result.is_empty() {
        None
    } else {
        Some(result)
    }
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

/// 把相对控制 URL 解析成绝对地址（基于 base；`http(s):` 直链原样返回）。
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

// ─────────────────────────────── HTTP 工具 ───────────────────────────────

/// GET 文本（设备描述 / SOAP）。带超时。
pub(crate) async fn http_get_text(url: &str, timeout: Duration) -> Result<String, String> {
    let client = reqwest::Client::builder()
        .timeout(timeout)
        .build()
        .map_err(|e| format!("HTTP 客户端创建失败: {e}"))?;
    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("请求失败: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {} 拉取失败", resp.status()));
    }
    resp.text()
        .await
        .map_err(|e| format!("读取响应失败: {e}"))
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
// 只服务规整的 DLNA 设备描述 / SOAP / DIDL：按标签名（忽略命名空间前缀）+ 属性遍历，
// 不校验结构。属性值里含引号实体（如 `&quot;`）的情况极罕见，此处不做实体还原。

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
                let name = String::from_utf8_lossy(&bytes[start..k]).trim().to_ascii_lowercase();
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
                let name = String::from_utf8_lossy(&bytes[start..k]).trim().to_ascii_lowercase();
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
                    let attr_name =
                        String::from_utf8_lossy(&bytes[an..p]).trim().to_ascii_lowercase();
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
            let text = unescape_str(&String::from_utf8_lossy(&bytes[start..i])).trim().to_string();
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
                        if let Some(code) = ent.strip_prefix('#').and_then(|c| c.parse::<u32>().ok())
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
