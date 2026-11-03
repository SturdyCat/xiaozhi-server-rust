//! 天气解析纯函数的离线测试（不联网）。

use super::parse::{
    extract_time, match_station, normalize_name, parse_forecast, parse_realtime,
    strip_weather_words,
};
use serde_json::{json, Value};

/// 对齐真实 NMC 形状的写死 `data` 样本。
fn sample_data() -> Value {
    json!({
        "real": {
            "publish_time": "2024-01-01 12:00",
            "weather": {
                "temperature": "12",
                "info": "晴",
                "humidity": "40",
                "feelslike": "10",
                "airpressure": "1013"
            },
            "wind": { "direct": "东南风", "power": "3级", "speed": "2", "degree": "135" }
        },
        "predict": {
            "detail": [
                { "date": "2024-01-01", "day": { "info": "晴", "temperature": "15", "wind": { "direct": "东南风", "power": "3级" } }, "night": { "info": "多云", "temperature": "4", "wind": { "direct": "北风", "power": "2级" } } },
                { "date": "2024-01-02", "day": { "info": "阴", "temperature": "13", "wind": { "direct": "东风", "power": "2级" } }, "night": { "info": "小雨", "temperature": "5", "wind": { "direct": "北风", "power": "3级" } } }
            ]
        },
        "passedchart": [
            { "time": "12:00", "temperature": "12", "weather": "晴" },
            { "time": "13:00", "temperature": "13", "weather": "晴" }
        ]
    })
}

#[test]
fn test_parse_realtime_fields() {
    let d = sample_data();
    let rt = parse_realtime(&d);
    assert_eq!(rt.get("temp_c").and_then(Value::as_str), Some("12"));
    assert_eq!(rt.get("weather").and_then(Value::as_str), Some("晴"));
    assert_eq!(rt.get("humidity").and_then(Value::as_str), Some("40"));
    assert_eq!(rt.get("feels_like").and_then(Value::as_str), Some("10"));
    assert_eq!(rt.get("wind_dir").and_then(Value::as_str), Some("东南风"));
    assert_eq!(rt.get("wind_power").and_then(Value::as_str), Some("3级"));
}

#[test]
fn test_parse_forecast_days_and_missing() {
    let d = sample_data();
    let f = parse_forecast(&d, 7);
    let arr = f.as_array().unwrap();
    // 样本仅 2 项，take(7) 取满也只 2 项；验证 min(2,7)=2。
    assert_eq!(arr.len(), 2);
    assert!(arr[0].get("day").is_some());

    // 缺字段样本：day 无 temperature → 键存在但为 null，不 panic。
    let sparse = json!({ "predict": { "detail": [ { "date": "x", "day": { "info": "晴" } } ] } });
    let f2 = parse_forecast(&sparse, 7);
    let arr2 = f2.as_array().unwrap();
    assert_eq!(arr2.len(), 1);
    assert!(arr2[0].get("day").unwrap().get("temp_c").unwrap().is_null());
}

#[test]
fn test_parse_realtime_empty_no_panic() {
    let d = json!({});
    let rt = parse_realtime(&d);
    // 键存在但值为 null（构造即含全部键），不 panic。
    assert!(rt.get("temp_c").unwrap().is_null());
    assert!(rt.get("weather").unwrap().is_null());
    let f = parse_forecast(&d, 7);
    assert!(f.as_array().unwrap().is_empty());
}

#[test]
fn test_normalize_name() {
    assert_eq!(normalize_name("北京市"), "北京");
    assert_eq!(normalize_name("广东省"), "广东");
    assert_eq!(normalize_name("深圳市"), "深圳");
    assert_eq!(normalize_name("新疆维吾尔自治区"), "新疆");
    assert_eq!(normalize_name("内蒙古自治区"), "内蒙古");
    assert_eq!(normalize_name("广西壮族自治区"), "广西");
    assert_eq!(normalize_name("延边朝鲜族自治州"), "延边朝鲜族");
    assert_eq!(normalize_name(" 杭州市 "), "杭州");
}

#[test]
fn test_extract_time() {
    assert_eq!(
        extract_time("北京今天", ""),
        ("北京".to_string(), "今天".to_string())
    );
    assert_eq!(
        extract_time("北京", "明天"),
        ("北京".to_string(), "明天".to_string())
    );
    // 长词优先：未来一周 不应被 未来 截断
    assert_eq!(
        extract_time("上海未来一周", ""),
        ("上海".to_string(), "未来一周".to_string())
    );
    assert_eq!(extract_time("深圳", ""), ("深圳".to_string(), String::new()));
}

#[test]
fn test_strip_weather_words_pipeline() {
    // 「未来一周天气」这类组合：先清洗非地名词，再切时段 → 城市为空（走默认定位）。
    let cleaned = strip_weather_words("未来一周天气");
    assert_eq!(extract_time(&cleaned, ""), (String::new(), "未来一周".to_string()));
    // 「深圳今天天气」→ 深圳 + 今天
    let cleaned = strip_weather_words("深圳今天天气");
    assert_eq!(
        extract_time(&cleaned, ""),
        ("深圳".to_string(), "今天".to_string())
    );
    // 纯城市不受影响
    assert_eq!(strip_weather_words("深圳市"), "深圳市");
}

#[test]
fn test_match_station_with_province_filter() {
    let list = vec![
        ("同名市".to_string(), "A1".to_string(), "广东".to_string()),
        ("同名市".to_string(), "B2".to_string(), "江苏".to_string()),
        ("深圳".to_string(), "C3".to_string(), "广东".to_string()),
    ];
    // 无省：取首个同名
    assert_eq!(match_station(&list, "同名市", "").unwrap(), "A1");
    // 指定省：过滤
    assert_eq!(match_station(&list, "同名市", "江苏").unwrap(), "B2");
    // 普通匹配
    assert_eq!(match_station(&list, "深圳市", "广东").unwrap(), "C3");
    // 归一化匹配（带后缀）
    assert_eq!(match_station(&list, "广东省深圳", "").unwrap(), "C3");
    // 未找到
    assert!(match_station(&list, "不存在市", "").is_none());
}
