//! 发音人目录（服务端唯一数据源）：`GET /api/tts/voices` 供客户端按 gender/type 分组建下拉。
//!
//! ## 数据来源：官方 API 真实探测（无硬编码"可用列表"）
//! 讯飞没有"列举发音人"的官方 API（目录仅存在于文档页/控制台 UI），故服务端采用
//! **探测式目录**：对候选池（官方发布目录的发音人命名空间，见 [`CANDIDATES`]）逐个调用
//! AIUI 主动合成 API（[`crate::plugins::tts::aiui`]）真实合成一句短文本，
//! **只有探测通过（当前账号/应用确实可用）的音色才会对外可见**。
//! - 探测与合成引擎同源 → 目录即"实际可用的音色集合"，不会出现"列表里有、合成报错"；
//! - 结果落盘缓存（`XIAOZHI_VOICES_CACHE`，默认 /data/voices.json）——缓存是**探测结果的
//!   持久化**，不是可用列表的回退；未探测过时目录为空（前端提示先填三要素/点刷新）；
//! - 探测在后台任务执行（4 并发），进度经 `probing` 字段暴露（前端轮询）。
//!
//! ## 接口（全部在 /api 之下）
//! - `GET /api/tts/voices`：当前目录 + 元信息（source/fetched_at/probing）。
//! - `POST /api/tts/voices/refresh`：触发后台探测（admin 动作，expected_token 非空需鉴权）。
//! - `POST /api/tts/voices/test-credentials`：用表单三要素真实测一次合成（**只测不存**）。
//! 服务端启动时若已配置三要素且无缓存，会自动探测一次。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

use axum::{
    extract::{Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Router,
};
use serde::{Deserialize, Serialize};

use crate::app::ws::auth_ok;
use crate::engine::Engines;
use crate::plugins::tts::XfyunTtsConfig;

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
    /// 来源标识："probe"（API 探测通过）/ "empty"（尚未探测，空目录）。
    source: String,
    /// 探测时刻（Unix 秒；0 = 尚未探测）。
    fetched_at: u64,
    voices: Vec<Voice>,
}

static STATE: OnceLock<RwLock<Snapshot>> = OnceLock::new();
/// 后台探测进行中（前端轮询此标志，完成后重拉目录）。
static PROBING: AtomicBool = AtomicBool::new(false);

fn state() -> &'static RwLock<Snapshot> {
    // 空目录起步：列表**只来自真实 API 探测**（无硬编码可用列表）
    STATE.get_or_init(|| RwLock::new(Snapshot { source: "empty".into(), fetched_at: 0, voices: Vec::new() }))
}

/// 缓存文件路径（`XIAOZHI_VOICES_CACHE`，默认 /data/voices.json——随唯一数据卷持久化）。
fn cache_path() -> String {
    std::env::var("XIAOZHI_VOICES_CACHE").unwrap_or_else(|_| "/data/voices.json".to_string())
}

/// 启动初始化：优先读盘缓存；无缓存保持空目录（等待探测，不阻塞、不联网）。
pub fn init() {
    let path = cache_path();
    let loaded = std::fs::read_to_string(&path)
        .ok()
        .and_then(|t| serde_json::from_str::<Snapshot>(&t).ok())
        .filter(|s| !s.voices.is_empty());
    match loaded {
        Some(snap) => {
            tracing::info!(
                "发音人目录：已加载探测缓存 {}（{} 条，探测于 {}）",
                path,
                snap.voices.len(),
                snap.fetched_at
            );
            *state().write().unwrap_or_else(|e| e.into_inner()) = snap;
        }
        None => {
            tracing::info!("发音人目录：无缓存——配置三要素后自动/手动探测（未探测时列表为空）");
        }
    }
}

/// 启动自动探测：已配置三要素且当前目录为空时，后台探测一次（失败仅告警）。
pub fn spawn_startup_probe(engines: Arc<Engines>) {
    let Some(cfg) = tts_creds(&engines) else {
        return; // 未配置三要素：跳过（目录保持空，前端提示先填凭据）
    };
    let has_cache = !state().read().unwrap_or_else(|e| e.into_inner()).voices.is_empty();
    if has_cache {
        return; // 有缓存先用缓存（要更新点「刷新音色目录」）
    }
    tokio::spawn(async move {
        tracing::info!("发音人目录：启动探测开始（{} 个候选）", CANDIDATES.len());
        probe_and_persist(cfg).await;
    });
}

