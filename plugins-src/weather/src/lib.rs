// weather.official：NMC 天气查询（官方默认插件，P1c 试点）。
//
// 执行模型 = 纯决策瀑布：宿主喂事件 → 本插件同步返回效果（http-get / kv-put / log）→
// 宿主执行后按步批量回灌 `http.responses`。多步流程的上下文编码在 request-id 里
//（回灌事件的唯一关联键；各字段以 `|` 连接——地名/req_id 约定不含 `|`，见 finish_* 注释）。
//
// 与原生实现（plugins/weather/）共享同一份 parse.rs（#[path] 单源；
// 绞杀者方向：P8 反转为原生从这里取）。全站城市列表刻意不缓存（结果 kv 值上限 64KB
// 放不下），只按城市缓存站点码与天气结果。

#[global_allocator]
static ALLOC: guest_alloc::GrowingBump = guest_alloc::GrowingBump::new();

#[path = "../../../plugins/weather/parse.rs"]
mod parse;

use serde_json::{json, Value};
use xiaozhi_plugin_sdk::builder::{FieldSpec, ManifestBuilder, RegistrationBuilder};
use xiaozhi_plugin_sdk::exports::lifecycle::Guest;
use xiaozhi_plugin_sdk::xiaozhi::plugin::{host, types};

/// 数据源（对齐原生默认 source；config-read 导入随 P2 契约扩展后可配置化）。
const SOURCE: &str = "https://www.nmc.cn";
/// 结果缓存 TTL（对齐原生默认 cache_secs=600）。
const CACHE_TTL_MS: u64 = 600_000;

struct Component;

impl Guest for Component {
    fn manifest() -> types::PluginManifest {
        ManifestBuilder::new("weather.official", "天气（国家气象中心）")
            .expect("静态构建")
            .version("0.1.0")
            .description("NMC 公开接口查询：城市解析→站点码→实时/预报，kv 缓存 + http 瀑布")
            .permission("kv")
            .expect("kv 权限")
            .permission("http-out:www.nmc.cn")
            .expect("http-out 权限")
            .implements("skill.weather")
            .expect("扩展点")
            .official(true)
            .build()
            .expect("静态构建")
    }

    fn register() -> types::Registration {
        RegistrationBuilder::new()
            .menu_entry("nav.main", "weather", "天气", "插件", 60, "settings")
            .expect("静态构建")
            .config_field(
                FieldSpec::str("source", "数据源根地址")
                    .hint("https://www.nmc.cn")
                    .help("NMC 公开接口根地址")
                    .build(),
            )
            .expect("静态构建")
            .config_field(
                FieldSpec::int("cache_secs", "结果缓存秒数")
                    .hint("600")
                    .help("同城市同时段的结果缓存 TTL")
                    .build(),
            )
            .expect("静态构建")
            .action(
                "forecast",
                "查天气",
                false,
                "事件通道全流程查询；动作仅做参数提示",
            )
            .expect("静态构建")
            .build()
    }

    fn handle_event(event: String, payload: String) -> Result<Vec<types::Effect>, String> {
        match event.as_str() {
            "weather.forecast" => on_forecast(&payload),
            "http.responses" => on_responses(&payload),
            other => Err(format!("未知事件：{other}")),
        }
    }

    fn handle_action(action_id: String, payload: String) -> Result<String, String> {
        match action_id.as_str() {
            // 双调用方演示：真正的查询是异步瀑布（事件通道），动作侧只做即时校验
            "forecast" => Ok(format!(
                "天气查询走事件通道（weather.forecast）。收到的参数：{payload}"
            )),
            other => Err(format!("未知动作：{other}")),
        }
    }

    fn self_test() -> Result<Vec<types::TestResult>, String> {
        // 证明共享 parse.rs 在 wasm 内真实可用（温度提取 + 站点匹配）
        let data = json!({"real": {"weather": {"temperature": "22"}, "wind": {}}});
        let rt = parse::parse_realtime(&data);
        let ok_rt = rt.get("temp_c").and_then(Value::as_str) == Some("22");
        let list = vec![(
            "杭州".to_string(),
            "58457".to_string(),
            "浙江省".to_string(),
        )];
        let ok_match = parse::match_station(&list, "杭州市", "浙江") == Some("58457".into());
        Ok(vec![
            types::TestResult {
                name: "parse-realtime".into(),
                ok: ok_rt,
                detail: format!("temp_c={:?}", rt.get("temp_c")),
            },
            types::TestResult {
                name: "match-station".into(),
                ok: ok_match,
                detail: "杭州市→58457".into(),
            },
        ])
    }
}

