use super::*;
use rusqlite::params;

#[test]
fn idempotency_window_is_finite_and_reuse_after_seven_days_is_explicit() {
    let db = TestDb::new();
    let args = json!({"text":"喝水","after":"1分钟"});
    let first = db
        .system
        .execute_api(&ctx("a", "same", NOW), "tasks.create", &args)
        .unwrap();
    let second = db
        .system
        .execute_api(
            &ctx("a", "same", NOW + 7 * 86_400_000 + 1),
            "tasks.create",
            &args,
        )
        .unwrap();
    assert_ne!(first["task"]["id"], second["task"]["id"]);
    assert_eq!(db.system.list_tasks(None, 100, 0).unwrap()["total"], 2);
}

#[test]
fn request_log_hard_capacity_rolls_back_new_side_effect() {
    let db = TestDb::new();
    let conn = db.system.connect().unwrap();
    conn.execute("WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM n WHERE x<8192) INSERT INTO skills_requests(device_id,request_id,fingerprint,response,created_ms) SELECT 'seed',CAST(x AS TEXT),'{}','{}',?1 FROM n",params![NOW]).unwrap();
    let error = db
        .system
        .execute_api(&ctx("a", "new", NOW), "messages.send", &json!({"text":"x"}))
        .unwrap_err();
    assert!(error.to_string().contains("capacity"));
    assert_eq!(pending_count(&db.system), 0);
    let result = db
        .system
        .execute_api(
            &ctx("a", "new", NOW + 7 * 86_400_000 + 1),
            "messages.send",
            &json!({"text":"x"}),
        )
        .unwrap();
    assert_eq!(result["state"], "pending");
}

#[test]
fn task_capacity_is_device_bound_and_other_device_can_still_create() {
    let db = TestDb::new();
    let conn = db.system.connect().unwrap();
    conn.execute("WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM n WHERE x<128) INSERT INTO skills_tasks(id,device_id,text,schedule,state,next_due_ms,created_ms,updated_ms) SELECT CAST(x AS TEXT),'full','x','{\"kind\":\"once\"}','active',?1,?1,?1 FROM n",params![NOW+60_000]).unwrap();
    assert!(db
        .system
        .execute_api(
            &ctx("full", "new", NOW),
            "tasks.create",
            &json!({"text":"x","after":"1分钟"})
        )
        .unwrap_err()
        .to_string()
        .contains("capacity"));
    create(
        &db.system,
        &ctx("healthy", "new", NOW),
        json!({"text":"x","after":"1分钟"}),
    );
    assert_eq!(
        db.system.list_tasks(Some("healthy"), 100, 0).unwrap()["total"],
        1
    );
}

#[test]
fn terminal_history_prunes_but_pending_rows_survive() {
    let db = TestDb::new();
    let conn = db.system.connect().unwrap();
    conn.execute("WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM n WHERE x<2050) INSERT INTO skills_outbox(id,device_id,text,state,available_ms,created_ms,updated_ms) SELECT CAST(x AS TEXT),'a','x','submitted',?1,?1,?1 FROM n",params![NOW]).unwrap();
    conn.execute("INSERT INTO skills_outbox(id,device_id,text,state,available_ms,created_ms,updated_ms) VALUES ('pending','a','retain','pending',?1,?1,?1)",params![NOW-1]).unwrap();
    assert_eq!(db.system.tick_due(NOW).unwrap().materialized, 0);
    let terminal: i64 = conn
        .query_row(
            "SELECT count(*) FROM skills_outbox WHERE state='submitted'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(terminal, 2048);
    assert_eq!(pending_count(&db.system), 1);
}
