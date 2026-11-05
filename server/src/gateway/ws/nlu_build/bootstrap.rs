//! 首启自举：模型产物缺失时自动训练一版候选，**冒烟门禁**通过后受控自动采用。
//!
//! 目标只有一个：全新部署（第一次启动、产物还没生成）不该要求用户先手动点一次构建。
//! 因此这里的每一步都刻意"窄"：
//! - 只在**真缺产物**时触发（`bert_enabled=false` 本身不算缺：现网模型文件在，重训没有理由）；
//! - 只在构建槽位**空闲**时抢占（绝不打断手动构建），失败**绝不**自动换臂重试；
//! - 采用只走 [`crate::gateway::ws::config::save_patch_strict`]（与 `PUT /api/config`
//!   共用同一条写库主链，只有"当前配置从哪来"不同）；
//! - 冒烟判定**由服务端自己重跑**（见 `smoke`），不信训练脚本自报的结果；
//! - 一切判定/失败都落 `state.json`（见 `state`），跨重启可解释。
//!
//! 状态机：`idle → pending → running → success | adopted | gated_failed | stale | failed
//! →（退避）→ giving_up`，另有终态 `skipped`（人为决定，本条不再自动重试）。
//!
//! 两类失败的方向**不能反**（这是本文件最容易写错的地方）：
//! - `RevisionConflict`（构建依据的配置已过期）→ **立即 `stale`，不重试**。
//!   用同一个 `expected_revision` 重试永远不可能成功，只是把失败延迟 0.6 秒；
//!   更危险的是"labels 没变就照样采用"——模板与 action 可能同批被改，模型是拿旧模板训的。
//! - `Io`/`Persist`（SQLite busy、磁盘抖动）→ 重试 200/400/800ms，仍失败 → `adopt_io_error`。

/// 槽位句柄只有单测会直接碰（等待逻辑已搬去 `wait`）；`pub(super)` 是为了让
/// `bootstrap_tests` 的 `use super::*` 仍然拿得到，同时不产生未使用导入警告。
#[cfg(test)]
pub(super) use super::job;
use super::{
    data_dir, load_candidate, push_log, run_build, Artifact, BuildStart, BASE_MODEL,
    ORIGIN_BOOTSTRAP,
};

use crate::{
    gateway::ws::config::{save_patch_strict, SaveError, SaveOutcome},
    plugins::nlu::bert_feature_enabled,
    runtime::Engines,
};
use std::{path::Path, sync::Arc, time::Duration};

mod diagnose;
mod smoke;
mod state;
mod wait;

use diagnose::*;
use smoke::*;
use state::*;
use wait::*;

/// 启动后首延迟：与 `skill_tasks::start` / `voices::spawn_startup_probe` 同风格，
/// 不阻塞监听就绪（健康检查必须能立刻响应）。
const START_DELAY_SECS: u64 = 30;
const POLL_MS: u64 = 250;
/// 训练等待上限略大于 docker 侧 3600s 超时，留出产物校验时间。
const TRAIN_WAIT_SECS: u64 = 3700;
/// 自动采用的**瞬时 IO** 重试退避（共 3 次）。
///
/// 只用于 `Io`/`Persist`。`RevisionConflict` 走 `stale` 终态，绝不进这里。
const ADOPT_RETRY_MS: [u64; 3] = [200, 400, 800];

// ---------------------------------------------------------------------------
// 主体
// ---------------------------------------------------------------------------

enum Tick {
    Stop,
    Sleep(u64),
}

async fn record_failure(path: &Path, st: &mut State, kind: &'static str, detail: String) -> Tick {
    st.last_error_kind = Some(kind.to_string());
    st.last_error = Some(truncate(&detail, 400));
    // 无 docker 的环境重试多少次都一样：直接定终态，别刷日志。
    st.phase = if kind == "docker_unavailable" || st.attempts >= MAX_ATTEMPTS {
        Phase::GivingUp
    } else {
        Phase::Failed
    };
    if let Err(e) = save_state(path, st) {
        tracing::warn!(target: "nlu_bootstrap", "写状态失败: {e}");
    }
    push_log(&format!(
        "[bootstrap] 失败（{kind}）：{}｜{}",
        truncate(&detail, 300),
        next_step(kind)
    ))
    .await;
    if st.phase == Phase::GivingUp {
        push_log("[bootstrap] 已放弃自动训练；修好后删掉 state.json 或手动点“构建新模型”").await;
        Tick::Stop
    } else {
        Tick::Sleep(backoff_secs(st.attempts))
    }
}

