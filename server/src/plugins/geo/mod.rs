//! 服务器出口 IP 默认定位。
//!
//! 进程启动时解析服务器出口 IP，仅当为**国内** IP（来自 `myip.ipip.net`，非国内不写入）
//! 才写入默认地区，供天气等需要定位的功能使用。国外出口（翻墙/代理）不写入，避免误用
//! 境外定位——且**不触发备用源兜底**（备用源是国内 IP 库，查国外 IP 无意义甚至误报），
//! 仅当主源**请求/解析失败**时才尝试备用源。任何错误都 `tracing::warn!` 后返回，绝不
//! 阻塞启动、绝不 panic。
//!
//! 供管理台展示的两份只读快照（[last_resolved] / [resolve_note]）：最近一次**成功解析**
//! 的出口信息（含境外）与未成功定位的原因。
//!
//! 网络使用 async `reqwest`（5s 超时），由调用方在 tokio 运行时内 `tokio::spawn`。

#[cfg(test)]
mod tests;

use serde::Serialize;
use serde_json::Value;
use std::sync::{Arc, OnceLock, RwLock};

/// 服务器出口 IP 定位结果（省市级）。
#[derive(Debug, Clone, Serialize)]
pub struct ServerLocation {
    pub ip: String,
    pub country: String,
    pub province: String,
    pub city: String,
    /// 数据来源（`myip.ipip.net` / `whois.pconline.com.cn`）。
    pub source: String,
    /// 是否为**国内**出口：只有 `true` 才写入默认地区（[current]），境外出口仅供管理台展示。
    pub domestic: bool,
}

static LOCATION: OnceLock<Arc<RwLock<Option<ServerLocation>>>> = OnceLock::new();
/// 最近一次**成功解析**的出口信息（含境外）——供管理台展示；与 [LOCATION] 分开，
/// 因为 `LOCATION` 只收国内（语义是"默认定位"），境外出口也该让用户看到。
static LAST_LOCATION: OnceLock<Arc<RwLock<Option<ServerLocation>>>> = OnceLock::new();
/// 未成功定位的原因（空串 = 尚未解析/解析中）；供管理台解释"为什么没有默认地区"。
static RESOLVE_NOTE: OnceLock<Arc<RwLock<String>>> = OnceLock::new();

fn shared<T: Default>() -> Arc<RwLock<T>> {
    Arc::new(RwLock::new(T::default()))
}

/// 设置进程级默认地区锁（重复 set 静默忽略）。
pub fn set_location_global(a: Arc<RwLock<Option<ServerLocation>>>) {
    let _ = LOCATION.set(a);
}

/// 读取当前默认定位（读锁取克隆；未初始化/锁 Poison 返回 None）。
///
/// 只返回**国内**出口的结果——境外出口虽会记录到 [last_resolved] 供展示，
/// 但绝不用作默认定位。
pub fn current() -> Option<ServerLocation> {
    LOCATION
        .get()
        .and_then(|a| a.read().ok().and_then(|g| g.clone()))
        .filter(|loc| loc.domestic)
}

/// 最近一次成功解析的出口信息（含境外；管理台展示用）。
pub fn last_resolved() -> Option<ServerLocation> {
    LAST_LOCATION
        .get_or_init(|| shared())
        .read()
        .ok()
        .and_then(|g| g.clone())
}

/// 未成功定位的原因（空串 = 尚未解析/解析中）。
pub fn resolve_note() -> String {
    RESOLVE_NOTE
        .get_or_init(|| shared())
        .read()
        .map(|n| n.clone())
        .unwrap_or_default()
}

fn set_note(note: &str) {
    if let Ok(mut w) = RESOLVE_NOTE.get_or_init(|| shared()).write() {
        *w = note.to_string();
    }
}

