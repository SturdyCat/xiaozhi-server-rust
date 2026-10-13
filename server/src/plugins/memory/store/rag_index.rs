//! [`Store`] 的 RAG 索引扩展（P1-9）：`rag_docs` / `rag_chunks` / `rag_terms` 三表。
//!
//! 放在 store 子模块的原因与 `maintenance.rs` 同构：需要访问 `Store` 的私有 `conn()`，
//! 且"同一 SQLite 文件内的表组"归同一所有者——graph/store/rag 的表互不触碰，
//! graph ⇄ rag 切换后各自数据都保留（PRD P1-9 验收 7）。
//!
//! ## 零 DDL 版本策略
//!
//! 三表用 `CREATE TABLE IF NOT EXISTS` 幂等建表（[`ensure_rag_tables`]），
//! **不 bump `schema::LATEST_VERSION`**：迁移编号留给"影响既有表结构"的变更；
//! rag 三表只被 rag 后端触碰，graph/store 的库文件没有它们也完全正常。
//!
//! ## 重建的原子性（PRD 验收 3）
//!
//! 全量重建在**单事务**内完成（先清后建）：SQLite 事务提交前读者看到的是**旧的完整索引**，
//! 提交后一次性切到新索引——不存在半成品索引窗口，"重建期间对话不阻塞、
//! 召回返回已就绪内容"以比逐文档可见**更强**的语义满足。

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use rusqlite::OptionalExtension;

use super::{schema, terms, Store};

/// 一次成功重建的摘要（stats 与状态卡显示）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BuildSummary {
    pub docs: u32,
    pub chunks: u32,
    pub at_ms: i64,
}

/// 索引内计数（`/api/memory/status` 的 rag 段）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RagIndexCounts {
    pub docs: i64,
    pub chunks: i64,
}

/// 一次文档检索命中（chunk 正文 + 来源文档路径 + 命中词项数）。
#[derive(Debug, Clone)]
pub struct RagSearchHit {
    /// 命中 chunk 的稳定 id：检索可解释性/后续"跳原文"预留，正文消费方不读。
    #[allow(dead_code)]
    pub chunk_id: String,
    pub text: String,
    /// 来源文档相对路径（可解释性：部署者能验证"为什么召回了这段"）。
    pub source: String,
    pub hits: i64,
}

impl Store {
    /// 幂等建 rag 三表（`RagMemory` 构造时调用一次；graph/store 库无需这些表）。
    pub(in crate::plugins::memory) fn ensure_rag_tables(&self) -> Result<()> {
        self.conn().execute_batch(
            "CREATE TABLE IF NOT EXISTS rag_docs (
                 id TEXT PRIMARY KEY,
                 path TEXT UNIQUE NOT NULL,
                 title TEXT NOT NULL,
                 size INTEGER NOT NULL,
                 mtime_ms INTEGER NOT NULL,
                 chunk_count INTEGER NOT NULL,
                 indexed_at_ms INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS rag_chunks (
                 id TEXT PRIMARY KEY,
                 doc_id TEXT NOT NULL REFERENCES rag_docs(id) ON DELETE CASCADE,
                 seq INTEGER NOT NULL,
                 text TEXT NOT NULL,
                 UNIQUE (doc_id, seq)
             );
             CREATE TABLE IF NOT EXISTS rag_terms (
                 term TEXT NOT NULL,
                 chunk_id TEXT NOT NULL REFERENCES rag_chunks(id) ON DELETE CASCADE,
                 PRIMARY KEY (term, chunk_id)
             ) WITHOUT ROWID;",
        )?;
        Ok(())
    }