/// 自动采用的结果。
enum AdoptOutcome {
    Adopted {
        revision: String,
    },
    /// 构建依据的配置已过期：终止，不重试、不重建。
    Stale {
        build: String,
        current: String,
    },
    Failed {
        kind: &'static str,
        detail: String,
    },
}

/// 自动采用：只改 4 个 bert 字段，走与 `PUT /api/config` 相同的写库主链。
///
/// `build_revision` 必须是 `artifact.source_revision`（构建那一刻的快照），
/// **不能**取"当前读到的最新值"——那等于放弃并发保护，标签会静默错位。
async fn adopt(
    engines: &Arc<Engines>,
    artifact: &Artifact,
    dir: &Path,
    build_revision: &str,
) -> AdoptOutcome {
    let isolated = artifact.config(dir);
    let patch = serde_json::json!({"nlu": {
        "bert_enabled": true,
        "bert_model": isolated.bert_model,
        "bert_vocab": isolated.bert_vocab,
        "bert_labels": isolated.bert_labels,
    }});
    let mut last = String::from("未产生具体错误");
    for (i, delay) in ADOPT_RETRY_MS.iter().enumerate() {
        let engines_owned = engines.clone();
        let patch_owned = patch.clone();
        let revision = build_revision.to_string();
        let result = tokio::task::spawn_blocking(move || {
            save_patch_strict(&engines_owned, &patch_owned, Some(&revision))
        })
        .await;
        let error = match result {
            Ok(Ok(SaveOutcome { revision, .. })) => {
                push_log(&format!("[bootstrap] 配置已保存，新指纹 {revision}")).await;
                return AdoptOutcome::Adopted { revision };
            }
            // 读不到 ≠ 变了：读失败必须当 IO 重试，否则一次 SQLite 抖动就会被判成 stale。
            Ok(Err(err)) if err.is_retryable_io() => match err {
                SaveError::Io { detail } => format!("读取配置库失败: {detail}"),
                SaveError::Persist { detail, .. } => format!("写入配置库失败: {detail}"),
                other => describe(&other),
            },
            Ok(Err(SaveError::RevisionConflict { current })) => {
                return AdoptOutcome::Stale {
                    build: build_revision.to_string(),
                    current,
                }
            }
            Ok(Err(err)) => {
                let detail = describe(&err);
                tracing::error!(target: "nlu_bootstrap", "采用配置被拒绝: {detail}");
                let kind = if matches!(err, SaveError::NoPersistence) {
                    "config_no_persistence"
                } else {
                    "config_invalid"
                };
                return AdoptOutcome::Failed { kind, detail };
            }
            Err(e) => format!("采用任务失败: {e}"),
        };
        last = error;
        // 最后一次不再睡：白等 800ms 没有意义。
        if i + 1 < ADOPT_RETRY_MS.len() {
            push_log(&format!(
                "[bootstrap] 配置库瞬时失败（第 {} 次）：{}，{}ms 后重试",
                i + 1,
                truncate(&last, 200),
                delay
            ))
            .await;
            tokio::time::sleep(Duration::from_millis(*delay)).await;
        }
    }
    AdoptOutcome::Failed {
        kind: "adopt_io_error",
        detail: last,
    }
}

fn describe(err: &SaveError) -> String {
    match err {
        SaveError::RevisionConflict { current } => format!("配置指纹已变为 {current}"),
        SaveError::Invalid { detail } => format!("补丁不合法: {detail}"),
        SaveError::WorkflowInvalid { detail } => format!("补丁不合法: {detail}"),
        SaveError::NoPersistence => "服务以内置默认配置启动，没有持久化落点".to_string(),
        SaveError::Io { detail } => format!("读取配置库失败: {detail}"),
        SaveError::Persist { path, detail } => format!("写入 {path} 失败: {detail}"),
    }
}

