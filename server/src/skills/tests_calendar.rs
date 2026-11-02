use super::*;
use crate::skills::schedule::Schedule;

fn ms(text: &str) -> i64 {
    chrono::DateTime::parse_from_rfc3339(text)
        .unwrap()
        .timestamp_millis()
}
fn next(args: Value, now: &str) -> i64 {
    Schedule::parse(&args, ms(now)).unwrap().1
}

#[test]
fn explicit_day_offsets_cross_month_year_and_leap_days() {
    let now = "2027-12-31T00:00:00+08:00";
    for (at, expected) in [
        ("今天上午10", "2027-12-31T10:00:00+08:00"),
        ("明天下午3", "2028-01-01T15:00:00+08:00"),
        ("后天晚上8", "2028-01-02T20:00:00+08:00"),
        ("大后天上午十点", "2028-01-03T10:00:00+08:00"),
    ] {
        assert_eq!(
            next(json!({"at":at,"timezone":"Asia/Shanghai"}), now),
            ms(expected)
        );
    }
    assert_eq!(
        next(
            json!({"at":"后天八点","timezone":"Asia/Shanghai"}),
            "2028-02-28T09:00:00+08:00"
        ),
        ms("2028-03-01T08:00:00+08:00")
    );
}

#[test]
fn bare_clocks_roll_forward_but_past_explicit_today_does_not() {
    let now = "2028-01-31T09:00:00+08:00";
    assert_eq!(
        next(json!({"at":"八点","timezone":"Asia/Shanghai"}), now),
        ms("2028-02-01T08:00:00+08:00")
    );
    assert_eq!(
        next(json!({"at":"下午三点","timezone":"Asia/Shanghai"}), now),
        ms("2028-01-31T15:00:00+08:00")
    );
    assert_eq!(
        next(json!({"at":"09:00","timezone":"Asia/Shanghai"}), now),
        ms("2028-02-01T09:00:00+08:00")
    );
    assert!(Schedule::parse(
        &json!({"at":"今天八点","timezone":"Asia/Shanghai"}),
        ms(now)
    )
    .is_err());
}

#[test]
fn hourly_is_wall_clock_minute_not_an_interval_since_creation() {
    assert_eq!(
        next(
            json!({"hourly":"二十分","timezone":"Asia/Shanghai"}),
            "2028-01-31T23:21:00+08:00"
        ),
        ms("2028-02-01T00:20:00+08:00")
    );
    assert_eq!(
        next(json!({"hourly":20}), "2028-01-01T10:19:59Z"),
        ms("2028-01-01T10:20:00Z")
    );
    assert_eq!(
        next(json!({"hourly":"20分"}), "2028-01-01T10:20:00Z"),
        ms("2028-01-01T11:20:00Z")
    );
}

#[test]
fn workdays_mean_monday_to_friday_and_weekly_crosses_year() {
    assert_eq!(
        next(
            json!({"workdays":"08:00","timezone":"Asia/Shanghai"}),
            "2027-12-31T09:00:00+08:00"
        ),
        ms("2028-01-03T08:00:00+08:00")
    );
    assert_eq!(
        next(
            json!({"weekly":{"weekday":"周一","time":"十点"},"timezone":"Asia/Shanghai"}),
            "2027-12-31T09:00:00+08:00"
        ),
        ms("2028-01-03T10:00:00+08:00")
    );
    assert_eq!(
        next(
            json!({"weekly":{"weekday":"日","time":"08:00"}}),
            "2028-01-02T08:00:00Z"
        ),
        ms("2028-01-09T08:00:00Z")
    );
}

#[test]
fn monthly_skips_short_months_and_yearly_skips_nonleap_centuries() {
    assert_eq!(
        next(
            json!({"monthly":{"day":"31","time":"08:00"}}),
            "2028-01-31T09:00:00Z"
        ),
        ms("2028-03-31T08:00:00Z")
    );
    assert_eq!(
        next(
            json!({"monthly":{"day":"二十九号","time":"08:00"}}),
            "2028-01-31T09:00:00Z"
        ),
        ms("2028-02-29T08:00:00Z")
    );
    assert_eq!(
        next(
            json!({"yearly":{"month":"二月","day":"29日","time":"08:00"}}),
            "2028-03-01T09:00:00Z"
        ),
        ms("2032-02-29T08:00:00Z")
    );
    assert_eq!(
        next(
            json!({"yearly":{"month":2,"day":29,"time":"08:00"}}),
            "2096-02-29T09:00:00Z"
        ),
        ms("2104-02-29T08:00:00Z")
    );
}

#[test]
fn calendar_invalid_fields_and_unsupported_shapes_fail_closed() {
    for args in [
        json!({"hourly":60}),
        json!({"hourly":-1}),
        json!({"weekly":{"weekday":0,"time":"08:00"}}),
        json!({"weekly":{"weekday":8,"time":"08:00"}}),
        json!({"monthly":{"day":32,"time":"08:00"}}),
        json!({"yearly":{"month":2,"day":30,"time":"08:00"}}),
        json!({"yearly":{"month":13,"day":1,"time":"08:00"}}),
    ] {
        assert!(Schedule::parse(&args, NOW).is_err(), "{args}");
    }
    for args in [
        json!({"text":"x","weekly":{"weekday":1,"time":"08:00","device_id":"evil"}}),
        json!({"text":"x","yearly":{"month":2,"time":"08:00"}}),
        json!({"text":"x","hourly":20,"daily":"08:00"}),
    ] {
        assert!(validate_step(&SkillStep {
            api: "tasks.create".into(),
            args
        })
        .is_err());
    }
}

#[test]
fn calendar_persists_restarts_and_coalesces_without_offline_flood() {
    let db = TestDb::new();
    let args =
        json!({"text":"月末总结","monthly":{"day":31,"time":"08:00"},"timezone":"Asia/Shanghai"});
    let start = ms("2028-01-31T09:00:00+08:00");
    let id = create(&db.system, &ctx("a", "monthly", start), args);
    let restarted = SkillSystem::new(db.system.path.clone());
    assert_eq!(
        restarted
            .tick_due(ms("2028-03-31T08:00:00+08:00"))
            .unwrap()
            .materialized,
        1
    );
    assert_eq!(
        restarted
            .tick_due(ms("2029-06-01T00:00:00+08:00"))
            .unwrap()
            .materialized,
        0
    );
    assert_eq!(pending_count(&restarted), 1);
    assert_eq!(
        restarted.get_task(&id, None).unwrap().unwrap()["next_due_ms"],
        ms("2029-07-31T08:00:00+08:00")
    );
}
