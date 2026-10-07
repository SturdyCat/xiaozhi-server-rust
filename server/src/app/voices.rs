//! 发音人目录（服务端唯一数据源）：`GET /api/tts/voices` 供客户端按 gender/type 分组建下拉。
//!
//! ## 数据来源（仅接口，无硬编码）
//! **AIUI 平台接口动态拉取**：`aiui.xfyun.cn` 控制台 informants 接口，
//! 按 `auth=true` 过滤出**当前账号已授权**的发音人（ttsType=2 普通 / 4 极速拟人
//! ——这两类可在经典 v2/tts 合成接口使用；x4 oral 系仅 AIUI 链路，不收）。
//! 凭据取 `[tts.xfyun].console_cookie` + `console_csrf`（浏览器会话，会过期）。
//! 结果落盘缓存（`XIAOZHI_VOICES_CACHE`，默认 /data/voices.json），重启即用——
//! 缓存是**拉取结果的持久化**，不是回退硬编码；未配置凭据且无缓存时目录为空。
//!
//! ## 接口（全部在 /api 之下）
//! - `GET /api/tts/voices`：当前目录（含 source/fetched_at 元信息）。
//! - `POST /api/tts/voices/refresh`：立即从平台拉取（admin 动作，expected_token 非空需鉴权）。
//!   服务端启动时若配置了 Cookie 也会后台自动刷新一次。

use std::sync::{Arc, OnceLock, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::{
    extract::{Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Router,
};
use serde::{Deserialize, Serialize};

use crate::engine::Engines;
use crate::app::ws::auth_ok;

/// 目录条目（客户端按 gender 分组、type 过滤）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Voice {
    pub vcn: String,
    pub name: String,
    /// 性别主分组：female（女）/ male（男）。
    pub gender: String,
    /// 音色类型：classic（普通发音人）/ x6（极速拟人，x5/x6 全系）。
    #[serde(rename = "type")]
    pub type_: String,
    /// 附加描述（场景 · 音色 等；空串则只显示名字）。
    pub tag: String,
}

/// 目录缓存快照（落盘格式）。
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Snapshot {
    /// 来源标识："console"（平台接口拉取）/ "empty"（尚未拉取，空目录）。
    source: String,
    /// 拉取时刻（Unix 秒；0 = 尚未拉取）。
    fetched_at: u64,
    voices: Vec<Voice>,
}

static STATE: OnceLock<RwLock<Snapshot>> = OnceLock::new();

fn state() -> &'static RwLock<Snapshot> {
    // 空目录起步：音色列表**只来自接口拉取**（无硬编码；缓存/内存任一有值即可用）
    STATE.get_or_init(|| RwLock::new(Snapshot { source: "empty".into(), fetched_at: 0, voices: Vec::new() }))
}

/// 缓存文件路径（`XIAOZHI_VOICES_CACHE`，默认 /data/voices.json——随唯一数据卷持久化）。
fn cache_path() -> String {
    std::env::var("XIAOZHI_VOICES_CACHE").unwrap_or_else(|_| "/data/voices.json".to_string())
}

/// 启动初始化：优先读盘缓存；无缓存保持空目录（等待接口拉取，不阻塞、不联网）。
pub fn init() {
    let path = cache_path();
    let loaded = std::fs::read_to_string(&path)
        .ok()
        .and_then(|t| serde_json::from_str::<Snapshot>(&t).ok())
        .filter(|s| !s.voices.is_empty());
    match loaded {
        Some(snap) => {
            tracing::info!(
                "发音人目录：已加载缓存 {}（{} 条，source={}）",
                path,
                snap.voices.len(),
                snap.source
            );
            *state().write().unwrap_or_else(|e| e.into_inner()) = snap;
        }
        None => {
            tracing::info!(
                "发音人目录：无缓存——等待配置控制台会话后自动/手动拉取（未配置时列表为空，前端提示先填凭据）"
            );
        }
    }
}

