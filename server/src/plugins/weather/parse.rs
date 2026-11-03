//! NMC 返回数据的宽松解析与归一化（纯函数，零 panic，可离线测试）。
//!
//! 字段路径对齐真实接口：`data.real.weather|wind`（实时）、`data.predict.detail`
//! （7 天预报）、`data.passedchart`（逐小时）。所有字段缺失一律 `null`，绝不 panic。

use serde_json::{json, Value};

/// 从 NMC `data.real` 提取实时天气（宽松，缺字段为 null）。
pub(super) fn parse_realtime(data: &Value) -> Value {
    let real = data.get("real");
    let weather = real.and_then(|r| r.get("weather"));
    let wind = real.and_then(|r| r.get("wind"));
    let feels = weather
        .and_then(|w| w.get("feelslike"))
        .or_else(|| weather.and_then(|w| w.get("feelst")));
    json!({
        "temp_c": weather.and_then(|w| w.get("temperature")).and_then(clean),
        "weather": weather.and_then(|w| w.get("info")).and_then(clean),
        "humidity": weather.and_then(|w| w.get("humidity")).and_then(clean),
        "feels_like": feels.and_then(clean),
        "wind_dir": wind.and_then(|w| w.get("direct")).and_then(clean),
        "wind_power": wind.and_then(|w| w.get("power")).and_then(clean),
        "airpressure": weather.and_then(|w| w.get("airpressure")).and_then(clean),
        "update_time": real.and_then(|r| r.get("publish_time")).and_then(clean),
    })
}

/// 从 NMC `data.predict.detail` 取前 `days` 项预报（宽松，缺字段为 null）。
pub(super) fn parse_forecast(data: &Value, days: usize) -> Value {
    let items = data
        .get("predict")
        .and_then(|p| p.get("detail"))
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .take(days)
                .map(build_forecast_item)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    json!(items)
}

/// 取 `data.predict.detail[index]`（不足则取末项），返回单元素数组（同 7 天形状）。
pub(super) fn parse_forecast_at(data: &Value, index: usize) -> Value {
    let items = data
        .get("predict")
        .and_then(|p| p.get("detail"))
        .and_then(Value::as_array)
        .map(|arr| {
            if arr.is_empty() {
                Vec::new()
            } else {
                let idx = index.min(arr.len() - 1);
                vec![build_forecast_item(&arr[idx])]
            }
        })
        .unwrap_or_default();
    json!(items)
}

/// 构造单条 `predict.detail` 项的归一化结构（day/night 各含天气、温度、风向、风力）。
fn build_forecast_item(d: &Value) -> Value {
    let day = d.get("day");
    let night = d.get("night");
    let day_wind = day.and_then(|x| x.get("wind"));
    let night_wind = night.and_then(|x| x.get("wind"));
    json!({
        "date": d.get("date").and_then(clean),
        "day": {
            "weather": day.and_then(|x| x.get("info")).and_then(clean),
            "temp_c": day.and_then(|x| x.get("temperature")).and_then(clean),
            "wind_dir": day_wind.and_then(|x| x.get("direct")).and_then(clean),
            "wind_power": day_wind.and_then(|x| x.get("power")).and_then(clean),
        },
        "night": {
            "weather": night.and_then(|x| x.get("info")).and_then(clean),
            "temp_c": night.and_then(|x| x.get("temperature")).and_then(clean),
            "wind_dir": night_wind.and_then(|x| x.get("direct")).and_then(clean),
            "wind_power": night_wind.and_then(|x| x.get("power")).and_then(clean),
        },
    })
}

/// 把 NMC 字段清洗为可选字符串：null/"None"/"-"/"" → None，其余转字符串。
pub(super) fn clean(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => {
            let t = s.trim();
            if t.is_empty() || t == "None" || t == "-" || t.eq_ignore_ascii_case("null") {
                None
            } else {
                Some(t.to_string())
            }
        }
        Value::Null => None,
        other => Some(other.to_string()),
    }
}

/// 地名归一化：去空白并循环剥掉省/市/县/区/自治区等后缀，直到不能剥。
pub(super) fn normalize_name(s: &str) -> String {
    // 长后缀先匹配，避免「新疆维吾尔自治区」被「自治区」提前截断成「新疆维吾尔」。
    let suffixes = [
        "维吾尔自治区",
        "壮族自治区",
        "回族自治区",
        "特别行政区",
        "自治州",
        "自治区",
        "地区",
        "盟",
        "省",
        "市",
        "县",
        "区",
    ];
    let mut result = s.trim().to_string();
    loop {
        let before = result.len();
        for suf in suffixes {
            if let Some(stripped) = result.strip_suffix(suf) {
                if !stripped.is_empty() {
                    result = stripped.trim().to_string();
                    break;
                }
            }
        }
        if result.len() == before {
            break;
        }
    }
    result
}

