//! Expanded clock contracts over real SQLite and HTTP. No model/audio execution or wall-clock sleeps.
use super::*;
use chrono::{Datelike, TimeZone, Timelike, Utc};

fn at(value: &str) -> i64 {
    chrono::DateTime::parse_from_rfc3339(value)
        .unwrap()
        .timestamp_millis()
}
fn fixed_context(device: &str, now_ms: i64) -> SkillExecutionContext {
    let mut ctx = context(device.into(), uuid::Uuid::new_v4().to_string());
    ctx.now_ms = now_ms;
    ctx
}
async fn created(client: &reqwest::Client, base: &str, device: &str, args: Value) -> Value {
    let response = post(
        client,
        base,
        "/api/skill-tasks",
        json!({"device_id":device,"request_id":uuid::Uuid::new_v4().to_string(),"args":args}),
    )
    .await;
    let status = response.status();
    let value: Value = response.json().await.unwrap();
    assert_eq!(status, StatusCode::OK, "{value}");
    assert_eq!(value["task"]["device_id"], device);
    assert_eq!(value["task"]["played"], Value::Null);
    assert_eq!(value["task"]["delivery_state"], "none");
    assert_eq!(value["played"], false);
    assert_eq!(value["socket_written"], false);
    assert!(!value["speech"].as_str().unwrap().trim().is_empty());
    value
}
async fn loaded(client: &reqwest::Client, base: &str, id: &str) -> Value {
    let response = get(client, base, &format!("/api/skill-tasks/{id}")).await;
    assert_eq!(response.status(), StatusCode::OK);
    response.json::<Value>().await.unwrap()["task"].clone()
}

#[tokio::test]
async fn http_create_accepts_all_calendar_schedules_and_preserves_metadata() {
    let (base, _server, engines) = fixture(config()).await;
    let client = reqwest::Client::new();
    for args in [
        json!({"text":"非法分钟","hourly":60}),
        json!({"text":"非法星期","weekly":{"weekday":0,"time":"08:00"}}),
        json!({"text":"非法月日","monthly":{"day":0,"time":"08:00"}}),
        json!({"text":"永不存在日期","yearly":{"month":2,"day":30,"time":"08:00"}}),
        json!({"text":"不能覆盖时钟","daily":"08:00","now_ms":0}),
        json!({"text":"两个频次","daily":"08:00","hourly":20}),
    ] {
        assert_eq!(
            post(
                &client,
                &base,
                "/api/skill-tasks",
                json!({"device_id":"calendar-device","args":args})
            )
            .await
            .status(),
            StatusCode::BAD_REQUEST
        );
    }
    let unclear = post(
        &client,
        &base,
        "/api/skill-tasks",
        json!({"device_id":"calendar-device","args":{"text":"需澄清","at":"不明确的时间"}}),
    )
    .await;
    assert_eq!(unclear.status(), StatusCode::BAD_REQUEST);
    let clarification: Value = unclear.json().await.unwrap();
    assert_eq!(clarification["error_code"], "clock_clarification");
    assert!(!clarification["speech"].as_str().unwrap().is_empty());
    let cases = [
        json!({"text":"每小时提醒","hourly":"20分","timezone":"Asia/Shanghai"}),
        json!({"text":"每日提醒","daily":"08:15","timezone":"Asia/Shanghai"}),
        json!({"text":"工作日提醒","workdays":"08:15","timezone":"Asia/Shanghai"}),
        json!({"text":"每周提醒","weekly":{"weekday":"周一","time":"10点"},"timezone":"Asia/Shanghai"}),
        json!({"text":"每月提醒","monthly":{"day":"三十一","time":"08:15"},"timezone":"Asia/Shanghai"}),
        json!({"text":"每年提醒","yearly":{"month":2,"day":29,"time":"08:15"},"timezone":"Asia/Shanghai"}),
    ];
    let tz = chrono::FixedOffset::east_opt(8 * 3600).unwrap();
    for (index, args) in cases.into_iter().enumerate() {
        let before = Utc::now().timestamp_millis();
        let value = created(&client, &base, "calendar-device", args).await;
        let due_ms = value["task"]["next_due_ms"].as_i64().unwrap();
        assert!(due_ms > before);
        let due = Utc
            .timestamp_millis_opt(due_ms)
            .single()
            .unwrap()
            .with_timezone(&tz);
        match index {
            0 => assert_eq!(due.minute(), 20),
            1 => assert_eq!((due.hour(), due.minute()), (8, 15)),
            2 => {
                assert!(due.weekday().number_from_monday() <= 5);
                assert_eq!((due.hour(), due.minute()), (8, 15));
            }
            3 => {
                assert_eq!(due.weekday().number_from_monday(), 1);
                assert_eq!(due.hour(), 10);
            }
            4 => {
                assert_eq!(due.day(), 31);
                assert_eq!(due.hour(), 8);
            }
            5 => assert_eq!((due.month(), due.day(), due.hour()), (2, 29, 8)),
            _ => unreachable!(),
        }
        let task = loaded(&client, &base, value["task"]["id"].as_str().unwrap()).await;
        assert_eq!(task["schedule"], value["task"]["schedule"]);
        assert_eq!(task["next_due_ms"], due_ms);
        assert_eq!(task["schedule_state"], "active");
        assert_eq!(task["played"], Value::Null);
    }
    let tasks: Value = get(&client, &base, "/api/skill-tasks?device_id=calendar-device")
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(tasks["total"], 6);
    assert!(engines.dev_mode); // No dev worker ticks or claims tasks in these tests.
}

