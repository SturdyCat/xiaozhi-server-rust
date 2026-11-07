//! 天气预报能力（NMC 国家气象中心公开接口，纯 Rust 复刻）。
//!
//! 本模块是 **skills 目录唯一允许联网的能力** 的实际网络发生地：持有 reqwest 阻塞客户端
//! 与按站点码缓存的简易 TTL 缓存。skills 目录只做调度与结果组装，不内联 HTTP（见
//! `server/src/skills/AGENTS.md`）。
//!
//! 数据来源：国家气象中心公开接口 `https://www.nmc.cn`（`valenovo/china-weather-agent-skill`
//! 同款端点，但本实现为纯 Rust，不 vendoring 任何 python 脚本——部署镜像 `debian:bookworm-slim`
//! 没有 python3）。解析与归一化见 [`parse`] 子模块。
//!
//! 解析原则：宽松、零 panic。任何外部失败都降级为 `{"ok":false,"speech":"..."}`，绝不崩溃。

mod parse;
#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use parse::{
    extract_time, match_station, parse_forecast, parse_forecast_at, parse_realtime, summarize,
};

/// 天气插件配置段（`[weather]`）。所有字段带 `#[serde(default)]`，缺省即工作。
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct WeatherConfig {
    /// 数据引擎标识（当前仅 "nmc" = 国家气象中心）。
    pub engine: String,
    /// 数据源根地址；默认 `https://www.nmc.cn`。
    pub source: String,
    /// 单次请求 connect/总超时（秒）。
    pub timeout_secs: u64,
    /// 按站点码缓存天气数据的 TTL（秒）。
    pub cache_secs: u64,
    /// 技能查询后端（绞杀者双跑开关）：`native` = 本模块 reqwest 实现（默认）；
    /// `wasm` = 经 `skill.weather` 扩展点插件（官方默认 `weather.official`，或用户
    /// 兼容实现）。wasm 缺依赖（feature 未编入/插件未装载）**明确报错，不静默回退**。
    pub backend: String,
}

impl Default for WeatherConfig {
    fn default() -> Self {
        Self {
            engine: "nmc".into(),
            source: "https://www.nmc.cn".into(),
            timeout_secs: 8,
            cache_secs: 600,
            backend: "native".into(),
        }
    }
}

/// 天气服务：持有阻塞 HTTP 客户端、数据源地址与按站点码的 TTL 缓存。
///
/// 此外维护一份懒加载的「省→城市→站点码」映射，避免每次解析城市都重新拉取全国列表。
pub struct WeatherService {
    /// 阻塞客户端**懒创建**：`reqwest::blocking::Client::build` 会在当前线程上初始化
    /// tokio blocking pool，**不能在 async 上下文里做**（`Engines::assemble` 运行在
    /// `block_on` 内，启动即 panic「Cannot drop a runtime…」——2026-11-03 事故根因）。
    /// 首次请求发生在 skills 的 `spawn_blocking` 线程里，那时创建是安全的。
    client: std::sync::OnceLock<reqwest::blocking::Client>,
    timeout_secs: u64,
    source: String,
    /// 按 station code 缓存天气 `data` 对象，TTL = `cache_secs`。
    cache: Mutex<HashMap<String, (Instant, Value)>>,
    cache_secs: u64,
    /// 懒加载的「(城市名, 站点码, 省份名)」列表；首次解析后缓存，后续直接内存匹配。
    stations: Mutex<Option<Vec<(String, String, String)>>>,
    /// 技能查询后端开关（`[weather].backend`）。
    backend: String,
    /// wasm 宿主句柄（feature 编入且初始化成功时由 Engines 装配注入；OnceLock：
    /// 装配一次性，运行期只读）。native 后端永不触碰。
    #[cfg(feature = "wasm-plugins")]
    wasm: std::sync::OnceLock<std::sync::Arc<crate::plugins::wasm::WasmHost>>,
}