/// 三要素凭据：**优先读盘**（用户可能在启动后才保存）；读盘失败回退内存。
/// 三要素未填齐返回 None。
fn tts_creds(engines: &Engines) -> Option<XfyunTtsConfig> {
    let tts = match &engines.config_path {
        Some(p) => crate::config::Config::load(p).map(|c| c.tts).unwrap_or_else(|e| {
            tracing::warn!("发音人目录：读盘配置失败（用启动配置）: {e}");
            engines.config.tts.clone()
        }),
        None => engines.config.tts.clone(),
    };
    let x = tts.xfyun;
    if x.app_id.trim().is_empty() || x.api_key.trim().is_empty() || x.api_secret.trim().is_empty() {
        return None;
    }
    Some(x)
}

/// 后台探测：对候选池逐项调用 AIUI 合成 API 真实探测，通过的入目录并落盘。
///
/// ⚠️ **平台有并发限流**（实测：6 并发时大量音色假阴性报 11200/10163，串行时同一批
/// 音色全部通过）——故并发上限固定为 2，且失败项**重试一次**（间隔 400ms）以区分
/// 限流假阴性 vs 真实不可用；`10163`（参数错误=音色不存在）不重试。
/// 同刻只允许一个探测在跑（`PROBING` 守卫）。
async fn probe_and_persist(cfg: XfyunTtsConfig) {
    if PROBING.swap(true, Ordering::SeqCst) {
        tracing::info!("发音人目录：探测已在进行中，忽略本次触发");
        return;
    }
    let t0 = std::time::Instant::now();
    let sem = Arc::new(tokio::sync::Semaphore::new(2)); // 低并发：规避平台限流（实测必需）
    let mut tasks = Vec::with_capacity(CANDIDATES.len());
    for v in candidates() {
        let cfg = cfg.clone();
        let sem = sem.clone();
        tasks.push(tokio::spawn(async move {
            let _permit = sem.acquire().await;
            let probe_once = |cfg: XfyunTtsConfig, vcn: String| async move {
                tokio::task::spawn_blocking(move || {
                    crate::plugins::tts::aiui::probe_voice(&cfg, &vcn)
                })
                .await
                .unwrap_or_else(|e| Err(anyhow::anyhow!("探测任务异常: {e}")))
            };
            let first = probe_once(cfg.clone(), v.vcn.clone()).await;
            let ok = match first {
                Ok(_) => true,
                // 10163 = 参数错误（音色不存在）：不重试；其余（含限流 11200）重试一次
                Err(e) if format!("{e:#}").contains("10163") => false,
                Err(_) => {
                    tokio::time::sleep(std::time::Duration::from_millis(400)).await;
                    probe_once(cfg, v.vcn.clone()).await.is_ok()
                }
            };
            if ok {
                Some(v)
            } else {
                None
            }
        }));
    }
    let mut passed = Vec::new();
    for t in tasks {
        if let Ok(Some(v)) = t.await {
            passed.push(v);
        }
    }
    // 保持候选池顺序（并发完成序不稳定，按 CANDIDATES 排序输出，前端列表稳定）
    let order: std::collections::HashMap<String, usize> = CANDIDATES
        .iter()
        .enumerate()
        .map(|(i, c)| (c.0.to_string(), i))
        .collect();
    passed.sort_by_key(|v| order.get(&v.vcn).copied().unwrap_or(usize::MAX));

    let n = passed.len();
    persist(&passed, "probe");
    PROBING.store(false, Ordering::SeqCst);
    tracing::info!(
        "发音人目录：探测完成——{n}/{} 个音色可用（耗时 {}s，已写盘）",
        CANDIDATES.len(),
        t0.elapsed().as_secs()
    );
}

/// 落盘缓存并更新内存状态。
fn persist(voices: &[Voice], source: &str) {
    let snap = Snapshot {
        source: source.to_string(),
        fetched_at: now_secs(),
        voices: voices.to_vec(),
    };
    if let Err(e) = std::fs::write(
        cache_path(),
        serde_json::to_string_pretty(&snap).unwrap_or_default(),
    ) {
        tracing::warn!("发音人目录：写盘失败（仅内存生效）: {e}");
    }
    *state().write().unwrap_or_else(|e| e.into_inner()) = snap;
}

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
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
            "probing": PROBING.load(Ordering::SeqCst),
        })
        .to_string(),
    )
        .into_response()
}

