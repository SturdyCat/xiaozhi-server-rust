# 第 2 轮设计：P1-10 MCP 清单驱动 + P1-9 本地 RAG 后端

> 依据：`docs/plans/p1-features-prd.md` §3.4/§3.5 + 探索报告（2026-10-12，Explore-1/Explore-2）。
> 第 1 轮（P1-6 `4df249f` / P1-7 `e8843ce`）已交付注册表地基，本轮两项在其上叠加。

---

## §1 P1-10 MCP 清单驱动动作模型（先做，改动面小）

### 1.1 决策原语：三态清单查询（`workflow/mcp/registry.rs`）

```rust
impl DeviceTools {                                   // L76 impl 块
    /// 工具清单是否含某工具（纯内存遍历，零 I/O）。
    fn has_tool(&self, name: &str) -> bool { self.tools.iter().any(|t| t.name == name) }
}

impl McpRegistry {                                   // L124 impl 块
    /// 指令闸门决策查询（三态）：
    /// None        = 清单未知（未登记 / 探测失败且零工具）→ 调用方保持试探路径；
    /// Some(false) = 清单在手且明确没有该工具 → 调用方跳过调用直接降级；
    /// Some(true)  = 清单在手且含该工具 → 正常调用。
    pub fn has_tool(&self, device_id: &str, tool: &str) -> Option<bool> {
        let dt = self.get(device_id)?;               // 未登记 → None
        if dt.error.is_some() && dt.tools.is_empty() {
            return None;                             // 探测失败 = 未知（保守试探）
        }
        Some(dt.has_tool(tool))
    }
}
```

"清单为空但探测成功"（`error=None, tools=[]`）= 设备**明确**没有该工具 → `Some(false)`。
`get()`（L185）已 clone 整条，决策点是内存读可接受（PRD 验收 3 零新阻塞点）。

### 1.2 指令闸门四分支（`workflow/session/command.rs::stand_down_via_mcp` L87-122）

开头插入一次查询，三分支变四分支：

```
has_tool(device_id, STOP_LISTENING_TOOL):
  Some(false) → warn 日志（含降级原因）→ close_requested = true，【零次 tools/call】
  Some(true) | None → 现有三分支原样（Ok 成功 / Ok 失败含 -32602 / Err）
```

- `self.device_id`（session.rs:111）与 `self.engines.mcp` 在 `impl Session` 子模块内直接可访问。
- `mcp_enabled=false`（旧固件/测试台）：设备未登记 → `None` → 现有路径零回归（验收 6）。
- 唤醒问候链路（`session/wakeup.rs`）**不动**（PRD：speak_canned 不动）。
- 三条触发路径（普通/AIUI/注入）共用 `decide_user_text → handle_command_gate`，改一处全覆盖。

### 1.3 管理面标记（`DeviceTools::to_json` registry.rs:86-103）

新增字段 `command_gate`（三态字符串）：

| 值 | 条件 | 前端展示 |
|---|---|---|
| `"compatible"` | `has_tool(STOP_LISTENING_TOOL)` | 指令闸门兼容 |
| `"will_degrade"` | 清单在手但不含 | 说「退下」将降级断开 |
| `"unknown"` | 探测失败/未登记（该设备不出现在清单时无此行） | 未知 |

### 1.4 前端（`McpToolsState.kt` L54-66 / `McpToolsCard.kt` deviceBlock L80-142）

- `DeviceToolsRow` 加 `commandGate: String?` 解析；deviceBlock 在在线/离线徽标旁并列显示标记文案。

### 1.5 文案（`registry/fields_command.rs` enabled help L18-20）

补一句：「设备侧动作由已发现的 MCP 工具清单驱动：清单含 stop 工具则保持连接，不含则跳过调用直接断开重连（老固件少一次注定失败的往返）。」

### 1.6 护栏测试

- `workflow/mcp/registry/tests.rs`：`has_tool` 四态（未登记=None、探测失败=None、成功无此工具=Some(false)、含=Some(true)）+ `to_json.command_gate` 三态。
- `workflow/session/mcp/tests.rs`：会话级——探测成功清单含 stop → tools/call 发出；清单不含 → 零 tools/call 报文 + close；探测失败 → 试探报文发出（不劣化）。
- 固件契约测试（`workflow/mcp/tests.rs:171`）不动。

---

## §2 P1-9 本地 RAG 后端（`[memory].engine = "rag"`）

### 2.1 配置层（`plugins/memory/config.rs`）

- `MemoryBackend::Rag` 变体（doc：本地文档 RAG，词法检索，零外网零 LLM）；
  `parse()` 加 `"rag"`、`expecting`/报错文案加 rag、`engine_id()` 加臂、`MEMORY_ENGINE_VALUES = ["graph","store","rag","none"]`。
- `RagConfig` 子结构（照 ExtractorConfig 惯例：字段级 serde default + **手写 Default**）：