impl WeatherService {
    /// 只记录配置，不建 HTTP 客户端（见 [WeatherService.client] 的字段注释）。
    pub fn new(cfg: &WeatherConfig) -> Self {
        Self {
            client: std::sync::OnceLock::new(),
            timeout_secs: cfg.timeout_secs,
            source: cfg.source.clone(),
            cache: Mutex::new(HashMap::new()),
            cache_secs: cfg.cache_secs,
            stations: Mutex::new(None),
            backend: cfg.backend.clone(),
            #[cfg(feature = "wasm-plugins")]
            wasm: std::sync::OnceLock::new(),
        }
    }

    /// 注入 wasm 宿主（Engines 装配期一次性；backend=wasm 时的事件桥执行载体）。
    #[cfg(feature = "wasm-plugins")]
    pub fn set_wasm_host(&self, host: std::sync::Arc<crate::plugins::wasm::WasmHost>) {
        let _ = self.wasm.set(host);
    }

    /// 取阻塞客户端（首次调用时创建；调用方必须在非 async 上下文，即 spawn_blocking）。
    fn client(&self) -> Result<&reqwest::blocking::Client> {
        self.client.get_or_init(|| {
            reqwest::blocking::Client::builder()
                .connect_timeout(Duration::from_secs(self.timeout_secs))
                .timeout(Duration::from_secs(self.timeout_secs))
                .user_agent(concat!("xiaozhi-server-rust/", env!("CARGO_PKG_VERSION")))
                .build()
                .unwrap_or_else(|_| reqwest::blocking::Client::new())
        });
        self.client.get().context("weather HTTP client 不可用")
    }

    /// GET 一个 JSON 接口：拼 `source` + path，3 次重试（0.4s 起指数退避）。
    fn http_get_json(&self, path: &str) -> Result<Value> {
        let url = format!("{}{}", self.source.trim_end_matches('/'), path);
        let client = self.client()?;
        let mut backoff = Duration::from_millis(400);
        let mut last_err: Option<anyhow::Error> = None;
        for attempt in 0..3 {
            let result = (|| -> Result<Value> {
                let resp = client.get(&url).send().context("request send failed")?;
                let resp = resp.error_for_status().context("http non-2xx status")?;
                let text = resp.text().context("read response body failed")?;
                let v: Value = serde_json::from_str(&text).context("parse response json failed")?;
                Ok(v)
            })();
            match result {
                Ok(v) => return Ok(v),
                Err(e) => {
                    last_err = Some(e);
                    if attempt < 2 {
                        std::thread::sleep(backoff);
                        backoff = backoff.saturating_mul(2);
                    }
                }
            }
        }
        Err(last_err.unwrap_or_else(|| anyhow!("unknown network error")))
    }

    /// 解析城市名为 NMC 站点码：拉取全国省份列表与各省城市列表，按归一化名匹配。
    /// `province` 非空时优先按省过滤同名城市。匹配不到返回 `Err`。
    fn resolve_station(&self, city: &str, province: &str) -> Result<String> {
        // 已缓存映射则直接匹配
        {
            let guard = self.stations.lock().unwrap();
            if let Some(list) = guard.as_ref() {
                if let Some(code) = match_station(list, city, province) {
                    return Ok(code);
                }
            }
        }
        // 懒加载全国「城市→站点码→省份」映射
        let mut list: Vec<(String, String, String)> = Vec::new();
        let provinces = self
            .http_get_json("/rest/province/all")?
            .as_array()
            .context("province list is not an array")?
            .iter()
            .filter_map(|p| {
                let code = p.get("code").and_then(Value::as_str)?.to_string();
                let name = p
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                Some((code, name))
            })
            .collect::<Vec<_>>();
        for (pcode, pname) in provinces {
            let cities = self.http_get_json(&format!("/rest/province/{}", pcode));
            if let Ok(cities) = cities {
                if let Some(arr) = cities.as_array() {
                    for c in arr {
                        let cname = c
                            .get("city")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string();
                        let ccode = c
                            .get("code")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string();
                        let cprov = c
                            .get("province")
                            .and_then(Value::as_str)
                            .unwrap_or(&pname)
                            .to_string();
                        if !cname.is_empty() && !ccode.is_empty() {
                            list.push((cname, ccode, cprov));
                        }
                    }
                }
            }
        }
        if list.is_empty() {
            return Err(anyhow!("无法获取城市列表（网络异常或接口变更）"));
        }
        {
            let mut guard = self.stations.lock().unwrap();
            *guard = Some(list.clone());
        }
        match_station(&list, city, province).ok_or_else(|| anyhow!("找不到城市 {}", city))
    }