    /// 全量重建文档索引（单事务；调用方负责放 `spawn_blocking`）。
    ///
    /// `docs_dir` 不存在 → 清空索引并返回零文档（"零知识零影响"，PRD 验收 2）。
    pub(in crate::plugins::memory) fn rebuild_docs(
        &self,
        docs_dir: &str,
        chunk_size: usize,
        overlap: usize,
    ) -> Result<BuildSummary> {
        let files = collect_docs(Path::new(docs_dir));
        let now = schema::now_ms() as i64;
        // ⚠️ ensure 必须在取 conn guard 之前（Mutex 不可重入）
        self.ensure_rag_tables()?;
        let mut conn = self.conn();
        let tx = conn.transaction().context("开启 RAG 重建事务失败")?;
        // 先清后建：级联清掉 chunks 与 terms（foreign_keys=ON，schema::open 已开启）。
        tx.execute("DELETE FROM rag_docs", [])?;
        let mut total_chunks = 0u32;
        for (path, size, mtime_ms) in &files {
            // 非法 UTF-8 文档跳过（不因一个坏文件拖垮整个重建）
            let Ok(content) = std::fs::read_to_string(path) else {
                tracing::warn!("RAG 跳过非 UTF-8 文档: {}", path.display());
                continue;
            };
            let path_str = path.to_string_lossy().replace('\\', "/");
            let title = path
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            let doc_id = schema::stable_id("rdoc", &[&path_str]);
            let chunks = chunk_text(&content, chunk_size, overlap);
            tx.execute(
                "INSERT OR REPLACE INTO rag_docs
                     (id, path, title, size, mtime_ms, chunk_count, indexed_at_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                rusqlite::params![doc_id, path_str, title, size, mtime_ms, chunks.len() as i64, now],
            )?;
            for (seq, text) in chunks.iter().enumerate() {
                let chunk_id = schema::stable_id("rchk", &[&doc_id, &seq.to_string()]);
                tx.execute(
                    "INSERT OR REPLACE INTO rag_chunks (id, doc_id, seq, text) VALUES (?1, ?2, ?3, ?4)",
                    rusqlite::params![chunk_id, doc_id, seq as i64, text],
                )?;
                for term in terms::index_terms(text) {
                    tx.execute(
                        "INSERT OR IGNORE INTO rag_terms (term, chunk_id) VALUES (?1, ?2)",
                        rusqlite::params![term, chunk_id],
                    )?;
                }
            }
            total_chunks += chunks.len() as u32;
        }
        tx.commit().context("提交 RAG 重建事务失败")?;
        Ok(BuildSummary {
            docs: files.len() as u32,
            chunks: total_chunks,
            at_ms: now,
        })
    }

    /// 词法检索文档分块（与 graph 的摘要检索同路线：命中词项数排序）。
    ///
    /// 返回前 `limit` 条；`terms` 为空 → 空结果（调用方提前短路）。
    pub(in crate::plugins::memory) fn search_chunks(&self, terms: &[String], limit: usize) -> Result<Vec<RagSearchHit>> {
        if terms.is_empty() {
            return Ok(Vec::new());
        }
        let placeholders = terms.iter().map(|_| "?").collect::<Vec<_>>().join(", ");
        let sql = format!(
            "SELECT c.id, c.text, d.path, COUNT(DISTINCT t.term) AS hits
             FROM rag_terms t
             JOIN rag_chunks c ON c.id = t.chunk_id
             JOIN rag_docs d ON d.id = c.doc_id
             WHERE t.term IN ({placeholders})
             GROUP BY c.id
             ORDER BY hits DESC, c.id
             LIMIT {limit}"
        );
        let params = terms.iter().map(|s| s as &dyn rusqlite::ToSql).collect::<Vec<_>>();
        let conn = self.conn();
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(rusqlite::params_from_iter(params), |r| {
            Ok(RagSearchHit {
                chunk_id: r.get(0)?,
                text: r.get(1)?,
                source: r.get(2)?,
                hits: r.get(3)?,
            })
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    /// 索引内计数（stats 用）。
    pub(in crate::plugins::memory) fn rag_counts(&self) -> Result<RagIndexCounts> {
        let conn = self.conn();
        let docs: i64 = conn
            .query_row("SELECT COUNT(*) FROM rag_docs", [], |r| r.get(0))
            .optional()?
            .unwrap_or(0);
        let chunks: i64 = conn
            .query_row("SELECT COUNT(*) FROM rag_chunks", [], |r| r.get(0))
            .optional()?
            .unwrap_or(0);
        Ok(RagIndexCounts { docs, chunks })
    }
}

/// 递归收集 `*.txt` / `*.md`（按路径排序保证重建顺序稳定、分块 id 幂等）。
fn collect_docs(dir: &Path) -> Vec<(PathBuf, i64, i64)> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out; // 目录不存在 = 零文档（正常态）
    };
    let mut paths: Vec<PathBuf> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .collect();
    paths.sort();
    for path in paths {
        if path.is_dir() {
            out.extend(collect_docs(&path));
            continue;
        }
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        if !matches!(ext.to_ascii_lowercase().as_str(), "txt" | "md") {
            continue;
        }
        let meta = match std::fs::metadata(&path) {
            Ok(m) => m,
            Err(_) => continue,
        };
        let mtime_ms = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        out.push((path, meta.len() as i64, mtime_ms));
    }
    out
}

/// 分块：按空行分段 → 段超 `chunk_size` 字符再按滑窗切（步长 = size - overlap）。
///
/// 保守策略：不做句子级切分（中文标点歧义多），段边界优先、滑窗兜底；
/// 单字符级别的 overlap 保证跨块边界的句子至少完整出现在一个块里。
pub(in crate::plugins::memory) fn chunk_text(text: &str, chunk_size: usize, overlap: usize) -> Vec<String> {
    let overlap = overlap.min(chunk_size.saturating_sub(1));
    let normalize = |s: &str| s.trim().to_string();
    let mut out: Vec<String> = Vec::new();
    for para in text.split("\n\n") {
        let para = normalize(para);
        if para.is_empty() {
            continue;
        }
        let chars: Vec<char> = para.chars().collect();
        if chars.len() <= chunk_size {
            out.push(para);
            continue;
        }
        // 滑窗（按字符数，中文友好）：步长 = size - overlap
        let step = (chunk_size - overlap).max(1);
        let mut start = 0;
        while start < chars.len() {
            let end = (start + chunk_size).min(chars.len());
            let piece: String = chars[start..end].iter().collect();
            let piece = normalize(&piece);
            if !piece.is_empty() {
                out.push(piece);
            }
            if end == chars.len() {
                break;
            }
            start += step;
        }
    }
    out
}

#[cfg(test)]
mod tests;