// ============================================================
// 第一步：weather.forecast（载荷 {req_id, city, province, range}）
// ============================================================

fn on_forecast(payload: &str) -> Result<Vec<types::Effect>, String> {
    let v: Value = serde_json::from_str(payload).map_err(|e| format!("载荷非法：{e}"))?;
    let req_id = v.get("req_id").and_then(Value::as_str).unwrap_or_default();
    let province = v
        .get("province")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if req_id.is_empty() {
        return Err("缺少 req_id".to_string());
    }

    // 清洗城市串 + 二次切分时段词（同原生 forecast ①②）
    let city_raw = v.get("city").and_then(Value::as_str).unwrap_or_default();
    let (city, range) = {
        let cleaned = parse::strip_weather_words(city_raw);
        let (c, r) = parse::extract_time(
            &cleaned,
            v.get("range").and_then(Value::as_str).unwrap_or_default(),
        );
        let range = if r.is_empty() {
            v.get("range")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim()
                .to_string()
        } else {
            r
        };
        (c, range)
    };

    // geo 默认定位回落属宿主服务：事件源侧（P2 接线）负责预填，这里明确报缺参
    if city.is_empty() {
        return Ok(finish_fail(req_id, "未指定城市，请告诉我城市名称"));
    }

    // 站点码缓存命中：天气原始数据也按 station code 缓存，和 native 一样让不同
    // 城市别名/查询时段共享一次 HTTP 拉取；结果文本仍按本次 range 现场组装。
    if let Some(code) = host::kv_get(&station_key(&city)) {
        let code = String::from_utf8_lossy(&code).into_owned();
        if let Some(data) = cached_weather_data(&code) {
            let (result, _) = build_result(&city, &province, &code, &range, &data);
            return Ok(finish_result(req_id, &result, None));
        }
        return Ok(vec![http_get(
            &wx_id(&code, req_id, &city, province, &range),
            &wx_url(&code),
        )]);
    }

    // 冷路径第一步：全国省份列表（ctx 编码进 request-id 随波携带）
    Ok(vec![http_get(
        &format!("prov-list|{req_id}|{city}|{province}|{range}"),
        &format!("{SOURCE}/rest/province/all"),
    )])
}

// ============================================================
// 回灌：http.responses（数组 [{id,status,body,error}]，按步批量）
// ============================================================