    /// 取天气 `data`：先查 TTL 缓存，未命中再请求并写入缓存。`code != 0` 视为失败。
    fn cached_or_fetch(&self, code: &str) -> Result<Value> {
        {
            let guard = self.cache.lock().unwrap();
            if let Some((ts, v)) = guard.get(code) {
                if ts.elapsed().as_secs() < self.cache_secs {
                    return Ok(v.clone());
                }
            }
        }
        let v = self.http_get_json(&format!("/rest/weather?stationid={}", code))?;
        if v.get("code").and_then(Value::as_i64).unwrap_or(-1) != 0 {
            return Err(anyhow!("weather api returned code != 0"));
        }
        let data = v
            .get("data")
            .cloned()
            .context("weather response missing data")?;
        {
            let mut guard = self.cache.lock().unwrap();
            guard.insert(code.to_string(), (Instant::now(), data.clone()));
        }
        Ok(data)
    }

    /// 查询天气入口（后端分发）：`native` = 本实现；`wasm` = `skill.weather` 扩展点
    /// 插件。永远返回成功形态的 `Value`（不会 panic、不会返回 `Err`）。
    pub fn forecast(&self, city: &str, province: &str, range: &str) -> Value {
        match self.backend.as_str() {
            "native" => self.forecast_native(city, province, range),
            "wasm" => self.forecast_wasm(city, province, range),
            other => json!({
                "ok": false,
                "speech": format!("未知天气后端 {other:?}（合法：native / wasm）"),
            }),
        }
    }

    /// wasm 后端：宿主句柄在则走事件桥；缺依赖明确报错（不静默回退 native——
    /// 双跑对照的价值在于差异可见，静默回退会把插件故障伪装成正常）。
    #[cfg(feature = "wasm-plugins")]
    fn forecast_wasm(&self, city: &str, province: &str, range: &str) -> Value {
        match self.wasm.get() {
            Some(host) => crate::plugins::wasm::bridge::forecast(host, city, province, range),
            None => json!({
                "ok": false,
                "speech": "天气后端为 wasm，但插件宿主未初始化（见服务启动日志）",
            }),
        }
    }

    #[cfg(not(feature = "wasm-plugins"))]
    fn forecast_wasm(&self, _city: &str, _province: &str, _range: &str) -> Value {
        json!({
            "ok": false,
            "speech": "天气后端为 wasm，但服务未编译 wasm-plugins feature（重新构建或改回 native）",
        })
    }