| 字段 | 默认 | 校验 |
|---|---|---|
| `docs_dir: String` | `/data/rag-docs` | 非空（engine=rag 时）；不存在/为空是**正常状态**（验收 2），不报错 |
| `chunk_size: u32` | `512` | `>= 64` |
| `chunk_overlap: u32` | `64` | `< chunk_size` |
| `top_k: u32` | `4` | `1..=20` |

- `MemoryConfig::default` 加 `rag: RagConfig::default()`；`validate()` 加 engine==Rag 分支；`warning()` 保持 graph-only（docs_dir 为空是正常态不是降级）。
- mod.rs re-export 加 `RagConfig`。

### 2.2 索引层（新文件 `plugins/memory/rag.rs` + `rag/` 子目录）

**表结构**（模块私有 `ensure_rag_tables`：`CREATE TABLE IF NOT EXISTS` 幂等，**零 DDL 版本路线**——`LATEST_VERSION` 保持 2，rag 三表只被 rag 后端触碰，graph/store 库无此表也正常）：

```sql
rag_docs   (id TEXT PRIMARY KEY, path TEXT UNIQUE NOT NULL, title TEXT,
            size INTEGER, mtime_ms INTEGER, chunk_count INTEGER, indexed_at_ms INTEGER)
rag_chunks (doc_id TEXT REFERENCES rag_docs(id) ON DELETE CASCADE,
            seq INTEGER, text TEXT NOT NULL, PRIMARY KEY(doc_id, seq)) WITHOUT ROWID
rag_terms  (term TEXT, chunk_id TEXT REFERENCES rag_chunks(doc_id...)
            -- 检索需要 chunk 级主键：rag_chunks 用 id TEXT PRIMARY KEY + (doc_id, seq) UNIQUE
```

（定稿：`rag_chunks(id TEXT PRIMARY KEY, doc_id ... , seq INTEGER UNIQUE(doc_id,seq), text)`；`rag_terms(term, chunk_id REFERENCES rag_chunks(id) ON DELETE CASCADE, PRIMARY KEY(term, chunk_id)) WITHOUT ROWID` + `idx_rag_terms_lookup(term)`。）

**分词**：直接复用 `terms::index_terms` / `query_terms`（CJK bigram + ASCII word，与 graph 同口径）。

**分块**（保守策略）：按空行分段 → 段超 `chunk_size` 字符再按 `chunk_size` 滑窗（步长 `chunk_size - chunk_overlap`）→ 尾段去重。

**打分**：v1 = 命中词项数 `COUNT(DISTINCT t.term) hits DESC`（与 graph `search` L370-469 完全同路线）；BM25 全量打分（df/词频统计）列 P2。

**RagMemory 结构**：`{ cfg: MemoryConfig, store: Arc<Store>, state: Mutex<IndexState> }`——**共用 `db_path` 同一 SQLite 文件**（graph⇄rag 切换数据互不删，验收 7 自动满足；对话原文走既有 `gm_messages`，state="stored"，验收 P0「对话沉淀不废弃」）。

```rust
struct IndexState { building: bool, last_build: Option<BuildSummary>, last_error: Option<String> }
```

**重建**（`rebuild(force) -> tokio spawn 后台`）：
- 扫 `docs_dir` 下 `*.txt/*.md` → **单事务**内逐文档 DELETE+INSERT（SQLite 事务原子性：构建期间 recall 读到**旧完整索引**，完成后切换到新索引——PRD 验收 3 的"已就绪内容"以更强语义满足，且无半成品索引窗口）；
- 幂等：全量重建先清后建，分块总数由内容决定（验收 4）；
- `docs_dir` 不存在 → 空索引（零文档），不报错（验收 2）；
- SQLite 同步操作一律 `run_blocking`（spawn_blocking 铁律）。

**MemoryProvider 实现**（模板 = store_only.rs）：
- `name()` = "rag"；`record` = 抄 store_only（active 门 + 空轮跳过 + `insert_turn_state(..., "stored", now)`）；
- `recall` = graph.rs L235-304 口径：`recall_enabled` 门 → `query_terms` 空提前返回 → `timeout(recall_budget_ms)` 包 run_blocking 检索（top_k 上限）→ 超时 warn + `none()`（验收 5）→ 命中组装 `block`（`【来源 path】\nchunk 正文` 逐条拼接，`assemble::estimate_tokens` 口径）→ `RecallResult { hits, matched_terms, elapsed_ms, block, error: None }`，**绝不 Err**；
- `extract_pending` = 恒 default（零 LLM）；`maintain` = 抄 store_only（保留策略对对话原文生效，rag 索引不动）；
- `stats` = engine:"rag"、lexical_only:true、extractor:"" + 对话侧 counts/turns + rag 专属计数（`MemoryStats` 加 `rag_docs/rag_chunks/rag_state: Option<…>`，to_json 条件透出——memory_status 端点自动可见）；
- `clear` = 只动对话原文（与 store 同款）；**rag 索引由重建管理**，clear 不碰（语义：清空对话 ≠ 删知识库）。