fn on_responses(payload: &str) -> Result<Vec<types::Effect>, String> {
    let batch: Vec<Value> =
        serde_json::from_str(payload).map_err(|e| format!("回灌载荷非法：{e}"))?;
    let first_id = batch
        .first()
        .and_then(|f| f.get("id"))
        .and_then(Value::as_str)
        .unwrap_or_default();

    if first_id.starts_with("prov-list|") {
        // 省列表到达 → 一步并发拉取全部省的城市列表（批量的典型用法）
        let parts: Vec<&str> = first_id.split('|').collect();
        let (Some(req_id), Some(city), province, range) =
            (parts.get(1), parts.get(2), parts.get(3), parts.get(4))
        else {
            return Err(format!("prov-list id 上下文残缺：{first_id}"));
        };
        let (req_id, city, province, range) = (
            *req_id,
            *city,
            province.copied().unwrap_or(""),
            range.copied().unwrap_or(""),
        );
        let mut effects = Vec::new();
        for f in &batch {
            let body = f.get("body").and_then(Value::as_str).unwrap_or("[]");
            if let Ok(provinces) = serde_json::from_str::<Value>(body) {
                if let Some(arr) = provinces.as_array() {
                    for p in arr {
                        if let Some(code) = p.get("code").and_then(Value::as_str) {
                            effects.push(http_get(
                                &format!("prov|{code}|{req_id}|{city}|{province}|{range}"),
                                &format!("{SOURCE}/rest/province/{code}"),
                            ));
                        }
                    }
                }
            }
        }
        if effects.is_empty() {
            return Ok(finish_fail(req_id, "天气服务暂时不可用（城市列表为空）"));
        }
        return Ok(effects);
    }

    if first_id.starts_with("prov|") {
        // 全省城市列表到达 → 匹配站点码（成功：缓存站点码 + 拉天气；失败：明确报错）
        let parts: Vec<&str> = first_id.split('|').collect();
        let (Some(req_id), Some(city), province, range) =
            (parts.get(2), parts.get(3), parts.get(4), parts.get(5))
        else {
            return Err(format!("prov id 上下文残缺：{first_id}"));
        };
        let (req_id, city, province, range) = (
            *req_id,
            *city,
            province.copied().unwrap_or(""),
            range.copied().unwrap_or(""),
        );
        let mut list: Vec<(String, String, String)> = Vec::new();
        for f in &batch {
            let body = f.get("body").and_then(Value::as_str).unwrap_or("[]");
            if let Ok(cities) = serde_json::from_str::<Value>(body) {
                if let Some(arr) = cities.as_array() {
                    for c in arr {
                        let cname = c.get("city").and_then(Value::as_str).unwrap_or_default();
                        let ccode = c.get("code").and_then(Value::as_str).unwrap_or_default();
                        let cprov = c
                            .get("province")
                            .and_then(Value::as_str)
                            .unwrap_or_default();
                        if !cname.is_empty() && !ccode.is_empty() {
                            list.push((cname.to_string(), ccode.to_string(), cprov.to_string()));
                        }
                    }
                }
            }
        }
        return Ok(match parse::match_station(&list, city, province) {
            Some(code) => vec![
                kv_put(&station_key(city), code.as_bytes()),
                http_get(&wx_id(&code, req_id, city, province, range), &wx_url(&code)),
            ],
            None => finish_fail(
                req_id,
                &format!("未找到城市 {city}，请确认城市名称或补充省份"),
            ),
        });
    }

    if first_id.starts_with("wx|") {
        // 天气数据到达 → 组装结果（同原生 forecast ⑤）+ 写缓存 + 交付
        let parts: Vec<&str> = first_id.split('|').collect();
        let (Some(req_id), Some(city), Some(code)) = (parts.get(2), parts.get(3), parts.get(1))
        else {
            return Err(format!("wx id 上下文残缺：{first_id}"));
        };
        let (req_id, city, code) = (*req_id, *city, *code);
        let province = parts.get(4).copied().unwrap_or("");
        let range = parts.get(5).copied().unwrap_or("");
        let body = batch
            .first()
            .and_then(|f| f.get("body"))
            .and_then(Value::as_str)
            .unwrap_or("null");
        let data: Value = match serde_json::from_str::<Value>(body) {
            Ok(v) if v.get("code").and_then(Value::as_i64) == Some(0) => {
                v.get("data").cloned().unwrap_or(Value::Null)
            }
            _ => return Ok(finish_fail(req_id, "天气服务暂时不可用，请稍后再试")),
        };
        let (result, publish_time) = build_result(city, province, code, &range, &data);
        let _ = publish_time;
        let mut effects = finish_result(req_id, &result, Some(&cache_key(city, range)));
        effects.push(kv_put(
            &weather_data_key(code),
            json!({"ts": host::now_ms(), "data": data})
                .to_string()
                .as_bytes(),
        ));
        return Ok(effects);
    }

    Err(format!("无法识别的回灌 id：{first_id}"))
}

// ============================================================
// 结果组装（同原生 forecast ⑤；range 归一化/实时/单日/7天分支）
// ============================================================