/// 启动后台刷新：仅当配置了控制台 Cookie 时触发（失败仅告警，保持现有目录）。
pub fn spawn_startup_refresh(engines: Arc<Engines>) {
    let (cookie, csrf) = console_creds(&engines);
    if cookie.is_empty() || csrf.is_empty() {
        return; // 未配置控制台会话：跳过（目录仍可用，走缓存/离线目录）
    }
    tokio::spawn(async move {
        match fetch_console(&cookie, &csrf, &engines.config.tts.xfyun.app_id).await {
            Ok(voices) => {
                let saved = persist(&voices, "console");
                tracing::info!(
                    "发音人目录：启动刷新成功（{} 条，缓存{}）",
                    voices.len(),
                    if saved { "已写盘" } else { "写盘失败" }
                );
            }
            Err(e) => tracing::warn!("发音人目录：启动刷新失败（保持现有目录）: {e:#}"),
        }
    });
}

fn console_creds(engines: &Engines) -> (String, String) {
    let x = &engines.config.tts.xfyun;
    (x.console_cookie.trim().to_string(), x.console_csrf.trim().to_string())
}

/// 落盘缓存并更新内存状态；返回是否写盘成功（写盘失败不影响内存生效）。
fn persist(voices: &[Voice], source: &str) -> bool {
    let snap = Snapshot {
        source: source.to_string(),
        fetched_at: now_secs(),
        voices: voices.to_vec(),
    };
    let ok = std::fs::write(
        cache_path(),
        serde_json::to_string_pretty(&snap).unwrap_or_default(),
    )
    .is_ok();
    *state().write().unwrap_or_else(|e| e.into_inner()) = snap;
    ok
}

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// 从 AIUI 控制台接口拉取已授权发音人（ttsType=2 普通 / 4 极速拟人）。
///
/// 鉴权（实测缺一不可）：`Cookie` 整串 + `X-Csrf-Token` 头 + `Referer: …/app/{appid}/config`
/// + 新鲜毫秒级 `ts` 查询参数（缺/过期 401）。响应 `data.informants[]`，
/// 过滤 `auth==true && status==1`。
async fn fetch_console(cookie: &str, csrf: &str, appid: &str) -> anyhow::Result<Vec<Voice>> {
    let client = reqwest::Client::builder()
        .user_agent("Mozilla/5.0")
        .timeout(Duration::from_secs(20))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new());
    let referer = format!("https://aiui.xfyun.cn/app/{appid}/config");
    let mut out: Vec<Voice> = Vec::new();
    for (tt, ty) in [(2u32, "classic"), (4u32, "x6")] {
        let ts = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
        let url = format!(
            "https://aiui.xfyun.cn/aiui/web/app/tts/informants?ttsType={tt}&search=&appid={appid}&sceneType=sparkos&ts={ts}"
        );
        let resp = client
            .get(&url)
            .header("X-Csrf-Token", csrf)
            .header("Referer", &referer)
            .header("Accept", "application/json, text/plain, */*")
            .header("Cookie", cookie)
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("请求 informants(ttsType={tt}) 失败: {e}"))?;
        let status = resp.status();
        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| anyhow::anyhow!("informants(ttsType={tt}) 响应解析失败（HTTP {status}）: {e}"))?;
        if status != 200 || body.get("code").and_then(|c| c.as_str()) != Some("0") {
            anyhow::bail!(
                "informants(ttsType={tt}) 被拒（HTTP {status}，{}）——控制台会话过期？请更新 console_cookie/console_csrf",
                body.get("desc").and_then(|d| d.as_str()).unwrap_or("无描述")
            );
        }
        let infos = body
            .pointer("/data/informants")
            .and_then(|i| i.as_array())
            .cloned()
            .unwrap_or_default();
        for i in infos {
            let auth = i.get("auth").and_then(|a| a.as_bool()).unwrap_or(false);
            let st = i.get("status").and_then(|s| s.as_i64()).unwrap_or(1);
            if !auth || st != 1 {
                continue; // 未授权/已下架：不进目录（正是「选了没效果」的根因）
            }
            let Some(vcn) = i.get("vcn").and_then(|v| v.as_str()) else { continue };
            let name = i.get("name").and_then(|n| n.as_str()).unwrap_or(vcn);
            // 性别：age（成年女声/女童…）优先，timbre 兜底
            let age = i.get("age").and_then(|a| a.as_str()).unwrap_or("");
            let timbre = i.get("timbre").and_then(|t| t.as_str()).unwrap_or("");
            let gender = if age.contains('女') || timbre.contains('女') {
                "female"
            } else if age.contains('男') || timbre.contains('男') {
                "male"
            } else {
                "female" // 缺字段兜底（信息缺失好过丢条目）
            };
            // 描述：场景缩写 · 音色；名字已含"男声/女声"时只留场景
            let scene = match i.get("scene").and_then(|s| s.as_str()).unwrap_or("") {
                "交互场景" => "交互",
                "旁白配音" => "旁白",
                other => other,
            };
            let tag = if name.ends_with("男声") || name.ends_with("女声") {
                scene.to_string()
            } else if scene.is_empty() {
                timbre.to_string()
            } else if timbre.is_empty() {
                scene.to_string()
            } else {
                format!("{scene} · {timbre}")
            };
            // 去重：同 vcn 以先到为准（ttsType=2 先于 4）
            if out.iter().any(|v| v.vcn == vcn) {
                continue;
            }
            out.push(Voice {
                vcn: vcn.to_string(),
                name: name.to_string(),
                gender: gender.to_string(),
                type_: ty.to_string(),
                tag,
            });
        }
    }
    if out.is_empty() {
        anyhow::bail!("informants 返回空目录（账号无已授权发音人？）");
    }
    Ok(out)
}

