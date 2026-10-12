//! 记忆插件的对外数据类型：召回/写入/抽取/维护的输入输出与状态快照。
//!
//! 本文件由 `plugins/memory/mod.rs` 拆分而来（纯移动，不改任何逻辑与字段语义）；
//! 对外路径经 `mod.rs` 的 `pub use` 保持不变（`crate::plugins::memory::RecallResult` 等）。

/// 召回结果（**从不返回 `Err`**：记忆失败不该影响对话，错误只用于日志/状态页）。
#[derive(Debug, Clone, Default)]
pub struct RecallResult {
    /// 已渲染的注入正文；`None` = 无命中/关闭/失败。
    pub block: Option<String>,
    pub hits: usize,
    pub matched_terms: usize,
    pub elapsed_ms: u128,
    /// 失败原因（仅用于日志与状态页；会话侧忽略）。
    pub error: Option<String>,
}

impl RecallResult {
    pub fn none() -> Self {
        Self::default()
    }

    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "hits": self.hits,
            "matched_terms": self.matched_terms,
            "elapsed_ms": self.elapsed_ms,
            "injected": self.block.is_some(),
            "preview": self.block,
            "error": self.error,
        })
    }
}

/// 一轮已完成的对话（写入记忆的输入）。
#[derive(Debug, Clone)]
pub struct CompletedTurn {
    pub scope: String,
    pub session_id: String,
    pub user_text: String,
    pub assistant_text: String,
}

/// 抽取批次的统计。
#[derive(Debug, Clone, Default, Copy, PartialEq, Eq)]
pub struct ExtractOutcome {
    pub attempted: u32,
    pub succeeded: u32,
    pub failed: u32,
}

/// 维护结果（`GET /api/memory/status` 与「立即维护」动作都要能看到"到底做了什么"）。
#[derive(Debug, Clone, Default, Copy, PartialEq, Eq)]
pub struct MaintainOutcome {
    /// 是否真的执行了（`false` = 未到维护间隔，被跳过）。
    pub ran: bool,
    /// 从隔离放回队列的轮次数。
    pub released: u64,
    /// 保留策略处理的原文条数（`dry_run` 时为"将会删除"的数量）。
    pub deleted: u64,
    pub dry_run: bool,
}

/// 维护调用的选项（会话周期调用 vs 管理页「立即维护」）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MaintainOpts {
    /// `true` = 忽略 `maintenance_interval` 立即执行（管理页动作）。
    pub force: bool,
    /// `true` = 只演练不删除（先看清会处理多少条）。
    pub dry_run: bool,
}

impl MaintainOpts {
    /// 会话每轮调用：由实现按间隔自行决定是否真的跑。
    pub fn periodic() -> Self {
        Self::default()
    }

    /// 管理页「立即维护」。
    pub fn forced(dry_run: bool) -> Self {
        Self {
            force: true,
            dry_run,
        }
    }
}

impl MaintainOutcome {
    pub fn skipped() -> Self {
        Self::default()
    }

    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "ran": self.ran,
            "released": self.released,
            "deleted": self.deleted,
            "dry_run": self.dry_run,
        })
    }
}

/// 记忆状态快照（`GET /api/memory/status` 与管理页状态卡）。
#[derive(Debug, Clone, Default)]
pub struct MemoryStats {
    /// 引擎 id（`graph` / `store` / `none`；P4 起 wire 键也叫 `engine`）。
    pub engine: &'static str,
    /// 记忆是否真正生效（启用 + 后端可用）。
    pub active: bool,
    pub db_path: String,
    pub db_bytes: u64,
    /// 累计轮次数（P1-7：store 后端的核心计数"积累了多少轮"；graph 一并填充，同口径可比）。
    pub turns: i64,
    pub messages: i64,
    pub memories: i64,
    pub pending: i64,
    pub quarantined: i64,
    /// `extraction_state='stored'` 的消息条数（store 积累的原文；graph 恒 0）。
    pub stored: i64,
    /// 各作用域（哪台设备有多少条记忆）。
    pub scopes: Vec<(String, i64)>,
    /// 当前是否为"无 embedding 的词法模式"（UI 据此显示降级原因）。
    pub lexical_only: bool,
    /// 抽取用的模型路由（**不含密钥**），如 `https://… · gpt-4o-mini`。
    pub extractor: String,
    /// RAG 专属状态（P1-9；仅 rag 后端填充，其他后端 None → JSON 里省略）。
    pub rag: Option<RagStats>,
    pub error: Option<String>,
}

/// RAG 文档索引状态（`/api/memory/status` 的 `rag` 段；管理台「文档索引」卡数据源）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RagStats {
    /// 当前索引内的文档数 / 分块数（重建中显示旧值 + building 标记）。
    pub docs: i64,
    pub chunks: i64,
    /// 是否正在后台重建。
    pub building: bool,
    /// 最近一次成功重建。
    pub last_build_at_ms: Option<i64>,
    pub last_build_docs: Option<u32>,
    pub last_build_chunks: Option<u32>,
    /// 最近一次重建失败原因。
    pub last_error: Option<String>,
}

impl RagStats {
    fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "docs": self.docs,
            "chunks": self.chunks,
            "building": self.building,
            "last_build_at_ms": self.last_build_at_ms,
            "last_build_docs": self.last_build_docs,
            "last_build_chunks": self.last_build_chunks,
            "last_error": self.last_error,
        })
    }
}

impl MemoryStats {
    pub fn to_json(&self) -> serde_json::Value {
        let mut v = serde_json::json!({
            "engine": self.engine,
            "active": self.active,
            "db_path": self.db_path,
            "db_bytes": self.db_bytes,
            "turns": self.turns,
            "messages": self.messages,
            "memories": self.memories,
            "pending": self.pending,
            "quarantined": self.quarantined,
            "stored": self.stored,
            "scopes": self.scopes.iter().map(|(s, n)| serde_json::json!({"scope": s, "memories": n})).collect::<Vec<_>>(),
            "lexical_only": self.lexical_only,
            "extractor": self.extractor,
            "error": self.error,
        });
        // rag 段只在 rag 后端时出现（graph/store 的 JSON 保持原形，前端按存在性判断）
        if let Some(rag) = &self.rag {
            v["rag"] = rag.to_json();
        }
        v
    }
}