### 2.3 工厂与注册表

- `build_memory` 加 `MemoryBackend::Rag => Ok(Arc::new(RagMemory::new(cfg)?))`（不触碰 llm）。
- `sel_memory_rag` + `MEMORY_RAG` 描述符（id `memory.rag`、`requires=&[]`、`warn_if=None` v1、fields=MEMORY_FIELDS、sig/validate/build 共用）→ REGISTRY 14 条。
- `fields_context.rs`：engine help 文案加 rag；新增 `MEMORY_RAG_FIELDS`（docs_dir/chunk_size/chunk_overlap/top_k，路径 `["memory","rag"]`）+ `CONTEXT_COMMON_FIELDS` 加条目。
- `registry/tests.rs` 护栏 `memory_backend_values_match_registry_impls`：impl_ids 断言加 `"memory.rag"`（其余三段自动对齐）。

### 2.4 管理面

- `POST /api/memory/rag/reindex`（ws/memory.rs + ws/mod.rs:140 后注册）：engine 非 rag → 400 中文报错；否则触发后台重建 → 立即返回 `{ok, building:true}`；进度/结果查 `/api/memory/status`（stats 已透出）。
- reindex 经 `MaintainOpts`-free 直连：`engines.memory()` 返回 `Arc<dyn MemoryProvider>` 无 reindex 方法——**方案**：handler 读 `engines.config().memory.engine` 判 rag 后，经全局注册的单例句柄（`build_memory` 返回的 Arc 由 Engines 持有；在 Engines 上加 `rag_index()` 辅助——若 downcast 困难，则 trait 加带默认空实现的方法 `fn reindex(&self) -> bool { false }`，RagMemory 覆写）。定稿：**trait 加默认空实现**（零 downcast、零新全局状态，与 trait 现有风格一致）。

### 2.5 前端（Kuikly 传统 DSL）

- `ConfigContextState.kt`：`memoryRagDocsDir/ChunkSize/ChunkOverlap/TopK` 字段 + `memoryBackendOptions` 加 `"rag" to "本地文档 RAG（扫描目录建索引）"` + putJson/fill 的 `rag` 子对象 + `reindex()` 动作（照 maintain L528-543 模板）。
- `ConfigSoulMemoryCards.kt`：rag 选中时 `vif({ sm.memoryBackend == "rag" })` 说明文案（照 store L295-303）+ rag 参数四字段（vif rag 内）+ 「重建索引」secondaryButton + 状态文本（照 actionRow/maintainMsg 模式，不新造卡）。

### 2.6 护栏测试（`rag/tests.rs`，FakeOk Llm 不需要——rag 零 LLM）

1. 摄入 md 含「售后服务电话是 400-123-4567」→ 重建 → `recall("售后电话")` 命中该分块、block 含号码与来源文件名（验收 1）；
2. `docs_dir` 空 → recall 恒空、stats 0 文档（验收 2）；
3. record 照常落 stored、检索只查文档表（互不污染）；
4. 连续两次全量重建分块数一致（验收 4）；
5. graph⇄rag 切换（同 db 文件）：图记忆数据与 rag 索引各自保留（验收 7）；
6. 维护：保留策略对对话原文生效、索引不删。

---

## §3 任务表与顺序

| # | 任务 | 项 | 文件 |
|---|---|---|---|
| R2-1 | registry has_tool 三态 + command_gate 字段 + 测试 | P1-10 | workflow/mcp/registry.rs(+tests) |
| R2-2 | stand_down_via_mcp 四分支 + 测试 | P1-10 | workflow/session/command.rs、session/mcp/tests.rs |
| R2-3 | fields_command 文案 + 前端标记 | P1-10 | fields_command.rs、McpToolsState/Card.kt |
| R2-4 | RagConfig + MemoryBackend::Rag + validate | P1-9 | memory/config.rs(+tests) |
| R2-5 | rag.rs 索引/分块/检索 + RagMemory | P1-9 | memory/rag.rs(+tests) |
| R2-6 | registry 挂载 + 护栏同步 | P1-9 | registry/{impls,fields_context,mod,tests}.rs |
| R2-7 | reindex 端点 + stats 透出 | P1-9 | app/ws/{memory,mod}.rs、memory/types.rs |
| R2-8 | 前端 rag 配置 + 重建按钮 | P1-9 | ConfigContextState/ConfigSoulMemoryCards.kt |
| R2-9 | 全量验证 + 双 commit（P1-10、P1-9 各一） | — | — |

**验证口径**：`cargo test` 全绿 + 双 feature check + gradle compileKotlinJs/compileKotlinMacosArm64；每项独立 commit（中文标题 + 双语摘要，本地 main 不 push）。