/// GET /api/tts/voices：当前目录 + 元信息。
async fn voices() -> Response {
    let snap = state().read().unwrap_or_else(|e| e.into_inner()).clone();
    (
        [(header::CONTENT_TYPE, "application/json; charset=utf-8")],
        serde_json::json!({
            "voices": snap.voices,
            "source": snap.source,
            "fetched_at": snap.fetched_at,
        })
        .to_string(),
    )
        .into_response()
}

/// POST /api/tts/voices/refresh：立即从平台拉取目录（admin 动作）。
async fn refresh(
    State(engines): State<Arc<Engines>>,
    headers: HeaderMap,
    Query(query): Query<std::collections::HashMap<String, String>>,
) -> Response {
    if !auth_ok(&headers, &query, &engines.config.server) {
        return (StatusCode::UNAUTHORIZED, "unauthorized").into_response();
    }
    let (cookie, csrf) = console_creds(&engines);
    if cookie.is_empty() || csrf.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            "未配置 [tts.xfyun].console_cookie / console_csrf——请在 TTS 配置卡填写控制台会话后重试",
        )
            .into_response();
    }
    match fetch_console(&cookie, &csrf, &engines.config.tts.xfyun.app_id).await {
        Ok(v) => {
            let saved = persist(&v, "console");
            tracing::info!(
                "发音人目录：手动刷新成功（{} 条，缓存{}）",
                v.len(),
                if saved { "已写盘" } else { "写盘失败" }
            );
            (
                [(header::CONTENT_TYPE, "application/json; charset=utf-8")],
                serde_json::json!({ "ok": true, "count": v.len(), "source": "console" }).to_string(),
            )
                .into_response()
        }
        Err(e) => {
            tracing::warn!("发音人目录：手动刷新失败: {e:#}");
            (
                StatusCode::BAD_GATEWAY,
                format!("刷新失败（保持现有目录）: {e:#}"),
            )
                .into_response()
        }
    }
}