/// POST /api/tts/voices/refresh：触发后台探测（立即返回，前端轮询 `probing` 结束后重拉目录）。
async fn refresh(
    State(engines): State<Arc<Engines>>,
    headers: HeaderMap,
    Query(query): Query<std::collections::HashMap<String, String>>,
) -> Response {
    if !auth_ok(&headers, &query, &engines.config.server) {
        return (StatusCode::UNAUTHORIZED, "unauthorized").into_response();
    }
    let Some(cfg) = tts_creds(&engines) else {
        return (
            StatusCode::BAD_REQUEST,
            "未配置 [tts.xfyun] 的 app_id / api_key / api_secret——音色目录由真实 API 探测生成，             请先在 TTS 配置卡填写三要素并保存",
        )
            .into_response();
    };
    if PROBING.load(Ordering::SeqCst) {
        return (
            [(header::CONTENT_TYPE, "application/json; charset=utf-8")],
            serde_json::json!({ "ok": true, "probing": true, "candidates": CANDIDATES.len() }).to_string(),
        )
            .into_response();
    }
    tokio::spawn(async move { probe_and_persist(cfg).await });
    (
        [(header::CONTENT_TYPE, "application/json; charset=utf-8")],
        serde_json::json!({ "ok": true, "probing": true, "candidates": CANDIDATES.len() }).to_string(),
    )
        .into_response()
}

/// POST /api/tts/voices/test-credentials：用传入的三要素真实测试一次合成
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
    let cfg = XfyunTtsConfig {
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
    };
    // 真连讯飞是阻塞调用（HTTPS 握手 + 合成 ~300ms+），放 spawn_blocking 隔离
    let result = tokio::task::spawn_blocking(move || crate::plugins::tts::aiui::test_credentials(&cfg))
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

