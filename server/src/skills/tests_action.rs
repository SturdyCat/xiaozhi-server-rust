use super::*;
use rusqlite::params;

// ---------------------------------------------------------------------------
// 到期动作任务（skills_tasks.action）：分流、降级、播报写入
// （直推本身经 workflow::device_cmds 的会话通道，见 device_cmds_tests.rs；
// 这里只测 SkillSystem 侧的持久化与分流。）
// ---------------------------------------------------------------------------

fn action_step() -> Value {
    serde_json::json!({"api":"devices.call","args":{"tool":"power","arguments":{"state":"off"}}})
}

/// NOW = 2023-11-14T22:13:20Z；下一个 UTC 00:00 恰在 +6_400_000ms。
const NEXT_MIDNIGHT_UTC: i64 = NOW + 6_400_000;

#[test]
fn an_action_task_is_returned_for_direct_push_not_materialized() {
    let db = TestDb::new();
    let id = create(
        &db.system,
        &ctx("a", "create", NOW),
        serde_json::json!({"text":"已按时关闭电源","after":"五分钟","action":action_step()}),
    );
    // 创建即带出 action（管理页能看出这个任务"到期会动手"）。
    let task = db.system.get_task(&id, Some("a")).unwrap().unwrap();
    assert_eq!(task["action"]["api"], "devices.call");
    assert_eq!(task["action"]["args"]["tool"], "power");
    // 到期：不进 outbox，原样交还调用方直推。
    let report = db.system.tick_due(NOW + 300_000).unwrap();
    assert_eq!(report.materialized, 0);
    assert_eq!(report.actions.len(), 1);
    let due = &report.actions[0];
    assert_eq!(due.task_id, id);
    assert_eq!(due.device_id, "a");
    assert_eq!(due.text, "已按时关闭电源");
    assert!(due.action.contains("devices.call"));
    // 一次性动作任务照常标记完成，且没有任何 outbox 行。
    assert_eq!(
        db.system.get_task(&id, None).unwrap().unwrap()["schedule_state"],
        "completed"
    );
    assert_eq!(pending_count(&db.system), 0);
}

#[test]
fn an_offline_action_degrades_to_the_text_reminder() {
    // 离线降级 = scheduler 不直推，直接把原文写进 outbox（enqueue_task_speech）。
    let db = TestDb::new();
    let id = create(
        &db.system,
        &ctx("a", "create", NOW),
        serde_json::json!({"text":"已按时关闭电源","after":"五分钟","action":action_step()}),
    );
    let report = db.system.tick_due(NOW + 300_000).unwrap();
    assert_eq!(report.actions.len(), 1);
    let due = &report.actions[0];
    assert!(db
        .system
        .enqueue_task_speech(&due.task_id, &due.device_id, &due.text, NOW + 300_001)
        .unwrap());
    // 任务已完成，但 outbox 行仍可被设备认领（claim 只排除 cancelled）。
    let claim = db
        .system
        .claim_delivery("a", "s", NOW + 300_002)
        .unwrap()
        .unwrap();
    assert_eq!(claim.text, "已按时关闭电源");
    assert_eq!(claim.task_id.as_deref(), Some(id.as_str()));
}

#[test]
fn a_second_speech_yields_to_an_undelivered_one() {
    // 每任务至多一条未投递（唯一 unsettled 索引）：循环任务上一轮播报还没被
    // 设备认领时，本轮播报写不进——返回 false，不报错、不丢任务。
    let db = TestDb::new();
    let id = create(
        &db.system,
        &ctx("a", "create", NOW),
        serde_json::json!({"text":"已按时关闭电源","after":"五分钟","action":action_step()}),
    );
    let report = db.system.tick_due(NOW + 300_000).unwrap();
    let due = &report.actions[0];
    assert!(db
        .system
        .enqueue_task_speech(&due.task_id, &due.device_id, &due.text, NOW + 300_001)
        .unwrap());
    assert!(!db
        .system
        .enqueue_task_speech(&due.task_id, &due.device_id, &due.text, NOW + 300_002)
        .unwrap());
    assert_eq!(pending_count(&db.system), 1);
}

#[test]
fn a_recurring_action_task_advances_and_keeps_delivering() {
    let db = TestDb::new();
    let id = create(
        &db.system,
        &ctx("a", "create", NOW),
        serde_json::json!({"text":"已按时关闭电源","daily":"00:00","timezone":"UTC",
                           "action":action_step()}),
    );
    assert_eq!(
        db.system.get_task(&id, None).unwrap().unwrap()["next_due_ms"],
        NEXT_MIDNIGHT_UTC
    );
    // 第一次到期：交还直推 + 调度推进到次日。
    let report = db.system.tick_due(NEXT_MIDNIGHT_UTC).unwrap();
    assert_eq!(report.materialized, 0);
    assert_eq!(report.actions.len(), 1);
    assert_eq!(
        db.system.get_task(&id, None).unwrap().unwrap()["next_due_ms"],
        NEXT_MIDNIGHT_UTC + 86_400_000
    );
    // 直推成功 → 播原文 → 可认领；下一轮到期时上一轮已投递，不再互相顶掉。
    let due = &report.actions[0];
    assert!(db
        .system
        .enqueue_task_speech(&due.task_id, &due.device_id, &due.text, NEXT_MIDNIGHT_UTC + 1)
        .unwrap());
    let claim = db
        .system
        .claim_delivery("a", "s", NEXT_MIDNIGHT_UTC + 2)
        .unwrap()
        .unwrap();
    assert!(db
        .system
        .finish_delivery(&claim, true, NEXT_MIDNIGHT_UTC + 3)
        .unwrap());
    // 次日到期照常分流（动作任务不受上一轮已投递影响）。
    let report = db.system.tick_due(NEXT_MIDNIGHT_UTC + 86_400_000).unwrap();
    assert_eq!(report.actions.len(), 1);
    assert_eq!(report.actions[0].task_id, id);
}

#[test]
fn a_normal_task_is_unaffected_by_the_action_split() {
    // 无 action 的任务走原路径：物化进 outbox，actions 为空。
    let db = TestDb::new();
    create(
        &db.system,
        &ctx("a", "create", NOW),
        serde_json::json!({"text":"喝水","after":"五分钟"}),
    );
    let report = db.system.tick_due(NOW + 300_000).unwrap();
    assert_eq!(report.materialized, 1);
    assert!(report.actions.is_empty());
    assert_eq!(pending_count(&db.system), 1);
}

#[test]
fn outbox_backpressure_blocks_action_tasks_too() {
    // 背压语义与普通任务一致：outbox 满时动作任务也推进不了，
    // 否则一次性动作任务会"标记完成却无声无息"。
    let db = TestDb::new();
    let id = create(
        &db.system,
        &ctx("a", "create", NOW),
        serde_json::json!({"text":"已按时关闭电源","after":"五分钟","action":action_step()}),
    );
    let conn = db.system.connect().unwrap();
    for i in 0..super::store::MAX_DEVICE_PENDING {
        conn.execute(
            "INSERT INTO skills_outbox(id,task_id,device_id,text,state,available_ms,created_ms,updated_ms) VALUES (?1,NULL,'a','x','pending',?2,?2,?2)",
            params![format!("fill-{i}"), NOW],
        )
        .unwrap();
    }
    drop(conn);
    let report = db.system.tick_due(NOW + 300_000).unwrap();
    assert_eq!(report.actions.len(), 0);
    // 背压下调度也不推进：任务仍 active、next_due 未变。
    let task = db.system.get_task(&id, None).unwrap().unwrap();
    assert_eq!(task["schedule_state"], "active");
}