    /// 原生实现：reqwest 直连 NMC。
    ///
    /// 流程：① 解析有效城市（空 → 回落服务器出口 IP 默认定位）；② 从城市串二次切出时段
    /// 关键词；③ 解析站点码；④ 拉取天气；⑤ 按 range 选择实时/单日/7 天。任意外部失败
    /// 都降级为 `{"ok":false,"speech":"..."}`。
    fn forecast_native(&self, city: &str, province: &str, range: &str) -> Value {
        // ① 清洗城市串（去掉「天气/预报」等非地名词）并二次切分时段关键词
        //   （city 串内嵌「今天/明天/...」由 weather.forecast 处理）
        let (city_cut, range_eff) = extract_time(&parse::strip_weather_words(city), range);
        let range_eff = if range_eff.is_empty() {
            range.trim().to_string()
        } else {
            range_eff
        };
        let city_cut = city_cut.trim().to_string();

        // ② 解析有效城市（切分后为空 → 回落服务器出口 IP 定位）
        let (city_clean, province_eff) = if city_cut.is_empty() {
            match crate::plugins::geo::current() {
                Some(loc) => {
                    let c = if loc.city.trim().is_empty() {
                        loc.province.clone()
                    } else {
                        loc.city.clone()
                    };
                    (c, loc.province.clone())
                }
                None => {
                    return json!({
                        "ok": false,
                        "speech": "未指定城市，且服务器未能定位默认地区（可能处于境外网络），请告诉我城市名称"
                    });
                }
            }
        } else {
            (city_cut, province.to_string())
        };

        // ③ 解析站点码
        let station_code = match self.resolve_station(&city_clean, &province_eff) {
            Ok(code) => code,
            Err(_) => {
                return json!({
                    "ok": false,
                    "speech": format!("未找到城市 {}，请确认城市名称或补充省份", city_clean)
                });
            }
        };

        // ④ 拉取天气 data
        let data = match self.cached_or_fetch(&station_code) {
            Ok(d) => d,
            Err(_) => {
                return json!({
                    "ok": false,
                    "speech": format!("{} 天气服务暂时不可用，请稍后再试", city_clean)
                });
            }
        };

        // ⑤ 按 range 选择模式
        let is_7day = ["一周", "七天", "7天", "未来"]
            .iter()
            .any(|w| range_eff.contains(w));
        let is_tomorrow = range_eff.contains("明天");
        let is_dayafter = range_eff.contains("后天");

        let (realtime, forecast_items, hourly) = if is_7day {
            (Value::Null, parse_forecast(&data, 7), Value::Null)
        } else if is_tomorrow {
            (Value::Null, parse_forecast_at(&data, 1), Value::Null)
        } else if is_dayafter {
            (Value::Null, parse_forecast_at(&data, 2), Value::Null)
        } else {
            let rt = parse_realtime(&data);
            let hr = data
                .get("passedchart")
                .and_then(Value::as_array)
                .map(|arr| json!(arr.iter().take(6).cloned().collect::<Vec<_>>()))
                .unwrap_or(Value::Null);
            (rt, Value::Null, hr)
        };

        let range_norm: &str = if is_7day {
            "近7天"
        } else if is_tomorrow {
            "明天"
        } else if is_dayafter {
            "后天"
        } else if range_eff.contains("当前") || range_eff.contains("实时") {
            "当前"
        } else if range_eff.contains("今天") {
            "今天"
        } else {
            "实时"
        };

        let publish_time = data
            .get("real")
            .and_then(|r| r.get("publish_time"))
            .and_then(parse::clean)
            .unwrap_or_default();
        let speech = summarize(
            &city_clean,
            range_norm,
            &realtime,
            forecast_items.as_array(),
        );
        let update_time = publish_time.clone();

        json!({
            "ok": true,
            "source": "nmc",
            "location": {
                "province": province_eff,
                "city": city_clean,
                "code": station_code,
            },
            "range": range_norm,
            "realtime": realtime,
            "forecast": forecast_items,
            "hourly": hourly,
            "publish_time": publish_time,
            "speech": speech,
            "update_time": update_time,
        })
    }
}

// ───────────────────────── 进程级全局单例 ─────────────────────────

static WEATHER: std::sync::OnceLock<std::sync::Arc<WeatherService>> = std::sync::OnceLock::new();

/// 设置进程级天气服务单例（重复 set 静默忽略）。
pub fn set_global(s: std::sync::Arc<WeatherService>) {
    let _ = WEATHER.set(s);
}

/// 取进程级天气服务单例（未初始化返回 None）。
pub fn global() -> Option<std::sync::Arc<WeatherService>> {
    WEATHER.get().cloned()
}