/// 探测候选池（**不是音色列表**）：讯飞公开音色命名空间的完整清单（2026-10-07 从控制台
/// 目录拉取，148 条，含名称/性别/类型元数据）。服务端对候选项逐个做真实短合成，
/// **探测通过的才进入音色列表**——列表始终是实测结果，候选池仅声明"探测范围"。
const CANDIDATES: &[(&str, &str, &str, &str, &str)] = &[
        ("x4_lingxiaoying_em_v2", "聆小樱", "female", "classic", "交互 · 情感女声"),
        ("x_dangdang", "小飞小姐姐", "female", "classic", "通用场景 · 温柔女声"),
        ("x_qige", "小飞小哥哥", "male", "classic", "通用场景 · 成熟男声"),
        ("x2_xiaojuan", "讯飞小娟", "female", "classic", "通用场景 · 温柔女声"),
        ("x2_xiaoyuan", "讯飞小媛", "female", "classic", "新闻场景 · 温柔女声"),
        ("x2_xiaoxi", "讯飞水哥", "male", "classic", "小说场景 · 成熟男声"),
        ("x2_chongchong", "讯飞虫虫", "female", "classic", "客服场景 · 温柔女声"),
        ("x2_xiaoxue", "讯飞小雪", "female", "classic", "客服场景 · 温柔女声"),
        ("x2_pengfei", "讯飞小鹏", "male", "classic", "通用场景 · 成熟男声"),
        ("x2_yifei", "讯飞一菲", "female", "classic", "通用场景 · 温柔女声"),
        ("x2_chaoge", "讯飞超哥", "male", "classic", "通用场景 · 成熟男声"),
        ("x2_qige", "讯飞七哥", "male", "classic", "通用场景 · 成熟男声"),
        ("x2_xiaofang", "讯飞芳芳", "female", "classic", "故事场景/交互场景 · 可爱女童"),
        ("x2_nannan", "讯飞楠楠", "female", "classic", "故事场景/交互场景 · 可爱女童"),
        ("x2_wanshu", "讯飞万叔", "male", "classic", "小说场景 · 成熟男声"),
        ("x2_xiaofeng", "讯飞晓峰", "male", "classic", "新闻场景 · 成熟男声"),
        ("x2_xiaohou", "讯飞小侯", "male", "classic", "小说场景 · 成熟男声"),
        ("x2_xiaozhang", "讯飞刚哥", "male", "classic", "小说场景 · 成熟男声"),
        ("x2_yezi", "讯飞小露", "female", "classic", "交互场景/客服场景 · 温柔女声"),
        ("x2_qianqian", "讯飞倩倩", "female", "classic", "新闻场景 · 温柔女声"),
        ("x2_xiaoshi_cts", "讯飞小师", "female", "classic", "交互 · 温柔女声"),
        ("x2_xiaoyan", "讯飞晓燕", "female", "classic", "新闻场景 · 温柔女声"),
        ("x2_xiaoya", "讯飞小雅", "female", "classic", "客服场景 · 温柔女声"),
        ("x2_xiaomo", "讯飞小莫", "female", "classic", "客服场景 · 温柔女声"),
        ("x2_xiaolan", "讯飞小兰", "female", "classic", "客服场景 · 温柔女声"),
        ("x2_john", "John", "male", "classic", "多语种场景 · 成熟男声"),
        ("x2_xiaowanzi", "讯飞小桃丸", "female", "classic", "交互 · 可爱女童"),
        ("x2_xiaoxin", "讯飞萌小新", "male", "classic", "交互 · 可爱男童"),
        ("x2_ningning", "讯飞宁宁", "male", "classic", "交互 · 可爱男童"),
        ("x2_qianxue", "讯飞千雪", "female", "classic", "阅读场景 · 温柔女声"),
        ("x2_xiaoguo", "讯飞小果", "female", "classic", "新闻场景 · 温柔女声"),
        ("x2_xiaowan", "讯飞小婉", "female", "classic", "阅读场景 · 温柔女声"),
        ("x2_xiaolin", "讯飞晓琳", "female", "classic", "新闻场景 · 温柔女声"),
        ("x2_xiaoqian", "讯飞晓倩", "female", "classic", "新闻场景/方言场景 · 温柔女声"),
        ("x2_xiaorong", "讯飞小蓉", "female", "classic", "新闻场景/方言场景 · 温柔女声"),
        ("x2_xiaoying", "讯飞小莹", "female", "classic", "新闻场景/方言场景 · 温柔女声"),
        ("x2_xiaoqiang", "讯飞小强", "male", "classic", "新闻场景/方言场景 · 成熟男声"),
        ("x2_xiaokun", "讯飞小坤", "male", "classic", "新闻场景/方言场景 · 成熟男声"),
        ("x2_xiaodong", "讯飞小东", "male", "classic", "新闻场景/方言场景 · 成熟男声"),
        ("x2_xiaowang", "讯飞小王", "male", "classic", "新闻场景/方言场景 · 成熟男声"),
        ("x2_xiaofei", "讯飞小肥", "male", "classic", "新闻场景/方言场景 · 成熟男声"),
        ("x2_xiaobao", "讯飞小包", "male", "classic", "新闻场景/方言场景 · 成熟男声"),
        ("x2_catherine", "讯飞凯瑟琳", "female", "classic", "多语种场景 · 温柔女声"),
        ("x_mengmengneutral", "讯飞萌萌-中立", "female", "classic", "交互 · 可爱女童"),
        ("x2_yifeng", "讯飞一峰", "male", "classic", "阅读场景 · 磁性男声"),
        ("x2_qianxue2", "讯飞千雪-情感", "female", "classic", "阅读场景 · 情感女声"),
        ("x2_yuanye", "讯飞易阳泽(原野)", "male", "classic", "阅读场景 · 磁性男声"),
        ("x2_xiaoding", "讯飞小丁", "female", "classic", "客服场景 · 亲切女声"),
        ("x2_xiaoxuan", "讯飞晓璇", "female", "classic", "交互 · 甜美女声"),
        ("x2_dahuilang", "讯飞康铭(大灰狼)", "male", "classic", "阅读场景 · 磁性男声"),
        ("x2_yiping", "讯飞一萍", "female", "classic", "阅读场景 · 亲切女声"),
        ("x2_guange", "管哥", "male", "classic", "阅读场景 · 磁性男声"),
        ("x2_mingge", "明哥", "male", "classic", "阅读场景 · 小说男声"),
        ("x2_tiange", "天哥", "male", "classic", "阅读场景 · 小说男声"),
        ("x2_chunchun", "讯飞春春", "female", "classic", "客服场景 · 温柔女声"),
        ("x2_dangdang", "讯飞当当", "female", "classic", "交互 · 温柔女声"),
        ("x2_xiaozhong", "小忠", "male", "classic", "阅读场景 · 磁性男声"),
        ("x2_xiaoai_novel", "讯飞诺诺", "female", "classic", "阅读场景 · 温柔女声"),
        ("sgron_dkar", "达娃", "female", "classic", "民族语场景 · 藏语女声"),
        ("x2_xiaoyue", "小月", "female", "classic", "方言场景 · 香港粤语"),
        ("x2_abha", "艾伯哈", "female", "classic", "多语种场景 · 印地语女声"),
        ("x2_miya", "miya", "female", "classic", "多语种场景 · 韩语女声"),
        ("x2_keshu", "keshu", "female", "classic", "多语种场景 · 俄语女声"),
        ("x2_lrina", "lrina", "female", "classic", "多语种场景 · 意大利语女声"),
        ("x2_felisa", "felisa", "female", "classic", "多语种场景 · 西班牙语女声"),
        ("x2_natali", "natali", "female", "classic", "多语种场景 · 波兰女声"),
        ("x2_mariane", "玛丽安", "female", "classic", "多语种场景 · 法语女声"),
        ("x2_suparut", "suparut", "female", "classic", "多语种场景 · 泰语女声"),
        ("x2_zhongcun", "zhongcun", "female", "classic", "多语种场景 · 日语女声"),
        ("x2_malayrole", "malayrole", "female", "classic", "多语种场景 · 马来女声"),
        ("x2_christiane", "christiane", "female", "classic", "多语种场景 · 德语女声"),
        ("x2_annakz", "annakz", "female", "classic", "多语种场景 · 哈萨克(西里尔)女"),
        ("x2_mohamed", "mohamed", "female", "classic", "多语种场景 · 阿拉伯女声"),
        ("x4_enuk_ashleigh_assist", "ashleigh", "female", "classic", "多语种场景 · 温柔女声"),
        ("x4_enuk_amanda_education", "amanda", "female", "classic", "多语种场景 · 温柔女声"),
        ("x4_enus_gavin_assist", "gavin", "male", "classic", "多语种场景 · 成熟男声"),
        ("x4_lingxiaowan_boytalk", "聆万万", "male", "classic", "交互 · 可爱男童"),
        ("x2_SuhCn_XiXi", "讯飞苏小曦", "female", "classic", "方言场景 · 温柔女声"),
        ("x4_enus_luna_assist", "luna", "female", "classic", "多语种场景 · 温柔女声"),
        ("x3_ziling", "讯飞阮灵", "female", "classic", "新闻场景/方言场景 · 温柔女声"),
        ("x4_lingxiaoyao_en", "聆小瑶", "female", "classic", "交互 · 情感女声"),
        ("x4_lingxiaoqi_oral", "聆小琪（超拟人）", "female", "classic", "交互 · 情感女声"),
        ("x4_lingfeizhe_oral", "聆飞哲（超拟人）", "male", "classic", "交互 · 成熟男声"),
        ("x4_lingyouyou_oral", "聆佑佑（超拟人）", "female", "classic", "交互 · 可爱女童"),
        ("x4_zijin_oral", "子津（超拟人）", "male", "classic", "交互 · 成熟男声"),
        ("x4_ziyang_oral", "子阳（超拟人）", "male", "classic", "交互 · 成熟男声"),
        ("x4_lingxiaoli_oral", "聆小璃（超拟人）", "female", "classic", "交互 · 情感女声"),
        ("x4_EnUs_Lila_emo", "Lila（超拟人）", "female", "classic", "多语种场景 · 温柔女声"),
        ("x4_EnUs_Grant_emo", "Grant（超拟人）", "male", "classic", "多语种场景 · 成熟男声"),
        ("x6_lingxiaoyue_pro", "聆小玥", "female", "x6", "交互 · 情感女声"),
        ("x6_lingfeiyi_pro", "聆飞逸", "male", "x6", "交互 · 成熟男声"),
        ("x6_lingyuyan_pro", "聆玉言", "female", "x6", "交互 · 情感女声"),
        ("x5_lingxiaotang_flow", "聆小糖", "female", "x6", "交互 · 情感女声"),
        ("x6_lingxiaoxuan_pro", "聆小璇", "female", "x6", "交互 · 情感女声"),
        ("x5_lingyuzhao_flow", "聆玉昭", "female", "x6", "交互 · 情感女声"),
        ("x6_lingxiaoying_pro", "聆小颖", "female", "x6", "交互 · 情感女声"),
        ("x6_lingfeibo_pro", "聆飞博", "male", "x6", "交互 · 成熟男声"),
        ("x6_gaolengnanshen_pro", "高冷男神", "male", "x6", "交互 · 成熟男声"),
        ("x6_waiguodashu_pro", "外国人大叔", "male", "x6", "交互 · 成熟男声"),
        ("x6_dongmanshaonv_pro", "动漫少女", "female", "x6", "交互 · 情感女声"),
        ("x6_lingxiaozhen_pro", "聆小瑱", "female", "x6", "交互 · 情感女声"),
        ("x6_lingyouyou_pro", "聆佑佑", "female", "x6", "交互 · 可爱女童"),
        ("x6_gufengpangbai_pro", "古风旁白", "male", "x6", "旁白 · 成熟男声"),
        ("x6_ganliannvxing_pro", "干练女性", "female", "x6", "交互 · 情感女声"),
        ("x6_pangbainan1_pro", "旁白男声", "male", "x6", "旁白"),
        ("x6_lingfeihan_pro", "聆飞瀚", "male", "x6", "旁白 · 成熟男声"),
        ("x6_lingfeihao_pro", "聆飞皓", "male", "x6", "旁白 · 成熟男声"),
        ("x6_lingyufei_pro", "聆玉菲", "female", "x6", "交互 · 情感女声"),
        ("x6_ruyadashu_pro", "儒雅大叔", "male", "x6", "旁白 · 成熟男声"),
        ("x6_pangbainv1_pro", "旁白女声", "female", "x6", "交互"),
        ("x6_lingxiaoyun_pro", "聆小芸", "female", "x6", "交互 · 情感女声"),
        ("x6_lingyuaner_pro", "聆园儿", "female", "x6", "交互 · 情感女声"),
        ("x6_lingxiaoshan_pro", "聆小珊", "female", "x6", "交互 · 情感女声"),
        ("x6_feizheChat_pro", "聆飞哲", "male", "x6", "交互 · 成熟男声"),
        ("x6_lingxiaoli_pro", "聆小璃", "female", "x6", "交互 · 情感女声"),
        ("x6_xiaoqiChat_pro", "聆小琪", "female", "x6", "交互 · 情感女声"),
        ("x6_huanlemianbao_pro", "海绵宝宝", "male", "x6", "交互 · 童年男声"),
        ("x6_dudulibao_pro", "少女可莉", "female", "x6", "交互 · 童年女声"),
        ("x6_cuishounvsheng_pro", "催收女声", "female", "x6", "交互"),
        ("x6_yingxiaonv_pro", "营销女声", "female", "x6", "交互"),
        ("x6_shibingnvsheng_mini", "士兵女声", "female", "x6", "交互"),
        ("x6_kongbunvsheng_mini", "恐怖女声", "female", "x6", "交互"),
        ("x6_yulexinwennvsheng_mini", "娱乐新闻女声", "female", "x6", "交互"),
        ("x6_jingqudaolannvsheng_mini", "景区导览女声", "female", "x6", "交互"),
        ("x6_wumeinv_pro", "妩媚姐姐", "female", "x6", "交互 · 成年女声"),
        ("x6_huajidama_pro", "滑稽大妈", "female", "x6", "交互 · 成年女声"),
        ("x6_lingxiaoxue_pro", "聆小雪", "female", "x6", "交互 · 成年女声"),
        ("x6_gufengxianv_mini", "古风侠女", "female", "x6", "交互 · 成年女声"),
        ("x6_wuyediantai_mini", "午夜电台", "female", "x6", "交互 · 成年女声"),
        ("x6_wennuancixingnansheng_mini", "温暖磁性男声", "male", "x6", "交互"),
        ("x6_xiaonaigoudidi_mini", "小奶狗弟弟", "male", "x6", "交互 · 成年男声"),
        ("x6_wenrounansheng_mini", "温柔男声", "male", "x6", "交互"),
        ("x6_daqixuanchuanpiannansheng_mini", "大气宣传片男声", "male", "x6", "交互"),
        ("x6_xiangruiyingyu_pro", "商务殷语", "male", "x6", "交互 · 成年男声"),
        ("x6_taiqiangnuannan_pro", "台湾腔温柔男声", "male", "x6", "交互"),
        ("x6_lingbosong_pro", "聆伯松", "male", "x6", "交互 · 成年男声"),
        ("x6_huoposhaonian_pro", "活泼少年", "male", "x6", "交互 · 成年男声"),
        ("x6_tiexinnanyou_mini", "贴心男友", "male", "x6", "交互 · 成年男声"),
        ("x6_youxinanshibing_pro", "士兵男声", "male", "x6", "交互"),
        ("x6_zhuanyenvzhuchi_pro", "大会主持女声", "female", "x6", "交互"),
        ("x6_zuixianlibai_pro", "醉仙李白", "male", "x6", "交互 · 成年男声"),
        ("x6_shiwangxiaoxin_pro", "奶凶辛巴", "male", "x6", "交互 · 可爱男童"),
        ("x6_bokenansheng_pro", "播客男声", "male", "x6", "交互"),
        ("x6_zhuanyenanzhuchi_pro", "大会主持男声", "male", "x6", "交互"),
        ("x6_ranzhinvdazi_pro", "运动陪练女声", "female", "x6", "交互"),
        ("x6_zhantingnanjiedai_pro", "展厅接待男声", "male", "x6", "交互"),
        ("x6_zhantingnvjiedai_pro", "展厅接待女声", "female", "x6", "交互"),
        ("x6_huifangnv_pro", "回访女声", "female", "x6", "交互"),
];