#[tokio::test]
async fn relative_dates_and_bare_clock_are_real_future_times_with_receipts() {
    let (base, _server, _) = fixture(config()).await;
    let client = reqwest::Client::new();
    let tz = chrono::FixedOffset::east_opt(8 * 3600).unwrap();
    for (text, offset_days, hour, minute) in [
        ("明天上午八点", 1, 8, 0),
        ("后天下午八点半", 2, 20, 30),
        ("大后天晚上九点", 3, 21, 0),
    ] {
        let before = Utc::now().with_timezone(&tz).date_naive();
        let value = created(
            &client,
            &base,
            "relative-device",
            json!({"text":"日期提醒","at":text,"timezone":"Asia/Shanghai"}),
        )
        .await;
        let after = Utc::now().with_timezone(&tz).date_naive();
        let due = Utc
            .timestamp_millis_opt(value["task"]["next_due_ms"].as_i64().unwrap())
            .single()
            .unwrap()
            .with_timezone(&tz);
        assert!(
            due.date_naive() == before + chrono::Duration::days(offset_days)
                || due.date_naive() == after + chrono::Duration::days(offset_days)
        );
        assert_eq!((due.hour(), due.minute()), (hour, minute));
        let speech = value["speech"].as_str().unwrap();
        assert!(
            speech.contains(&due.year().to_string()),
            "receipt must name the real date: {speech}"
        );
    }
    let before = Utc::now().timestamp_millis();
    let bare = created(
        &client,
        &base,
        "relative-device",
        json!({"text":"最近八点","at":"上午八点","timezone":"Asia/Shanghai"}),
    )
    .await;
    let due = bare["task"]["next_due_ms"].as_i64().unwrap();
    assert!(due > before && due <= Utc::now().timestamp_millis() + 86_400_000);
}

#[tokio::test]
async fn calendar_invalid_dates_are_skipped_not_clamped_and_get_never_claims_playback() {
    let (base, _server, engines) = fixture(config()).await;
    let client = reqwest::Client::new();
    // Only the underlying API accepts an injected clock; the HTTP request cannot override now_ms.
    let month = engines
        .skill_system
        .execute_api(
            &fixed_context("skip-device", at("2028-01-31T09:00:00Z")),
            "tasks.create",
            &json!({"text":"每月31日","monthly":{"day":31,"time":"08:00"},"timezone":"UTC"}),
        )
        .unwrap();
    let month_id = month["task"]["id"].as_str().unwrap();
    assert_eq!(month["task"]["next_due_ms"], at("2028-03-31T08:00:00Z"));
    assert_eq!(
        loaded(&client, &base, month_id).await["next_due_ms"],
        at("2028-03-31T08:00:00Z")
    );
    assert_eq!(
        engines
            .skill_system
            .tick_due(at("2028-03-31T08:00:00Z"))
            .unwrap()
            .materialized,
        1
    );
    let advanced = loaded(&client, &base, month_id).await;
    assert_eq!(advanced["next_due_ms"], at("2028-05-31T08:00:00Z"));
    assert_eq!(advanced["schedule_state"], "active");
    assert_eq!(advanced["delivery_state"], "pending");
    assert_eq!(advanced["played"], Value::Null);
    let year = engines.skill_system.execute_api(&fixed_context("leap-device", at("2028-03-01T00:00:00Z")), "tasks.create", &json!({"text":"闰年提醒","yearly":{"month":2,"day":29,"time":"08:00"},"timezone":"UTC"})).unwrap();
    let year_id = year["task"]["id"].as_str().unwrap();
    assert_eq!(year["task"]["next_due_ms"], at("2032-02-29T08:00:00Z"));
    engines
        .skill_system
        .tick_due(at("2032-02-29T08:00:00Z"))
        .unwrap();
    let leap = loaded(&client, &base, year_id).await;
    assert_eq!(leap["next_due_ms"], at("2036-02-29T08:00:00Z"));
    assert_eq!(leap["delivery_state"], "pending");
    assert_eq!(leap["played"], Value::Null);
    let cancelled: Value = post(
        &client,
        &base,
        &format!("/api/skill-tasks/{year_id}/cancel"),
        json!({"device_id":"leap-device"}),
    )
    .await
    .json()
    .await
    .unwrap();
    assert_eq!(cancelled["task_state"], "cancelled");
    assert!(!cancelled["speech"].as_str().unwrap().is_empty());
    assert_eq!(
        loaded(&client, &base, year_id).await["delivery_state"],
        "cancelled"
    );
}