/// POST /api/tts/voices/test-credentials：用传入的三要素真实测试一次讯飞合成
/// （管理页「测试凭据」按钮；**只测不存**，不读写服务端配置）。
///
/// 请求体：`{"app_id":"…","api_key":"…","api_secret":"…","voice":"…"}`（voice 可选，默认 xiaoyan）。
/// 响应：`{"ok":true,"elapsed_ms":123}` 或 `{"ok":false,"error":"…（含 11200 等可行动提示）"}`。
async fn test_credentials(
    State(engines): State<Arc<Engines>>,
    headers: HeaderMap,
    Query(query): Query<std::collections::HashMap<String, String>>,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> Response {
    if !auth_ok(&headers, &query, &engines.config.server) {
        return (StatusCode::UNAUTHORIZED, "unauthorized").into_response();
    }
    let cfg = crate::plugins::tts::XfyunTtsConfig {
        app_id: body.get("app_id").and_then(|v| v.as_str()).unwrap_or("").trim().to_string(),
        api_key: body.get("api_key").and_then(|v| v.as_str()).unwrap_or("").trim().to_string(),
        api_secret: body.get("api_secret").and_then(|v| v.as_str()).unwrap_or("").trim().to_string(),
        voice: body
            .get("voice")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .unwrap_or("xiaoyan")
            .to_string(),
        ..Default::default()
    };
    // 真连讯飞是阻塞调用（HTTPS 握手 + 合成 ~200ms+），放 spawn_blocking 隔离
    let result = tokio::task::spawn_blocking(move || crate::plugins::tts::xfyun::test_credentials(&cfg))
        .await
        .unwrap_or_else(|e| Err(anyhow::anyhow!("测试任务异常: {e}")));
    match result {
        Ok(ms) => (
            [(header::CONTENT_TYPE, "application/json; charset=utf-8")],
            serde_json::json!({ "ok": true, "elapsed_ms": ms }).to_string(),
        )
            .into_response(),
        Err(e) => {
            tracing::warn!("凭据测试失败: {e:#}");
            (
                [(header::CONTENT_TYPE, "application/json; charset=utf-8")],
                serde_json::json!({ "ok": false, "error": format!("{e:#}") }).to_string(),
            )
                .into_response()
        }
    }
}

/// 发音人目录子路由（并入主 router）。
pub fn router() -> Router<Arc<Engines>> {
    Router::new()
        .route("/api/tts/voices", get(voices))
        .route("/api/tts/voices/refresh", post(refresh))
        .route("/api/tts/voices/test-credentials", post(test_credentials))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 无硬编码契约：内存态初始为空目录（仅接口/缓存填充）。
    #[test]
    fn state_starts_empty_without_cache() {
        // 不设 XIAOZHI_VOICES_CACHE 指向不存在的文件时，init 后目录应保持空
        std::env::set_var("XIAOZHI_VOICES_CACHE", "/nonexistent/voices-test.json");
        init();
        let snap = state().read().unwrap_or_else(|e| e.into_inner()).clone();
        assert!(snap.voices.is_empty(), "无缓存时应为空目录（不得回退硬编码）");
        assert_eq!(snap.source, "empty");
        std::env::remove_var("XIAOZHI_VOICES_CACHE");
    }

    /// Voice 序列化：`type` 字段名（客户端按此解析）。
    #[test]
    fn voice_serializes_type_field() {
        let v = Voice {
            vcn: "xiaoyan".into(),
            name: "小燕".into(),
            gender: "female".into(),
            type_: "classic".into(),
            tag: "标准女声".into(),
        };
        let j = serde_json::to_value(&v).unwrap();
        assert_eq!(j["vcn"], "xiaoyan");
        assert_eq!(j["gender"], "female");
        assert_eq!(j["type"], "classic");
    }

    /// 真实连平台接口的拉取冒烟（**排障工具，日常不跑**）：Cookie/CSRF 经环境变量传入——
    /// `XF_CK='…' XF_CSRF='…' XF_APPID=488ee08e cargo test voices_live -- --ignored --nocapture`
    #[test]
    #[ignore = "live 平台调用：需 XF_CK/XF_CSRF/XF_APPID 环境变量"]
    fn voices_live_fetch() {
        let (Ok(ck), Ok(csrf), Ok(appid)) =
            (std::env::var("XF_CK"), std::env::var("XF_CSRF"), std::env::var("XF_APPID"))
        else {
            eprintln!("跳过：未设置 XF_CK/XF_CSRF/XF_APPID");
            return;
        };
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        match rt.block_on(fetch_console(&ck, &csrf, &appid)) {
            Ok(v) => {
                let f = v.iter().filter(|x| x.gender == "female").count();
                println!(
                    "== OK {} 条（女 {} / 男 {}），示例: {:?}",
                    v.len(),
                    f,
                    v.len() - f,
                    v.first().map(|x| (&x.vcn, &x.name, &x.type_, x.gender.as_str()))
                );
            }
            Err(e) => println!("== ERR {e:#}"),
        }
    }
}
