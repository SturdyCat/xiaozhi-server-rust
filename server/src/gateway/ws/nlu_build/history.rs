//! 构建历史列表：`GET /api/nlu/model/builds`。
//!
//! 判据是 [`load_manifest`]（O(1)、不算哈希），**绝不用** `load_candidate` 当列表
//! 判据——它要对每个 800MB 产物算三遍哈希，列表页一次翻几行就把磁盘吃满；且它与
//! `/test` 不同源，会出现"列表说半成品、点进去 404 不存在"。
//!
//! 行来源是**目录**，不是"manifest 解析成功"：连 manifest 都坏掉的行照样出现在
//! 列表里（`integrity: corrupt`）——把坏产物从界面上抹掉，等于让运维看不见它，
//! 而那正是需要看见的一类。只有"目录名不是规范 UUID / 不是目录 / 符号链接"
//! 才丢弃（那不是构建目录）。
//!
//! `active` 判据是**配置真相**：`[nlu].bert_model` 的父目录名 == build_id。
//! 绝不读 `bootstrap/state.json`——那是自举流程的内部状态，不是配置。
//!
//! 排序显式三段 `(created_at 缺失者垫底, Reverse(created_at), build_id)`：
//! `created_at` 是定长 UTC `Z` 后缀（见 `now_rfc3339`），字符串序 == 时间序；
//! 同秒或同为缺失用 build_id 字典序稳定兜底，不依赖 HashMap 迭代顺序。
use super::{
    access, candidate_http_error, data_dir, error, load_manifest, valid_id, CandidateError,
    ManifestHead, INTEGRITY_CORRUPT,
};
use crate::runtime::Engines;
use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde_json::{json, Value};
use std::path::Path;

/// 单次列表的行数上界：build 目录是低频产物（一次训练一个），100 足够；
/// 超出只截断展示、总数照实报，不悄悄吞。
const MAX_ROWS: usize = 100;

/// 一行历史。`error` 仅在 corrupt 时非空——把"为什么坏"原样带给人。
struct HistoryRow {
    head: ManifestHead,
    active: bool,
    error: Option<String>,
}

impl HistoryRow {
    fn to_json(&self) -> Value {
        let integrity = if self.error.is_some() {
            INTEGRITY_CORRUPT
        } else {
            self.head.integrity
        };
        json!({
            "build_id": self.head.build_id,
            "created_at": self.head.created_at,
            "base_model": self.head.base_model,
            "base_model_source": self.head.base_model_source,
            "labels": self.head.labels,
            "integrity": integrity,
            "missing": self.head.missing,
            "active": self.active,
            "error": self.error,
        })
    }
}

/// 三段排序键：`created_at` 缺失垫底 → 新的在前 → build_id 字典序兜底。
fn sort_rows(rows: &mut [HistoryRow]) {
    rows.sort_by(|a, b| {
        let a_key = (a.head.created_at.is_none(), std::cmp::Reverse(&a.head.created_at), &a.head.build_id);
        let b_key = (b.head.created_at.is_none(), std::cmp::Reverse(&b.head.created_at), &b.head.build_id);
        a_key.cmp(&b_key)
    });
}

/// 现役 build_id：`[nlu].bert_model` 的父目录名。路径形态不对（旧配置 / 外部路径）
/// 返回 None，不猜。
fn active_build_id(bert_model: &str) -> Option<String> {
    let dir = Path::new(bert_model).parent()?.file_name()?.to_str()?;
    valid_id(dir).then(|| dir.to_string())
}

pub(crate) async fn build_history(
    State(engines): State<std::sync::Arc<Engines>>,
    headers: HeaderMap,
    axum::extract::Query(query): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Response {
    if let Some(r) = access(&engines, &headers, &query, false) {
        return r;
    }
    let store = engines.config_store.clone();
    let task = tokio::task::spawn_blocking(move || -> Result<Value, (StatusCode, &'static str, String)> {
        let cfg = store.load().map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "config_read_failed",
                e.to_string(),
            )
        })?;
        let data = data_dir(&cfg);
        let active = active_build_id(&cfg.nlu.bert_model);
        let root = data.join("models").join("nlu-builds");
        // 根目录不存在 = 从未构建过：空列表是**正常**回答，不是 404。
        let entries = match std::fs::read_dir(&root) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok(json!({"builds": [], "total": 0, "active_build_id": active}));
            }
            Err(e) => {
                return Err((
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "history_unavailable",
                    format!("build 根目录不可读: {e}"),
                ))
            }
        };
        let mut ids: Vec<String> = Vec::new();
        for entry in entries.flatten() {
            // 行来源是目录：文件与符号链接不是构建目录，丢弃（读目录本身就是行为）。
            let Ok(file_type) = entry.file_type() else { continue };
            if !file_type.is_dir() {
                continue;
            }
            let Ok(name) = entry.file_name().into_string() else {
                continue;
            };
            if !valid_id(&name) {
                continue;
            }
            ids.push(name);
        }
        let mut rows: Vec<HistoryRow> = Vec::with_capacity(ids.len());
        for id in &ids {
            match load_manifest(&data, id) {
                Ok(head) => rows.push(HistoryRow {
                    active: active.as_deref() == Some(id),
                    head,
                    error: None,
                }),
                Err(CandidateError::Corrupt { detail }) => {
                    // corrupt 行照常进列表（build_id 取自目录名），error 带原因。
                    rows.push(HistoryRow {
                        active: active.as_deref() == Some(id),
                        head: ManifestHead {
                            build_id: id.clone(),
                            created_at: None,
                            base_model: None,
                            base_model_source: None,
                            labels: Vec::new(),
                            files: Vec::new(),
                            integrity: INTEGRITY_CORRUPT,
                            missing: Vec::new(),
                        },
                        error: Some(detail),
                    });
                }
                // RootMissing / DirectoryMissing 对"从目录枚举出来的 id"不可能发生；
                // Incomplete 是行状态不是错误。出现即内部错位，报 500 让人来看。
                Err(e) => {
                    let (status, code, message) = candidate_http_error(&e);
                    return Err((status, code, message));
                }
            }
        }
        let total = rows.len();
        sort_rows(&mut rows);
        rows.truncate(MAX_ROWS);
        Ok(json!({
            "builds": rows.iter().map(HistoryRow::to_json).collect::<Vec<_>>(),
            "total": total,
            "active_build_id": active,
        }))
    });
    match tokio::time::timeout(std::time::Duration::from_secs(10), task).await {
        Ok(Ok(Ok(value))) => Json(value).into_response(),
        Ok(Ok(Err((status, code, e)))) => error(status, code, e),
        Ok(Err(e)) => error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "history_failed",
            e.to_string(),
        ),
        Err(_) => error(
            StatusCode::GATEWAY_TIMEOUT,
            "history_timeout",
            "History listing exceeded 10 seconds; capacity held until worker finishes",
        ),
    }
}

#[cfg(test)]
#[path = "history_tests.rs"]
mod tests;
