use super::*;
use rusqlite::params;
use std::{
    sync::{atomic::Ordering, Barrier},
    thread,
};

#[test]
fn one_shot_persists_restart_offline_then_submitted_is_not_played() {
    let db = TestDb::new();
    let id = create(
        &db.system,
        &ctx("a", "create", NOW),
        json!({"text":"喝水","after":"五分钟"}),
    );
    assert_eq!(
        db.system.get_task(&id, Some("a")).unwrap().unwrap()["delivery_state"],
        "none"
    );
    let restarted = SkillSystem::new(db.system.path.clone());
    assert_eq!(restarted.tick_due(NOW + 299_999).unwrap(), 0);
    assert_eq!(restarted.tick_due(NOW + 300_000).unwrap(), 1);
    assert_eq!(restarted.tick_due(NOW + 999_999).unwrap(), 0);
    let task = restarted.get_task(&id, Some("a")).unwrap().unwrap();
    assert_eq!(task["schedule_state"], "completed");
    assert_eq!(task["delivery_state"], "pending");
    assert!(task["played"].is_null());
    assert!(restarted
        .claim_delivery("b", "s", NOW + 999_999)
        .unwrap()
        .is_none());
    let claim = restarted
        .claim_delivery("a", "s", NOW + 999_999)
        .unwrap()
        .unwrap();
    assert_eq!(claim.task_id.as_deref(), Some(id.as_str()));
    assert!(restarted
        .delivery_is_current(&claim, NOW + 999_999)
        .unwrap());
    assert_eq!(
        restarted.get_task(&id, None).unwrap().unwrap()["delivery_state"],
        "leased"
    );
    assert!(restarted
        .finish_delivery(&claim, true, NOW + 1_000_000)
        .unwrap());
    let task = restarted.get_task(&id, None).unwrap().unwrap();
    assert_eq!(task["delivery_state"], "submitted");
    assert!(task["played"].is_null());
    assert!(!restarted
        .finish_delivery(&claim, true, NOW + 1_000_001)
        .unwrap());
    assert!(restarted
        .claim_delivery("a", "s", NOW + 1_000_001)
        .unwrap()
        .is_none());
}

#[test]
fn idempotency_survives_restart_and_conflicting_payload_is_rejected() {
    let db = TestDb::new();
    let context = ctx("a", "stable", NOW);
    let args = json!({"text":"喝水","after":"五分钟"});
    let first = db
        .system
        .execute_api(&context, "tasks.create", &args)
        .unwrap();
    let restarted = SkillSystem::new(db.system.path.clone());
    let second = restarted
        .execute_api(&ctx("a", "stable", NOW + 1000), "tasks.create", &args)
        .unwrap();
    assert_eq!(first["task"]["id"], second["task"]["id"]);
    assert_eq!(first["task"]["next_due_ms"], second["task"]["next_due_ms"]);
    assert_eq!(second["deduplicated"], true);
    assert!(restarted
        .execute_api(
            &context,
            "tasks.create",
            &json!({"text":"吃药","after":"五分钟"})
        )
        .unwrap_err()
        .to_string()
        .contains("idempotency conflict"));
    assert_eq!(restarted.list_tasks(None, 100, 0).unwrap()["total"], 1);
}

#[test]
fn cancellation_before_due_and_during_lease_stops_future_delivery() {
    let db = TestDb::new();
    let id = create(
        &db.system,
        &ctx("a", "create", NOW),
        json!({"text":"喝水","after":"1秒"}),
    );
    assert!(db.system.get_task(&id, Some("b")).unwrap().is_none());
    assert!(db
        .system
        .execute_api(
            &ctx("b", "cancel", NOW),
            "tasks.cancel",
            &json!({"task_id":id})
        )
        .is_err());
    assert_eq!(db.system.tick_due(NOW + 1000).unwrap(), 1);
    let claim = db
        .system
        .claim_delivery("a", "s", NOW + 1000)
        .unwrap()
        .unwrap();
    db.system
        .execute_api(
            &ctx("a", "cancel", NOW + 1001),
            "tasks.cancel",
            &json!({"task_id":id}),
        )
        .unwrap();
    assert!(!db.system.delivery_is_current(&claim, NOW + 1002).unwrap());
    assert!(!db.system.finish_delivery(&claim, true, NOW + 1002).unwrap());
    assert!(!db
        .system
        .finish_delivery(&claim, false, NOW + 1002)
        .unwrap());
    assert!(db
        .system
        .claim_delivery("a", "s", NOW + 999_999)
        .unwrap()
        .is_none());
    assert_eq!(
        db.system.get_task(&id, None).unwrap().unwrap()["delivery_state"],
        "cancelled"
    );
    let second = create(
        &db.system,
        &ctx("a", "create2", NOW),
        json!({"text":"吃药","after":"1秒"}),
    );
    db.system
        .execute_api(
            &ctx("a", "cancel2", NOW),
            "tasks.cancel",
            &json!({"task_id":second}),
        )
        .unwrap();
    assert_eq!(db.system.tick_due(NOW + 1000).unwrap(), 0);
}

