use super::*;
use crate::skills::schedule::{duration_ms, Schedule};

fn ms(text: &str) -> i64 {
    chrono::DateTime::parse_from_rfc3339(text)
        .unwrap()
        .timestamp_millis()
}

#[test]
fn natural_durations_are_explicit_positive_and_bounded() {
    for (text, expected) in [
        ("五分钟", 300_000),
        ("10分钟", 600_000),
        ("半小时", 1_800_000),
        ("一小时", 3_600_000),
        ("每隔两小时", 7_200_000),
        ("every 5 minutes", 300_000),
        ("10分钟之后", 600_000),
        ("一百零五秒", 105_000),
    ] {
        assert_eq!(duration_ms(text).unwrap(), expected, "{text}");
    }
    for text in [
        "",
        "五",
        "0秒",
        "-1分钟",
        "1.5小时",
        "一分半",
        "明天",
        "每周一",
        "367天",
        "一二分钟",
        "十十分钟",
    ] {
        assert!(duration_ms(text).is_err(), "{text}");
    }
}

#[test]
fn after_and_interval_compute_from_injected_clock_without_sleep() {
    let (once, next) = Schedule::parse(&json!({"after":"五分钟"}), NOW).unwrap();
    assert_eq!(next, NOW + 300_000);
    assert!(once.next_after(next, NOW + 900_000).unwrap().is_none());
    let (repeat, next) = Schedule::parse(&json!({"interval":"一小时"}), NOW).unwrap();
    assert_eq!(next, NOW + 3_600_000);
    assert_eq!(
        repeat.next_after(next, NOW + 100 * 3_600_000).unwrap(),
        Some(NOW + 101 * 3_600_000)
    );
}

#[test]
fn rfc3339_and_explicit_today_tomorrow_use_requested_timezone() {
    let now = ms("2026-11-01T00:00:00Z");
    let (_, at) = Schedule::parse(&json!({"at":"2026-11-02T08:00:00+08:00"}), now).unwrap();
    assert_eq!(at, ms("2026-11-02T00:00:00Z"));
    let (_, at) =
        Schedule::parse(&json!({"at":"明天八点","timezone":"Asia/Shanghai"}), now).unwrap();
    assert_eq!(at, ms("2026-11-02T00:00:00Z"));
    let (_, at) = Schedule::parse(
        &json!({"at":"今天下午三点半","timezone":"Asia/Shanghai"}),
        now,
    )
    .unwrap();
    assert_eq!(at, ms("2026-11-01T07:30:00Z"));
    let (_, at) = Schedule::parse(&json!({"at":"明天08:30","timezone":"UTC"}), now).unwrap();
    assert_eq!(at, ms("2026-11-02T08:30:00Z"));
}

#[test]
fn ambiguous_past_invalid_and_unsupported_times_are_rejected() {
    let now = ms("2026-11-01T10:00:00Z");
    for args in [
        json!({"at":"周一八点"}),
        json!({"at":"今天八点"}),
        json!({"at":"2026-01-01T00:00:00Z"}),
        json!({"daily":"25:00"}),
        json!({"daily":"08:60"}),
        json!({"daily":"08:00","timezone":"America/New_York"}),
        json!({"daily":"晚上十二点"}),
        json!({"daily":"早上十二点"}),
        json!({"daily":"上午十二点"}),
        json!({"daily":"下午十二点"}),
        json!({"at":"明天晚上十二点"}),
    ] {
        assert!(Schedule::parse(&args, now).is_err(), "{args}");
    }
}

#[test]
fn daily_utc_and_shanghai_coalesce_to_next_calendar_occurrence() {
    let now = ms("2026-11-01T00:00:00Z");
    let (daily, next) =
        Schedule::parse(&json!({"daily":"08:00","timezone":"Asia/Shanghai"}), now).unwrap();
    assert_eq!(next, ms("2026-11-02T00:00:00Z"));
    assert_eq!(
        daily.next_after(next, ms("2026-11-09T01:00:00Z")).unwrap(),
        Some(ms("2026-11-10T00:00:00Z"))
    );
    let (_, next) = Schedule::parse(&json!({"daily":"八点","timezone":"UTC"}), now).unwrap();
    assert_eq!(next, ms("2026-11-01T08:00:00Z"));
    let (_, next) = Schedule::parse(
        &json!({"daily":"晚上八点半","timezone":"Asia/Shanghai"}),
        now,
    )
    .unwrap();
    assert_eq!(next, ms("2026-11-01T12:30:00Z"));
    let (_, next) = Schedule::parse(&json!({"daily":"中午十二点","timezone":"UTC"}), now).unwrap();
    assert_eq!(next, ms("2026-11-01T12:00:00Z"));
}