fn build_result(
    city: &str,
    province: &str,
    code: &str,
    range: &str,
    data: &Value,
) -> (Value, String) {
    let is_7day = ["一周", "七天", "7天", "未来"]
        .iter()
        .any(|w| range.contains(w));
    let is_tomorrow = range.contains("明天");
    let is_dayafter = range.contains("后天");

    let (realtime, forecast_items, hourly) = if is_7day {
        (Value::Null, parse::parse_forecast(data, 7), Value::Null)
    } else if is_tomorrow {
        (Value::Null, parse::parse_forecast_at(data, 1), Value::Null)
    } else if is_dayafter {
        (Value::Null, parse::parse_forecast_at(data, 2), Value::Null)
    } else {
        let hr = data
            .get("passedchart")
            .and_then(Value::as_array)
            .map(|arr| json!(arr.iter().take(6).cloned().collect::<Vec<_>>()))
            .unwrap_or(Value::Null);
        (parse::parse_realtime(data), Value::Null, hr)
    };

    let range_norm: &str = if is_7day {
        "近7天"
    } else if is_tomorrow {
        "明天"
    } else if is_dayafter {
        "后天"
    } else if range.contains("当前") || range.contains("实时") {
        "当前"
    } else if range.contains("今天") {
        "今天"
    } else {
        "实时"
    };

    let publish_time = data
        .get("real")
        .and_then(|r| r.get("publish_time"))
        .and_then(parse::clean)
        .unwrap_or_default();
    let speech = parse::summarize(city, range_norm, &realtime, forecast_items.as_array());
    let result = json!({
        "ok": true,
        "source": "nmc",
        "location": {"province": province, "city": city, "code": code},
        "range": range_norm,
        "realtime": realtime,
        "forecast": forecast_items,
        "hourly": hourly,
        "publish_time": publish_time,
        "speech": speech,
        "update_time": publish_time,
    });
    (result, publish_time)
}

// ============================================================
// 效果构造小件
// ============================================================

fn finish_result(req_id: &str, result: &Value, cache_key: Option<&str>) -> Vec<types::Effect> {
    let mut effects = vec![kv_put(
        &format!("result:{req_id}"),
        result.to_string().as_bytes(),
    )];
    if let Some(key) = cache_key {
        let envelope = json!({"ts": host::now_ms(), "result": result});
        effects.push(kv_put(key, envelope.to_string().as_bytes()));
    }
    effects.push(types::Effect::Log(types::EffectLog {
        level: "info".to_string(),
        message: format!("weather.forecast 完成（req_id={req_id}）"),
    }));
    effects
}

fn finish_fail(req_id: &str, speech: &str) -> Vec<types::Effect> {
    // 失败也走结果通道（同原生 forecast：永远成功形态，ok:false + 话术）
    let result = json!({"ok": false, "speech": speech});
    vec![
        kv_put(&format!("result:{req_id}"), result.to_string().as_bytes()),
        types::Effect::Log(types::EffectLog {
            level: "warn".to_string(),
            message: format!("weather.forecast 失败（req_id={req_id}）：{speech}"),
        }),
    ]
}

fn http_get(id: &str, url: &str) -> types::Effect {
    types::Effect::HttpGet(types::EffectHttpGet {
        request_id: id.to_string(),
        url: url.to_string(),
    })
}

fn kv_put(key: &str, value: &[u8]) -> types::Effect {
    types::Effect::KvPut(types::EffectKvPut {
        key: key.to_string(),
        value: value.to_vec(),
    })
}

fn cache_key(city: &str, range: &str) -> String {
    format!("wx:{city}:{range}")
}

fn weather_data_key(code: &str) -> String {
    format!("weather-data:{code}")
}

fn cached_weather_data(code: &str) -> Option<Value> {
    let bytes = host::kv_get(&weather_data_key(code))?;
    let cached: Value = serde_json::from_slice(&bytes).ok()?;
    let ts = cached.get("ts").and_then(Value::as_u64)?;
    if host::now_ms().saturating_sub(ts) >= CACHE_TTL_MS {
        return None;
    }
    cached.get("data").cloned()
}

fn station_key(city: &str) -> String {
    format!("station:{city}")
}

fn wx_id(code: &str, req_id: &str, city: &str, province: &str, range: &str) -> String {
    format!("wx|{code}|{req_id}|{city}|{province}|{range}")
}

fn wx_url(code: &str) -> String {
    format!("{SOURCE}/rest/weather?stationid={code}")
}

xiaozhi_plugin_sdk::export!(Component);
