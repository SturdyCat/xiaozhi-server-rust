//! 等待与退避时序：`wait_build_done` 怎么区分"自己的构建结束"，以及被接管后
//! "还要等多久"是按**构建启动时刻**连续算的，不是一个阶跃。
//!
//! 和 `bootstrap_tests.rs` 分开放，是因为这几条全在钉时序/竞态：它们的失败信息是
//! "等错了"，与状态机、门禁规则的失败信息不同类。

use super::*;

#[test]
fn a_build_only_counts_as_done_when_it_is_our_own() {
    // 竞态回归：自举构建 A 结束的 250ms 轮询窗口内用户点了手动构建 B。
    // 只看 `!running` 会等到 B 结束、拿 **B 的日志**配 **A 的产物**跑门禁。
    let mut j = super::super::BuildJob::new();
    j.build_id = Some("mine".into());
    j.running = true;
    assert!(!is_finished(&j, "mine"), "在跑就不算结束");
    j.running = false;
    assert!(is_finished(&j, "mine"));
    j.build_id = Some("theirs".into());
    j.state = "success";
    assert!(!is_finished(&j, "mine"), "别人的构建结束不能当成自己的");
    // 槽位被清空（重启回 idle）时也不能误认。
    let fresh = super::super::BuildJob::new();
    assert!(!is_finished(&fresh, "mine"));
}

#[test]
fn a_taken_over_slot_is_not_a_timeout() {
    // 槽位被手动构建接管：必须能立刻区分出来，否则只能等满 3700s 再误报"训练超时"。
    let mut j = super::super::BuildJob::new();
    j.build_id = Some("mine".into());
    j.running = true;
    assert_eq!(taken_over(&j, "mine"), None, "自己还在跑，不算被接管");

    j.running = false;
    j.build_id = Some("theirs".into());
    j.state = "success";
    assert_eq!(taken_over(&j, "mine").as_deref(), Some("theirs"));

    // 别人还在跑也算被接管（我们那次的结果已经观察不到了）。
    j.running = true;
    assert_eq!(taken_over(&j, "mine").as_deref(), Some("theirs"));

    // 槽位被清空：同样拿不到结果。
    let reset = super::super::BuildJob::new();
    assert!(taken_over(&reset, "mine").is_some());
}

#[tokio::test]
async fn a_taken_over_slot_ends_the_wait_instead_of_timing_out() {
    // 真造一次接管：`wait_build_done` 正等 "mine"，槽位被换成 "theirs"。
    // 没有这条，`TakenOver` 只是一段"看着对"的代码——它正是本轮新加、又最难自然触发
    // 的一支（250ms 采样窗口 + 手动构建抢槽），必须由产出它的函数亲自钉住。
    let _gate = super::super::tests::JOB_TEST_GATE
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    {
        let mut j = job().lock().await;
        *j = super::super::BuildJob {
            running: true,
            build_id: Some("mine".into()),
            state: "running",
            ..Default::default()
        };
    }
    let waiter = tokio::spawn(async { wait_build_done("mine").await });
    // 先让它至少采样一次"自己还在跑"，再换槽位（真实竞态就是这个顺序：
    // A 结束 → 用户点了手动构建 B 覆盖 build_id）。
    tokio::time::sleep(Duration::from_millis(POLL_MS * 2)).await;
    {
        let mut j = job().lock().await;
        j.running = false;
        j.build_id = Some("theirs".into());
        j.state = "success";
    }
    let outcome = tokio::time::timeout(Duration::from_secs(5), waiter)
        .await
        .expect("槽位被接管必须立刻返回，不能等满 TRAIN_WAIT_SECS")
        .expect("等待任务不应 panic");
    match outcome {
        WaitOutcome::TakenOver { by } => assert_eq!(by, "theirs"),
        WaitOutcome::Done(_) => panic!("接管不能当成 Done：那是别人那一版的结果"),
    }
    // 复原槽位，别把状态留给后面的用例。
    {
        let mut j = job().lock().await;
        *j = super::super::BuildJob::new();
    }
}

#[test]
fn precheck_releases_by_elapsed_since_build_start_not_by_a_step() {
    // 被接管后 `attempts` 停在 1、`last_attempt_at` 是**构建启动时刻**，所以"还要等多久"
    // 随构建耗时连续变化。只测两端的话，一个
    // `if elapsed > 600 { None } else { Some(600) }` 的**阶跃**实现也能同时满足两端——
    // 中间那条才是"按 elapsed 连续放行"的证据。
    let mut nlu = crate::plugins::nlu::NluConfig::default();
    nlu.enabled = true;
    let now = 10_000;
    let mut st = State::default();
    st.attempts = 1;
    let wait = |st: &State| match precheck(
        false,
        Mode::Adopt,
        true,
        &nlu,
        ArtifactState::Missing,
        st,
        now,
    ) {
        None => None,
        Some(Block::Wait { reason, seconds }) => {
            assert_eq!(reason, "backoff", "这里只该撞退避，不该是别的 Block");
            Some(seconds)
        }
        Some(other) => panic!("{other:?}"),
    };
    // 构建跑了 1 小时：早过 600s 窗口，放行。
    st.last_attempt_at = Some(now - 3600);
    assert_eq!(wait(&st), None);
    // 构建跑了 3 分钟：还差 420s（600 - 180）。
    st.last_attempt_at = Some(now - 180);
    assert_eq!(wait(&st), Some(420));
    // 构建耗时为 0（刚启动就被接管）：满 600s。
    st.last_attempt_at = Some(now);
    assert_eq!(wait(&st), Some(600));
}

#[test]
fn a_taken_over_slot_lands_in_idle_which_precheck_still_lets_through_to_backoff() {
    // 这是 `Phase::Failed → Phase::Idle` 这一改动的**唯一行为风险点**：
    // 若 `Idle` 落进 `blocks_retry()` 的终态集合，`precheck` 会先返回
    // `phase_terminal`，退避那条分支永远走不到，自举就停在"看起来还在等"的假象里。
    // 所以这里同时钉两件事：`Idle` 不是终态、且被接管后的状态确实走到 `backoff`。
    let mut nlu = crate::plugins::nlu::NluConfig::default();
    nlu.enabled = true;
    assert!(!Phase::Idle.blocks_retry(), "Idle 必须是可重试态");
    // 对照：`Skipped` 是终态（这正是不用 `Skipped` 兜这一支的原因）。
    assert!(Phase::Skipped.blocks_retry(), "Skipped 是终态");

    let now = 10_000;
    // 被接管那一刻写下的状态：phase=Idle、attempts 停在 1、last_attempt_at 是**构建启动**时刻。
    let mut st = State::default();
    st.phase = Phase::Idle;
    st.attempts = 1;
    st.last_attempt_at = Some(now - 180);
    match precheck(
        false,
        Mode::Adopt,
        true,
        &nlu,
        ArtifactState::Missing,
        &st,
        now,
    ) {
        Some(Block::Wait { reason, seconds }) => {
            assert_eq!(reason, "backoff", "不能被 phase_terminal 拦掉");
            assert_eq!(seconds, 420);
        }
        None => panic!("此时还在退避窗口里，不该直接放行"),
        Some(Block::Stop { reason, .. }) => {
            panic!("Idle 被当成终态拦下了：{reason}（被接管后自举就再也不会跑了）")
        }
    }
}