async fn run_once(engines: &Arc<Engines>) -> Tick {
    let now = epoch_secs();
    let cfg = match engines.config_store.load() {
        Ok(cfg) => cfg,
        Err(e) => {
            tracing::warn!(target: "nlu_bootstrap", "读取配置失败，稍后重试: {e}");
            return Tick::Sleep(60);
        }
    };
    let data = data_dir(&cfg);
    let path = state_path(&data);
    let mut st = load_state(&path);
    let mode = mode_from_env(std::env::var("NLU_BOOTSTRAP").ok().as_deref());
    let artifact = artifact_state(&cfg.nlu);
    if let Some(block) = precheck(
        engines.dev_mode,
        mode,
        bert_feature_enabled(),
        &cfg.nlu,
        artifact,
        &st,
        now,
    ) {
        return match block {
            Block::Stop { reason, detail } => {
                push_log(&format!("[bootstrap] skipped：{reason}（{detail}）")).await;
                if st.phase != Phase::Skipped {
                    st.phase = Phase::Skipped;
                    st.last_error_kind = Some(reason.to_string());
                    st.last_error = Some(detail);
                    if let Err(e) = save_state(&path, &st) {
                        tracing::warn!(target: "nlu_bootstrap", "写状态失败: {e}");
                    }
                }
                Tick::Stop
            }
            Block::Wait { reason, seconds } => {
                push_log(&format!(
                    "[bootstrap] {reason}：{seconds}s 后再看（不消耗尝试次数）"
                ))
                .await;
                Tick::Sleep(seconds)
            }
        };
    }
    // 数据目录不可写时训练必失败，还会把 attempts 白烧掉：先探一次再动手。
    if let Err(e) = std::fs::create_dir_all(&data) {
        push_log(&format!(
            "[bootstrap] 数据目录 {} 不可写（{e}），{RECHECK_SECS}s 后再看",
            data.display()
        ))
        .await;
        return Tick::Sleep(RECHECK_SECS);
    }

    push_log(&format!(
        "[bootstrap] 模型产物缺失，开始自举训练（底座 {}）…",
        BASE_MODEL.name
    ))
    .await;
    let started = run_build(engines, None, ORIGIN_BOOTSTRAP).await;
    let (build_id, revision) = match started {
        BuildStart::Started {
            build_id,
            source_revision,
            ..
        } => {
            st.attempts += 1;
            st.last_attempt_at = Some(now);
            st.phase = Phase::Running;
            if st.first_seen_at == 0 {
                st.first_seen_at = now;
            }
            st.last_build_id = Some(build_id.clone());
            st.build_source_revision = Some(source_revision.clone());
            st.current_revision = None;
            st.last_error_kind = None;
            st.last_error = None;
            st.smoke = None;
            st.bad_class_detail.clear();
            st.rejected_detail.clear();
            if let Err(e) = save_state(&path, &st) {
                tracing::warn!(target: "nlu_bootstrap", "写状态失败: {e}");
            }
            push_log(&format!(
                "[bootstrap] 构建 {build_id} 已启动（底座 {}）",
                BASE_MODEL.name
            ))
            .await;
            (build_id, source_revision)
        }
        // 让路之一（**不是失败**）：手动构建在跑 → 不打断、不占 attempts、不落终态。
        // 「槽位被接管」那条（`WaitOutcome::TakenOver`）与这里同构，改一处必须改另一处。
        BuildStart::Busy => {
            st.phase = Phase::Idle;
            let _ = save_state(&path, &st);
            push_log("[bootstrap] 已有构建在跑，自举让路（稍后再试）").await;
            return Tick::Sleep(START_DELAY_SECS);
        }
        BuildStart::RevisionConflict => {
            st.attempts += 1;
            st.last_attempt_at = Some(now);
            return record_failure(
                &path,
                &mut st,
                "config_revision_conflict",
                "自举启动瞬间保存态被改动".into(),
            )
            .await;
        }
        BuildStart::Invalid { code, message } => {
            st.attempts += 1;
            st.last_attempt_at = Some(now);
            return record_failure(&path, &mut st, code, message).await;
        }
        BuildStart::Unavailable { message } => {
            st.attempts += 1;
            st.last_attempt_at = Some(now);
            return record_failure(&path, &mut st, "config_read_failed", message).await;
        }
    };

    let waited = tokio::time::timeout(
        Duration::from_secs(TRAIN_WAIT_SECS),
        wait_build_done(&build_id),
    )
    .await;
    let done = match waited {
        Ok(WaitOutcome::Done(snapshot)) => snapshot,
        // 让路之二（**不是失败**）：槽位被手动构建接管，我们这一次的结果已无法观察。
        // 与上面 `BuildStart::Busy` 是同一类"让路"，处置必须一致：
        // `Phase::Idle` + **不烧 attempts** + **不碰 `last_attempt_at`**。
        //
        // 为什么不能走 `record_failure`：它会 `attempts += 1` 并按 `backoff_secs(attempts)`
        // 退避，连着被抢三次就 `GivingUp` —— 用户手动构建失败一次，自举就永久停摆。
        // 为什么不能落 `Phase::Skipped`：`Skipped` 在 `blocks_retry()` 里是终态，同样停摆，
        // 只能人工删 `state.json`（那正是上一轮修掉的 P0 那类 bug）。
        //
        // 自愈链（不需要终态也能收敛）：下次 tick 先跑 `precheck` 的 `artifact_state` —
        // 对方成功 → 产物齐全 → `artifact_present` → Stop；对方还在跑 → `run_build`
        // 返回 `Busy` → 继续让路；对方失败 → 产物仍缺 → 正常再训一版。
        Ok(WaitOutcome::TakenOver { by }) => {
            st.phase = Phase::Idle;
            // 落 `Idle` 本身看不出发生过什么，日志又会被轮转掉：kind + detail 是唯一
            // 能留在 `state.json` 里的痕迹，必须写。
            st.last_error_kind = Some("build_slot_taken_over".to_string());
            st.last_error = Some(truncate(
                &format!(
                    "构建槽位被 {by} 接管，自举无法确认构建 {build_id} 的结果\
                     （成功与否未知，不读它的产物）"
                ),
                400,
            ));
            if let Err(e) = save_state(&path, &st) {
                tracing::warn!(target: "nlu_bootstrap", "写状态失败: {e}");
            }
            push_log(&format!(
                "[bootstrap] 构建槽位被 {by} 接管：本次让路，{START_DELAY_SECS}s 后重查产物\
                 （不读取被接管构建的产物）。{}",
                next_step("build_slot_taken_over")
            ))
            .await;
            // `Sleep(START_DELAY_SECS)` 只是**下限**：下一轮 `precheck` 的 `backoff_remaining`
            // 还会按 `last_attempt_at`（构建**启动**时刻）再挡一道，实际间隔
            // = max(30s, 600s - 构建耗时)。故此处**不得**刷新 `last_attempt_at`，也不得用
            // `backoff_secs`——否则要么变成固定 600s，要么（attempts 不增时）退化成
            // `backoff_secs(0)=0` 的紧循环。
            return Tick::Sleep(START_DELAY_SECS);
        }
        Err(_) => {
            return record_failure(
                &path,
                &mut st,
                "timeout",
                format!("等待构建 {build_id} 结束超过 {TRAIN_WAIT_SECS}s"),
            )
            .await
        }
    };
    if done.state != "success" {
        let kind = classify_failure(&done.error, &done.log_tail);
        return record_failure(&path, &mut st, kind, done.error.clone()).await;
    }
    // S1：从**磁盘**重新加载候选（`load_candidate` 会重新做三件套完整性校验），
    // 而不是复用内存里那份——门禁要验的是落盘产物，不是刚算出来的对象。
    let loaded = {
        let data = data.clone();
        let id = build_id.clone();
        tokio::task::spawn_blocking(move || load_candidate(&data, &id)).await
    };
    let (artifact, dir) = match loaded {
        Ok(Ok(v)) => v,
        Ok(Err(e)) => {
            return record_failure(&path, &mut st, "artifact_invalid", e.to_string()).await
        }
        Err(e) => {
            return record_failure(
                &path,
                &mut st,
                "artifact_invalid",
                format!("候选加载任务失败: {e}"),
            )
            .await
        }
    };
    // S3：从日志尾取脚本自报的冒烟文本（取不到 = fail closed）。
    let plan = match parse_smoke(&done.log_tail, &artifact.labels) {
        Ok(plan) => plan,
        Err(f) => return gate_failed(&path, &mut st, &f, &build_id).await,
    };
    let guard = SnapshotGuard {
        threshold: cfg.nlu.bert_threshold,
        label_map: cfg.nlu.bert_label_map.clone(),
    };
    let gate = {
        let artifact = artifact.clone();
        let dir = dir.clone();
        let plan = plan.clone();
        tokio::task::spawn_blocking(move || run_gate(&artifact, &dir, &plan, &guard)).await
    };
    let report = match gate {
        Ok(Ok(report)) => report,
        Ok(Err(failure)) => return gate_failed(&path, &mut st, &failure, &build_id).await,
        Err(e) => {
            return record_failure(
                &path,
                &mut st,
                "artifact_invalid",
                format!("门禁任务失败: {e}"),
            )
            .await
        }
    };
    // 实测值先落盘再决定去留：无论 adopted 还是 gated_failed，运维都要能看到这几个数。
    st.smoke = Some(report.summary());
    st.bad_class_detail = report.bad_class_detail.clone();
    st.rejected_detail = report.rejected_detail.clone();
    if let Err(e) = save_state(&path, &st) {
        tracing::warn!(target: "nlu_bootstrap", "写状态失败: {e}");
    }
    push_log(&format!(
        "[bootstrap] smoke_passed：用例 {} 条{}，正例 {}/{}（错类接受 {}），负例拒识 {}/{}（判 other {} / 低于阈值 {}），耗时 {}ms。仅通过冒烟自检，未经独立效果验收",
        plan.cases.len(),
        if plan.truncated { "（脚本已裁剪）" } else { "" },
        report.ok,
        report.positives,
        report.bad_class,
        report.rejected,
        report.negatives,
        report.rejected_other,
        report.rejected_threshold,
        report.elapsed_ms
    ))
    .await;

    if mode == Mode::BuildOnly {
        st.phase = Phase::Success;
        if let Err(e) = save_state(&path, &st) {
            tracing::warn!(target: "nlu_bootstrap", "写状态失败: {e}");
        }
        push_log(
            "[bootstrap] NLU_BOOTSTRAP=build-only：候选已就绪，未启用（请在管理台手测后启用）",
        )
        .await;
        return Tick::Stop;
    }
    match adopt(engines, &artifact, &dir, &revision).await {
        AdoptOutcome::Adopted { revision } => {
            st.phase = Phase::Adopted;
            st.adopted_at = Some(epoch_secs());
            st.current_revision = Some(revision);
            st.last_error_kind = None;
            st.last_error = None;
            if let Err(e) = save_state(&path, &st) {
                tracing::warn!(target: "nlu_bootstrap", "写状态失败: {e}");
            }
            push_log(&format!(
                "[bootstrap] 已自动采用候选 {build_id}（底座 {}），仅通过冒烟自检，未经独立效果验收；旧模型文件未删除",
                artifact.base_model.as_deref().unwrap_or("未记录")
            ))
            .await;
            Tick::Stop
        }
        AdoptOutcome::Stale { build, current } => {
            st.phase = Phase::Stale;
            st.build_source_revision = Some(build.clone());
            st.current_revision = Some(current.clone());
            st.last_error_kind = Some("source_revision_stale".into());
            st.last_error = Some(truncate(
                &format!("训练期间配置已被修改（构建时 {build} / 当前 {current}）"),
                400,
            ));
            if let Err(e) = save_state(&path, &st) {
                tracing::warn!(target: "nlu_bootstrap", "写状态失败: {e}");
            }
            push_log(&format!(
                "[bootstrap] 训练期间配置已被修改（构建时 {build} / 当前 {current}），\
                 候选 {build_id} 已保留但未启用；请按最新配置重新构建（配置一个字节都没改）"
            ))
            .await;
            Tick::Stop
        }
        AdoptOutcome::Failed { kind, detail } => {
            let failure = GateFailure::new(kind, detail);
            gate_failed(&path, &mut st, &failure, &build_id).await
        }
    }
}