/// 清洗城市串里的非地名词（天气/预报/气温/温度等，中文城市名不含这些词）。
pub(super) fn strip_weather_words(city: &str) -> String {
    let mut s = city.to_string();
    for w in ["天气", "预报", "气温", "温度", "情况", "怎么样"] {
        s = s.replace(w, "");
    }
    s.trim().to_string()
}

/// 从文本中切出时段关键词：返回 (清理后的城市, 时段)。命中则把该词从 city/range
/// 移出并作为有效 range；未命中 range 保持原参数。
///
/// 意图模板受 `catalog::validate_template` 约束不允许两个开放槽位直接相邻，因此
/// 「北京今天天气」这类组合只会以单槽（city 内嵌时段词）形式到达，这里负责二次切分。
/// 调用方应先用 [`strip_weather_words`] 清洗城市串再传入。
pub(super) fn extract_time(city: &str, range: &str) -> (String, String) {
    // 长词先匹配，避免「未来一周」被「未来」提前命中。
    let time_words = [
        "未来一周", "近7天", "近七天", "未来", "一周", "七天", "7天", "实时", "当前", "今天",
        "明天", "后天",
    ];
    let combined = format!("{}{}", range, city);
    for tw in time_words {
        if combined.contains(tw) {
            let mut c = city.to_string();
            let mut r = range.to_string();
            if c.contains(tw) {
                c = c.replace(tw, "");
            } else if r.contains(tw) {
                r = r.replace(tw, "");
            }
            return (c.trim().to_string(), tw.to_string());
        }
    }
    (city.to_string(), String::new())
}

/// 在候选列表中按归一化名匹配城市；`province` 非空时优先按省过滤同名城市。
///
/// 两级匹配：① 归一化名精确相等；② 省市连写回退——输入以归一化城市名**结尾**
/// （如「广东深圳」「广东省深圳」→「深圳」），要求城市名至少 2 字防单字误命中。
pub(super) fn match_station(
    list: &[(String, String, String)],
    city: &str,
    province: &str,
) -> Option<String> {
    let cn = normalize_name(city);
    let exact: Vec<(String, String)> = list
        .iter()
        .filter(|(c, _, _)| normalize_name(c) == cn)
        .map(|(c, code, p)| (code.clone(), p.clone()))
        .collect();
    let mut matched = exact;
    if matched.is_empty() && cn.chars().count() > 2 {
        matched = list
            .iter()
            .filter(|(c, _, _)| {
                let n = normalize_name(c);
                n.chars().count() >= 2 && cn.ends_with(&n)
            })
            .map(|(c, code, p)| (code.clone(), p.clone()))
            .collect();
    }
    if matched.is_empty() {
        return None;
    }
    if !province.is_empty() {
        let pn = normalize_name(province);
        let filtered: Vec<(String, String)> = matched
            .iter()
            .filter(|(_, p)| normalize_name(p) == pn)
            .cloned()
            .collect();
        if !filtered.is_empty() {
            matched = filtered;
        }
    }
    matched.first().map(|(code, _)| code.clone())
}

/// 生成中文语音摘要（实时 + 预报通用）。
pub(super) fn summarize(
    city: &str,
    range_norm: &str,
    realtime: &Value,
    forecast: Option<&Vec<Value>>,
) -> String {
    let mut s = String::new();
    if let Some(obj) = realtime.as_object() {
        let weather = obj.get("weather").and_then(Value::as_str).unwrap_or("");
        let temp = obj.get("temp_c").and_then(Value::as_str).unwrap_or("");
        let wind_dir = obj.get("wind_dir").and_then(Value::as_str).unwrap_or("");
        let wind_power = obj.get("wind_power").and_then(Value::as_str).unwrap_or("");
        let humidity = obj.get("humidity").and_then(Value::as_str).unwrap_or("");
        s.push_str(&format!(
            "{}当前{}，{}℃，{}{}风，湿度{}%。",
            city, weather, temp, wind_dir, wind_power, humidity
        ));
    }
    if let Some(items) = forecast {
        if !items.is_empty() {
            if range_norm == "近7天" {
                s.push_str(&format!("{}未来7天：", city));
            } else {
                s.push_str(&format!("{}未来：", city));
            }
            for item in items {
                let date = item.get("date").and_then(Value::as_str).unwrap_or("");
                let day_w = item
                    .get("day")
                    .and_then(|x| x.get("weather"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let day_t = item
                    .get("day")
                    .and_then(|x| x.get("temp_c"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let night_t = item
                    .get("night")
                    .and_then(|x| x.get("temp_c"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                s.push_str(&format!("{} {} {}/{}℃；", date, day_w, day_t, night_t));
            }
        }
    }
    if s.is_empty() {
        s.push_str("暂无可用的天气数据。");
    }
    s
}