/// 由候选池构造完整 Voice 列表（probe 会按合成结果过滤）。
fn candidates() -> Vec<Voice> {
    CANDIDATES
        .iter()
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

    /// 无硬编码可用列表的契约：内存态初始为空目录（仅探测/缓存填充）。
    #[test]
    fn state_starts_empty_without_cache() {
        std::env::set_var("XIAOZHI_VOICES_CACHE", "/nonexistent/voices-test.json");
        init();
        let snap = state().read().unwrap_or_else(|e| e.into_inner()).clone();
        assert!(snap.voices.is_empty(), "无缓存时应为空目录（不得回退硬编码可用列表）");
        assert_eq!(snap.source, "empty");
        std::env::remove_var("XIAOZHI_VOICES_CACHE");
    }

    /// 候选池契约：vcn 唯一、gender/type 合法（候选只是探测输入，不是可用列表）。
    #[test]
    fn candidates_are_unique_and_wellformed() {
        let c = candidates();
        let mut seen = std::collections::HashSet::new();
        for v in &c {
            assert!(seen.insert(v.vcn.clone()), "候选 vcn 重复: {}", v.vcn);
            assert!(matches!(v.gender.as_str(), "female" | "male"), "非法性别: {}", v.vcn);
            assert!(matches!(v.type_.as_str(), "classic" | "x6"), "非法类型: {}", v.vcn);
        }
        assert!(c.len() >= 100, "候选池过小（疑似被截断）: {}", c.len());
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

    /// 真实探测冒烟（**排障工具，日常不跑**）：三要素经环境变量传入——
    /// `XFYUN_APP_ID=… XFYUN_API_KEY=… XFYUN_API_SECRET=… cargo test voices_probe_live -- --ignored --nocapture`
    #[test]
    #[ignore = "live AIUI 调用：需 XFYUN_APP_ID/XFYUN_API_KEY/XFYUN_API_SECRET 环境变量"]
    fn voices_probe_live() {
        let (Ok(app_id), Ok(api_key), Ok(api_secret)) = (
            std::env::var("XFYUN_APP_ID"),
            std::env::var("XFYUN_API_KEY"),
            std::env::var("XFYUN_API_SECRET"),
        ) else {
            eprintln!("跳过：未设置 XFYUN_APP_ID/XFYUN_API_KEY/XFYUN_API_SECRET");
            return;
        };
        let cfg = XfyunTtsConfig { app_id, api_key, api_secret, voice: String::new() };
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let t0 = std::time::Instant::now();
        rt.block_on(probe_and_persist(cfg));
        let snap = state().read().unwrap_or_else(|e| e.into_inner()).clone();
        println!(
            "== 探测完成：{} 条可用（耗时 {}s），示例: {:?}",
            snap.voices.len(),
            t0.elapsed().as_secs(),
            snap.voices.first().map(|v| (&v.vcn, &v.name, &v.type_, v.gender.as_str()))
        );
    }
}