/// `myip.ipip.net` 一次请求的结果分类：
/// - `Located`：国内且城市非空，可直接写入默认地区；
/// - `Foreign`：解析成功但出口为国外——**停止**，不写默认定位也不兜底（记录供展示）；
/// - `Fallback`：国内但城市为空，或请求/解析失败——可尝试备用源。
enum IpipOutcome {
    Located(ServerLocation),
    Foreign(ServerLocation),
    Fallback(&'static str),
}

/// 启动期解析出口 IP 并写入默认定位。
///
/// 先试 `myip.ipip.net`（纯文本），仅当**请求/解析失败或城市为空**才再试
/// `whois.pconline.com.cn`（JSON，国内 IP 库）。国外出口直接放弃。
pub async fn resolve_location() {
    match fetch_ipip().await {
        IpipOutcome::Located(loc) => {
            write_location(loc);
            return;
        }
        // 以国内 IP 为准：国外出口（翻墙/代理）不写默认定位，也不查国内 IP 库兜底；
        // 但仍记录最近一次解析结果，供管理台展示出口信息。
        IpipOutcome::Foreign(loc) => {
            tracing::warn!("出口IP为国外地址（可能经代理/翻墙），不写入默认地区");
            write_last(loc);
            set_note("出口IP为国外地址（可能经代理/翻墙），未启用默认定位");
            return;
        }
        IpipOutcome::Fallback(reason) => {
            tracing::warn!("出口IP定位（myip.ipip.net）{reason}，尝试备用源");
        }
    }

    match fetch_pconline().await {
        Some(loc) if !loc.city.is_empty() => {
            write_location(loc);
        }
        Some(_) => {
            tracing::warn!("出口IP定位（whois.pconline.com.cn）省份有效但城市为空，不写入默认地区");
            set_note("出口IP定位结果缺少城市，未启用默认定位");
        }
        None => {
            tracing::warn!("备用出口IP定位（whois.pconline.com.cn）失败，不写入默认地区");
            set_note("出口IP定位失败（上游服务不可达），未启用默认定位");
        }
    }
}

/// 写入默认定位（仅国内）+ 最近解析快照（含境外），并清除未定位原因。
fn write_location(loc: ServerLocation) {
    if loc.domestic {
        if let Some(g) = LOCATION.get() {
            if let Ok(mut w) = g.write() {
                *w = Some(loc.clone());
            }
        }
    }
    write_last(loc);
    set_note("");
}

fn write_last(loc: ServerLocation) {
    if let Ok(mut w) = LAST_LOCATION.get_or_init(|| shared()).write() {
        *w = Some(loc);
    }
}

async fn fetch_ipip() -> IpipOutcome {
    let text = match fetch_text("https://myip.ipip.net").await {
        Ok(t) => t,
        Err(e) => {
            tracing::warn!("出口IP定位请求（myip.ipip.net）失败: {e:#}");
            return IpipOutcome::Fallback("请求失败");
        }
    };
    match parse_ipip_parts(&text) {
        Some((ip, country, province, city)) => {
            let domestic = country.contains("中国");
            if !domestic {
                return IpipOutcome::Foreign(ServerLocation {
                    ip,
                    country,
                    province,
                    city,
                    source: "myip.ipip.net".into(),
                    domestic: false,
                });
            }
            if city.is_empty() {
                return IpipOutcome::Fallback("解析到国内但城市为空");
            }
            IpipOutcome::Located(ServerLocation {
                ip,
                country,
                province,
                city,
                source: "myip.ipip.net".into(),
                domestic: true,
            })
        }
        // 返回内容不是预期格式（接口变更/被劫持页）：按失败兜底。
        None => IpipOutcome::Fallback("返回格式无法解析"),
    }
}

async fn fetch_pconline() -> Option<ServerLocation> {
    let text = fetch_text("https://whois.pconline.com.cn/ipJson.jsp?json=true")
        .await
        .ok()?;
    parse_pconline(&text)
}

async fn fetch_text(url: &str) -> anyhow::Result<String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .user_agent(concat!("xiaozhi-server-rust/", env!("CARGO_PKG_VERSION")))
        .build()?;
    Ok(client.get(url).send().await?.text().await?)
}

/// 纯函数：解析 `myip.ipip.net` 文本行的结构部分。
///
/// 形如 `当前 IP：1.2.3.4 来自于：中国 广东 深圳 电信`。
/// 返回 `(ip, country, province, city)`；格式不符返回 `None`。
fn parse_ipip_parts(text: &str) -> Option<(String, String, String, String)> {
    let trimmed = text.trim();
    let (before, after) = trimmed
        .split_once("来自于：")
        .or_else(|| trimmed.split_once("来自于:"))?;
    // IP 跟在「IP：」标记之后（兼容全角/半角冒号、「当前 」前缀有无）。
    // 不能取「来自于」前的最后一个空白 token——「IP：1.2.3.4」是一个 token，
    // 冒号在中间，按边裁剪修不掉。
    let ip = before
        .split_once("IP：")
        .or_else(|| before.split_once("IP:"))
        .map(|(_, rest)| rest.trim().to_string())
        .or_else(|| {
            before
                .split_whitespace()
                .last()
                .map(|s| s.trim_matches(|c| c == '：' || c == ':').to_string())
        })
        .unwrap_or_default();
    if ip.is_empty() {
        return None;
    }
    let parts: Vec<&str> = after.split_whitespace().collect();
    let country = parts.first().map(|s| s.to_string())?;
    let province = parts.get(1).map(|s| s.to_string()).unwrap_or_default();
    let city = parts.get(2).map(|s| s.to_string()).unwrap_or_default();
    Some((ip, country, province, city))
}

/// 纯函数：解析 `myip.ipip.net` 文本行。
///
/// 仅当解析成功且 country 含「中国」时返回 `Some`（国外或格式不符返回 `None`）。
pub fn parse_ipip(text: &str) -> Option<ServerLocation> {
    parse_ipip_parts(text)
        .filter(|(_, country, _, _)| country.contains("中国"))
        .map(|(ip, country, province, city)| ServerLocation {
            ip,
            country,
            province,
            city,
            source: "myip.ipip.net".into(),
            domestic: true,
        })
}

/// 解析 `whois.pconline.com.cn/ipJson.jsp?json=true` 返回 JSON `{pro, city, ...}`。
/// 该服务为国内 IP 库，仅作主源失败后的兜底（调用方已保证此时不区分国内外）；
/// 解析失败或省份为空返回 `None`。
fn parse_pconline(text: &str) -> Option<ServerLocation> {
    let v: Value = serde_json::from_str(text).ok()?;
    let pro = v.get("pro").and_then(Value::as_str).unwrap_or("").to_string();
    let city = v
        .get("city")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    if pro.is_empty() {
        return None;
    }
    // pconline 默认 GBK；若经 UTF-8 解码出现乱码（含替换符）则说明编码不对，不写入。
    if pro.contains('\u{FFFD}') || city.contains('\u{FFFD}') {
        return None;
    }
    Some(ServerLocation {
        ip: String::new(),
        country: "中国".into(),
        province: pro,
        city,
        source: "whois.pconline.com.cn".into(),
        domestic: true,
    })
}