/// 门禁/采用没过：候选保留，配置不动，本条不再自动重试。
async fn gate_failed(path: &Path, st: &mut State, failure: &GateFailure, build_id: &str) -> Tick {
    st.phase = Phase::GatedFailed;
    st.last_error_kind = Some(failure.kind.to_string());
    st.last_error = Some(truncate(&failure.detail, 400));
    if let Err(e) = save_state(path, st) {
        tracing::warn!(target: "nlu_bootstrap", "写状态失败: {e}");
    }
    push_log(&format!(
        "[bootstrap] 门禁未通过（{}）：{}｜候选 {build_id} 保留，配置未改动。{}",
        failure.kind,
        truncate(&failure.detail, 300),
        next_step(failure.kind)
    ))
    .await;
    Tick::Stop
}

// ---------------------------------------------------------------------------
// 入口
// ---------------------------------------------------------------------------

/// 启动首启自举后台任务。**不 await**：监听已经就绪，这里只是"顺手看看要不要训一次"。
pub(crate) fn spawn_nlu_bootstrap(engines: Arc<Engines>) {
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(START_DELAY_SECS)).await;
        loop {
            match run_once(&engines).await {
                Tick::Stop => return,
                Tick::Sleep(seconds) => {
                    tokio::time::sleep(Duration::from_secs(seconds.max(1))).await
                }
            }
        }
    });
}

/// 只给单测用：不启动后台任务，直接把"跑一轮"暴露出来。
#[cfg(test)]
pub(super) async fn run_once_for_test(engines: &Arc<Engines>) -> &'static str {
    match run_once(engines).await {
        Tick::Stop => "stop",
        Tick::Sleep(_) => "sleep",
    }
}

#[cfg(test)]
#[path = "bootstrap_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "bootstrap/wait_tests.rs"]
mod wait_tests;
