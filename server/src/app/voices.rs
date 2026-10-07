//! 发音人目录（服务端唯一数据源）：`GET /api/tts/voices` 供客户端按 gender/type 分组建下拉。
//!
//! ## 数据来源（优先级）
//! 1. **AIUI 平台接口动态拉取**（首选）：`aiui.xfyun.cn` 控制台 informants 接口，
//!    按 `auth=true` 过滤出**当前账号已授权**的发音人（ttsType=2 普通 / 4 极速拟人
//!    ——这两类可在经典 v2/tts 合成接口使用；x4 oral 系仅 AIUI 链路，不收）。
//!    凭据取 `[tts.xfyun].console_cookie` + `console_csrf`（浏览器会话，会过期）。
//!    结果落盘缓存（`XIAOZHI_VOICES_CACHE`，默认 /data/voices.json），重启即用。
//! 2. **内置离线目录**（兜底）：无缓存且接口不可达时使用（首次部署/内网环境）。
//!    目录为 2026-10-07 逐个真实合成**实测可调用**的音色。
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
    /// 来源标识："console"（平台接口）/ "seed"（内置离线目录）。
    source: String,
    /// 拉取时刻（Unix 秒；seed 为 0）。
    fetched_at: u64,
    voices: Vec<Voice>,
}

static STATE: OnceLock<RwLock<Snapshot>> = OnceLock::new();

fn state() -> &'static RwLock<Snapshot> {
    STATE.get_or_init(|| RwLock::new(Snapshot { source: "seed".into(), fetched_at: 0, voices: seed() }))
}

/// 缓存文件路径（`XIAOZHI_VOICES_CACHE`，默认 /data/voices.json——随唯一数据卷持久化）。
fn cache_path() -> String {
    std::env::var("XIAOZHI_VOICES_CACHE").unwrap_or_else(|_| "/data/voices.json".to_string())
}

/// 启动初始化：优先读盘缓存，无缓存用内置离线目录（不阻塞、不联网）。
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
            tracing::info!("发音人目录：无缓存，使用内置离线目录（{} 条）", seed().len());
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