#[test]
fn expired_or_forged_leases_cannot_ack_a_new_owner() {
    let db = TestDb::new();
    db.system
        .execute_api(
            &ctx("a", "m", NOW),
            "messages.send",
            &json!({"text":"你好"}),
        )
        .unwrap();
    let old = db.system.claim_delivery("a", "old", NOW).unwrap().unwrap();
    let restarted = SkillSystem::new(db.system.path.clone());
    let new = restarted
        .claim_delivery("a", "new", old.lease_until_ms)
        .unwrap()
        .unwrap();
    assert_ne!(old.lease_token, new.lease_token);
    assert!(!restarted
        .finish_delivery(&old, true, old.lease_until_ms)
        .unwrap());
    assert!(!restarted
        .finish_delivery(&old, false, old.lease_until_ms)
        .unwrap());
    let mut forged = new.clone();
    forged.device_id = "b".into();
    assert!(!restarted
        .finish_delivery(&forged, true, old.lease_until_ms)
        .unwrap());
    assert!(restarted
        .finish_delivery(&new, false, old.lease_until_ms)
        .unwrap());
    assert!(restarted
        .claim_delivery("a", "third", old.lease_until_ms + 999)
        .unwrap()
        .is_none());
    assert!(restarted
        .claim_delivery("a", "third", old.lease_until_ms + 1000)
        .unwrap()
        .is_some());
}

#[test]
fn concurrent_claims_have_one_winner_and_serialize_all_delivery_for_a_device() {
    let db = TestDb::new();
    for i in 0..2 {
        db.system
            .execute_api(
                &ctx("a", &format!("m{i}"), NOW),
                "messages.send",
                &json!({"text":"你好"}),
            )
            .unwrap();
    }
    let barrier = Arc::new(Barrier::new(8));
    let mut workers = vec![];
    for i in 0..8 {
        let system = db.system.clone();
        let barrier = barrier.clone();
        workers.push(thread::spawn(move || {
            barrier.wait();
            system.claim_delivery("a", &format!("s{i}"), NOW).unwrap()
        }));
    }
    let claims: Vec<_> = workers
        .into_iter()
        .filter_map(|worker| worker.join().unwrap())
        .collect();
    assert_eq!(claims.len(), 1);
    assert!(db
        .system
        .finish_delivery(&claims[0], true, NOW + 1)
        .unwrap());
    assert!(db
        .system
        .claim_delivery("a", "next", NOW + 1)
        .unwrap()
        .is_some());
}

#[test]
fn concurrent_same_request_creates_one_task() {
    let db = TestDb::new();
    db.system.list_tasks(None, 1, 0).unwrap();
    let barrier = Arc::new(Barrier::new(4));
    let workers: Vec<_> = (0..4)
        .map(|_| {
            let system = db.system.clone();
            let barrier = barrier.clone();
            thread::spawn(move || {
                barrier.wait();
                create(
                    &system,
                    &ctx("a", "same", NOW),
                    json!({"text":"喝水","after":"1分钟"}),
                )
            })
        })
        .collect();
    let ids: Vec<_> = workers.into_iter().map(|w| w.join().unwrap()).collect();
    assert!(ids.iter().all(|id| id == &ids[0]));
    assert_eq!(db.system.list_tasks(None, 100, 0).unwrap()["total"], 1);
}