#[tokio::test]
async fn device_scoped_clock_query_and_ambiguous_text_cancel_do_not_cross_devices() {
    let (base, _server, engines) = fixture(config()).await;
    let client = reqwest::Client::new();
    let one = created(
        &client,
        &base,
        "voice-A",
        json!({"text":"相同喝水提醒","daily":"08:00"}),
    )
    .await;
    let two = created(
        &client,
        &base,
        "voice-A",
        json!({"text":"相同喝水提醒","daily":"09:00"}),
    )
    .await;
    let other = created(
        &client,
        &base,
        "voice-B",
        json!({"text":"相同喝水提醒","daily":"10:00"}),
    )
    .await;
    let ctx = fixed_context("voice-A", Utc::now().timestamp_millis());
    let query = engines
        .skill_system
        .execute_api(&ctx, "tasks.list", &json!({}))
        .unwrap();
    assert_eq!(query["tasks"].as_array().unwrap().len(), 2);
    assert!(!query["speech"].as_str().unwrap().is_empty());
    assert!(query["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .all(|task| task["device_id"] == "voice-A"));
    let ambiguous = engines
        .skill_system
        .execute_api(
            &fixed_context("voice-A", ctx.now_ms),
            "tasks.cancel",
            &json!({"text":"相同喝水提醒"}),
        )
        .unwrap_err();
    let response = blocking(move || Err(execution_error(ambiguous))).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let bytes = axum::body::to_bytes(response.into_body(), 16384)
        .await
        .unwrap();
    let clarification: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(clarification["error_code"], "clock_clarification");
    assert!(!clarification["speech"].as_str().unwrap().is_empty());
    assert_eq!(
        loaded(&client, &base, two["task"]["id"].as_str().unwrap()).await["state"],
        "active"
    );
    let id = one["task"]["id"].as_str().unwrap();
    let bad = post(
        &client,
        &base,
        &format!("/api/skill-tasks/{id}/cancel"),
        json!({"device_id":"voice-B"}),
    )
    .await;
    assert_eq!(bad.status(), StatusCode::NOT_FOUND);
    assert_eq!(loaded(&client, &base, id).await["state"], "active");
    let response = post(
        &client,
        &base,
        &format!("/api/skill-tasks/{id}/cancel"),
        json!({"device_id":"voice-A"}),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let cancelled = engines
        .skill_system
        .execute_api(
            &fixed_context("voice-A", ctx.now_ms),
            "tasks.cancel",
            &json!({"text":"相同喝水提醒"}),
        )
        .unwrap();
    assert!(!cancelled["speech"].as_str().unwrap().is_empty());
    assert_eq!(
        loaded(&client, &base, two["task"]["id"].as_str().unwrap()).await["state"],
        "cancelled"
    );
    assert_eq!(
        loaded(&client, &base, other["task"]["id"].as_str().unwrap()).await["state"],
        "active"
    );
    assert!(engines
        .skill_system
        .execute_api(
            &fixed_context("voice-A", ctx.now_ms),
            "tasks.cancel",
            &json!({"text":"相同喝水提醒"})
        )
        .is_err());
}