/// 内置离线目录（**兜底**，非首选来源）：2026-10-07 逐个真实合成实测可调用。
/// 接口可达时以平台数据为准（含账号实际授权集）；离线/未配置会话时才用此表。
fn seed() -> Vec<Voice> {
    let rows: &[(&str, &str, &str, &str, &str)] = &[
        // (vcn, name, gender, type, tag)
        ("xiaoyan", "小燕", "female", "classic", "标准女声，默认"),
        ("xiaoqi", "小琪", "female", "classic", "经典女声"),
        ("aisxping", "小萍", "female", "classic", "女声"),
        ("aisjinger", "小婧", "female", "classic", "女声"),
        ("vixy", "vixy", "female", "classic", "女声·中英"),
        ("vimeiyu", "vimeiyu", "female", "classic", "女声"),
        ("vixying", "vixying", "female", "classic", "女声"),
        ("vixx", "vixx", "female", "classic", "女声"),
        ("catherine", "catherine", "female", "classic", "英文女声，中文不出声"),
        ("mary", "mary", "female", "classic", "英文女声，中文不出声"),
        ("xiaoyu", "小宇", "male", "classic", "男声"),
        ("xiaofeng", "小峰", "male", "classic", "男声"),
        ("aisjiuxu", "久许", "male", "classic", "男声"),
        ("vinn", "vinn", "male", "classic", "男声"),
        ("x6_dongmanshaonv_pro", "动漫少女", "female", "x6", "交互 · 情感女声"),
        ("x6_lingxiaoyue_pro", "聆小玥", "female", "x6", "交互 · 情感女声"),
        ("x6_lingyuyan_pro", "聆玉言", "female", "x6", "交互 · 情感女声"),
        ("x5_lingxiaotang_flow", "聆小糖", "female", "x6", "交互 · 情感女声"),
        ("x6_lingxiaoxuan_pro", "聆小璇", "female", "x6", "交互 · 情感女声"),
        ("x5_lingyuzhao_flow", "聆玉昭", "female", "x6", "交互 · 情感女声"),
        ("x6_lingxiaoying_pro", "聆小颖", "female", "x6", "交互 · 情感女声"),
        ("x6_lingxiaozhen_pro", "聆小瑱", "female", "x6", "交互 · 情感女声"),
        ("x6_ganliannvxing_pro", "干练女性", "female", "x6", "交互 · 情感女声"),
        ("x6_lingyufei_pro", "聆玉菲", "female", "x6", "交互 · 情感女声"),
        ("x6_pangbainv1_pro", "旁白女声", "female", "x6", "旁白"),
        ("x6_lingxiaoyun_pro", "聆小芸", "female", "x6", "交互 · 情感女声"),
        ("x6_lingyuaner_pro", "聆园儿", "female", "x6", "交互 · 情感女声"),
        ("x6_lingxiaoshan_pro", "聆小珊", "female", "x6", "交互 · 情感女声"),
        ("x6_lingxiaoli_pro", "聆小璃", "female", "x6", "交互 · 情感女声"),
        ("x6_xiaoqiChat_pro", "聆小琪", "female", "x6", "交互 · 情感女声"),
        ("x6_cuishounvsheng_pro", "催收女声", "female", "x6", "交互 · 成年女声"),
        ("x6_yingxiaonv_pro", "营销女声", "female", "x6", "交互 · 成年女声"),
        ("x6_shibingnvsheng_mini", "士兵女声", "female", "x6", "交互 · 成年女声"),
        ("x6_kongbunvsheng_mini", "恐怖女声", "female", "x6", "交互 · 成年女声"),
        ("x6_yulexinwennvsheng_mini", "娱乐新闻女声", "female", "x6", "交互 · 成年女声"),
        ("x6_jingqudaolannvsheng_mini", "景区导览女声", "female", "x6", "交互 · 成年女声"),
        ("x6_wumeinv_pro", "妩媚姐姐", "female", "x6", "交互 · 成年女声"),
        ("x6_huajidama_pro", "滑稽大妈", "female", "x6", "交互 · 成年女声"),
        ("x6_lingxiaoxue_pro", "聆小雪", "female", "x6", "交互 · 成年女声"),
        ("x6_gufengxianv_mini", "古风侠女", "female", "x6", "交互 · 成年女声"),
        ("x6_wuyediantai_mini", "午夜电台", "female", "x6", "交互 · 成年女声"),
        ("x6_zhuanyenvzhuchi_pro", "大会主持女声", "female", "x6", "交互 · 成年女声"),
        ("x6_ranzhinvdazi_pro", "运动陪练女声", "female", "x6", "交互 · 成年女声"),
        ("x6_zhantingnvjiedai_pro", "展厅接待女声", "female", "x6", "交互 · 成年女声"),
        ("x6_huifangnv_pro", "回访女声", "female", "x6", "交互 · 成年女声"),
        ("x6_dudulibao_pro", "少女可莉", "female", "x6", "女声"),
        ("x6_lingyouyou_pro", "聆佑佑", "female", "x6", "女童"),
        ("x6_lingfeiyi_pro", "聆飞逸", "male", "x6", "交互 · 成熟男声"),
        ("x6_lingfeibo_pro", "聆飞博", "male", "x6", "交互 · 成熟男声"),
        ("x6_gaolengnanshen_pro", "高冷男神", "male", "x6", "交互 · 成熟男声"),
        ("x6_waiguodashu_pro", "外国人大叔", "male", "x6", "交互 · 成熟男声"),
        ("x6_gufengpangbai_pro", "古风旁白", "male", "x6", "旁白 · 成熟男声"),
        ("x6_pangbainan1_pro", "旁白男声", "male", "x6", "旁白 · 成熟男声"),
        ("x6_lingfeihan_pro", "聆飞瀚", "male", "x6", "旁白 · 成熟男声"),
        ("x6_lingfeihao_pro", "聆飞皓", "male", "x6", "旁白 · 成熟男声"),
        ("x6_ruyadashu_pro", "儒雅大叔", "male", "x6", "旁白 · 成熟男声"),
        ("x6_feizheChat_pro", "聆飞哲", "male", "x6", "交互 · 成熟男声"),
        ("x6_wennuancixingnansheng_mini", "温暖磁性男声", "male", "x6", "交互 · 成年男声"),
        ("x6_xiaonaigoudidi_mini", "小奶狗弟弟", "male", "x6", "交互 · 成年男声"),
        ("x6_wenrounansheng_mini", "温柔男声", "male", "x6", "交互 · 成年男声"),
        ("x6_daqixuanchuanpiannansheng_mini", "大气宣传片男声", "male", "x6", "交互 · 成年男声"),
        ("x6_xiangruiyingyu_pro", "商务殷语", "male", "x6", "交互 · 成年男声"),
        ("x6_taiqiangnuannan_pro", "台湾腔温柔男声", "male", "x6", "交互 · 成年男声"),
        ("x6_lingbosong_pro", "聆伯松", "male", "x6", "交互 · 成年男声"),
        ("x6_huoposhaonian_pro", "活泼少年", "male", "x6", "交互 · 成年男声"),
        ("x6_tiexinnanyou_mini", "贴心男友", "male", "x6", "交互 · 成年男声"),
        ("x6_youxinanshibing_pro", "士兵男声", "male", "x6", "交互 · 成年男声"),
        ("x6_zuixianlibai_pro", "醉仙李白", "male", "x6", "交互 · 成年男声"),
        ("x6_bokenansheng_pro", "播客男声", "male", "x6", "交互 · 成年男声"),
        ("x6_zhuanyenanzhuchi_pro", "大会主持男声", "male", "x6", "交互 · 成年男声"),
        ("x6_zhantingnanjiedai_pro", "展厅接待男声", "male", "x6", "交互 · 成年男声"),
        ("x6_huanlemianbao_pro", "海绵宝宝", "male", "x6", "男声"),
        ("x6_shiwangxiaoxin_pro", "奶凶辛巴", "male", "x6", "男童"),
    ];
    rows.iter()
        .map(|(vcn, name, gender, ty, tag)| Voice {
            vcn: vcn.to_string(),
            name: name.to_string(),
            gender: gender.to_string(),
            type_: ty.to_string(),
            tag: tag.to_string(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 离线目录契约：vcn 唯一、gender/type 合法、总数与实测数一致。
    #[test]
    fn seed_is_unique_and_grouped() {
        let s = seed();
        let mut vcns = std::collections::HashSet::new();
        for v in &s {
            assert!(vcns.insert(v.vcn.clone()), "重复 vcn: {}", v.vcn);
            assert!(matches!(v.gender.as_str(), "female" | "male"), "非法性别: {}", v.vcn);
            assert!(matches!(v.type_.as_str(), "classic" | "x6"), "非法类型: {}", v.vcn);
        }
        assert_eq!(s.len(), 73);
    }

    /// 目录序列化：`type` 字段名（客户端按此解析）。
    #[test]
    fn voice_serializes_type_field() {
        let j = serde_json::to_value(&seed()[0]).unwrap();
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