#[test]
fn repeating_tasks_coalesce_offline_and_advance_after_submission() {
    let db = TestDb::new();
    let id = create(
        &db.system,
        &ctx("a", "repeat", NOW),
        json!({"text":"喝水","interval":"1秒"}),
    );
    assert_eq!(db.system.tick_due(NOW + 1000).unwrap(), 1);
    assert_eq!(db.system.tick_due(NOW + 10_000_000).unwrap(), 0);
    assert_eq!(pending_count(&db.system), 1);
    let task = db.system.get_task(&id, None).unwrap().unwrap();
    assert!(task["next_due_ms"].as_i64().unwrap() > NOW + 10_000_000);
    let claim = db
        .system
        .claim_delivery("a", "s", NOW + 10_000_000)
        .unwrap()
        .unwrap();
    assert!(db
        .system
        .finish_delivery(&claim, true, NOW + 10_000_001)
        .unwrap());
    assert_eq!(db.system.tick_due(NOW + 10_001_000).unwrap(), 1);
    assert_eq!(pending_count(&db.system), 1);
}

#[test]
fn capacity_retains_pending_and_does_not_starve_another_device() {
    let db = TestDb::new();
    let blocked = create(
        &db.system,
        &ctx("full", "r", NOW),
        json!({"text":"x","after":"1秒"}),
    );
    let other = create(
        &db.system,
        &ctx("other", "r", NOW),
        json!({"text":"x","after":"1秒"}),
    );
    let conn = db.system.connect().unwrap();
    for i in 0..256 {
        conn.execute("INSERT INTO skills_outbox(id,device_id,text,state,available_ms,created_ms,updated_ms) VALUES (?1,'full','x','pending',?2,?2,?2)",params![format!("pending-{i}"),NOW]).unwrap();
    }
    // Put more than the tick limit of older blocked tasks ahead of the healthy device.
    for i in 0..300 {
        conn.execute("INSERT INTO skills_tasks(id,device_id,text,schedule,state,next_due_ms,created_ms,updated_ms) VALUES (?1,'full','x','{\"kind\":\"once\"}','active',?2,?2,?2)",params![format!("blocked-{i}"),NOW]).unwrap();
    }
    assert!(db
        .system
        .execute_api(
            &ctx("full", "overflow", NOW),
            "messages.send",
            &json!({"text":"x"})
        )
        .is_err());
    assert_eq!(db.system.tick_due(NOW + 1000).unwrap(), 1);
    assert_eq!(
        db.system.get_task(&other, None).unwrap().unwrap()["delivery_state"],
        "pending"
    );
    assert_eq!(
        db.system.get_task(&blocked, None).unwrap().unwrap()["schedule_state"],
        "active"
    );
    assert_eq!(pending_count(&db.system), 257);
}

#[test]
fn partial_plan_failure_does_not_claim_rollback_of_previously_committed_steps() {
    let db = TestDb::new();
    db.system
        .execute_api(
            &ctx("a", "step0", NOW),
            "messages.send",
            &json!({"text":"first"}),
        )
        .unwrap();
    assert!(db
        .system
        .execute_api(
            &ctx("a", "step1", NOW),
            "tasks.create",
            &json!({"text":"second","at":"周一八点"})
        )
        .is_err());
    assert_eq!(pending_count(&db.system), 1);
    assert_eq!(db.system.list_tasks(None, 100, 0).unwrap()["total"], 0);
    let cancelled = ctx("a", "step2", NOW);
    cancelled.cancelled.store(true, Ordering::Release);
    assert!(db
        .system
        .execute_api(&cancelled, "messages.send", &json!({"text":"third"}))
        .is_err());
    assert_eq!(pending_count(&db.system), 1);
}

#[test]
fn pagination_is_device_scoped_and_bounded() {
    let db = TestDb::new();
    for (d, r) in [("a", "1"), ("a", "2"), ("b", "3")] {
        create(
            &db.system,
            &ctx(d, r, NOW),
            json!({"text":"x","after":"1分钟"}),
        );
    }
    let result = db.system.list_tasks(Some("a"), 1, 1).unwrap();
    assert_eq!(result["total"], 2);
    assert_eq!(result["tasks"].as_array().unwrap().len(), 1);
    assert_eq!(db.system.list_tasks(None, 100, 0).unwrap()["total"], 3);
    assert!(db.system.list_tasks(None, 101, 0).is_err());
    assert!(db.system.list_tasks(None, 10, 10001).is_err());
}
