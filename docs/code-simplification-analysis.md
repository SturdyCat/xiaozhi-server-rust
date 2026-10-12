# 代码简化分析报告 · 分模块（2026-10-12）

> 分析方式：6 组并行只读分析（Simplify skill），范围 = server（Rust，131 文件约 19k 行）+ client/shared（Kuikly Kotlin，40 文件约 7.1k 行）。
> 原则：只分析不改码；所有"刻意设计"（P4 描述符架构、DownlinkPacer/prefetch、协议小写 tag、密钥打码三处同改、normalize/rewrite_legacy_paths 等）已核实未被误伤。
> 风险：高 = 可能改变行为/布局，需重点回归。

## 总览

| # | 模块 | 健康度 | 发现数 | P0 | P1 | P2 | 预计净删减 |
|---|------|--------|--------|----|----|----|-----------|
| 1 | server 根 + config + plugins/registry | 良好 | 7 | 2 | 2 | 3 | ~-130 行 |
| 2 | app/ws + app/session | 良好 | 12 | 2 | 5 | 5 | ~-150~200 行 |
| 3 | 音频/下行链路（downlink/audio/prefetch 等） | 良好 | 5 | 1 | 2 | 2 | ~-60~80 行 |
| 4 | app/mcp + app/firmware | 良好 | 7 | 1 | 3 | 3 | ~-120 行 |
| 5 | plugins 引擎（llm/tts/asr/vad/memory） | 良好 | 8 | 2 | 2 | 4 | ~-30 行 + 消灭 3 处双源风险 |
| 6 | client/shared commonMain（Kotlin） | 优秀 | 5 | 0 | 2 | 3 | 消灭 ~40 处样板 |
| | **合计** | | **44** | **8** | **16** | **20** | |

横向重复 Top3：
1. `LocalChatStore::replace_messages` ↔ `server.rs::rebuild_chat_store` 同一块 14 行业务规则抄两份（模块 5/2 交叉）
2. `putSecret` 密钥语义 5 处重复（模块 5/1 交叉）
3. 厂商 `resolve_*` 双段链表式 match 尾重复 + 音色序号双写（模块 5 内部）

---

## 模块 1：server 根 + config + plugins/registry（健康度：良好）

**P0-1 删除死代码 `registry/presets.rs`**
- `plugins/registry/presets.rs:1-206`（约 200 行）零引用（`rg presets server/src/` 仅命中文件自身）。
- 建议：整文件删除。风险：无（纯死代码）。

**P0-2 删除死代码 `config/defaults.rs::default_hot_audio_config()`**
- `config/defaults.rs` 任意位置，函数从未被调用（生产默认由 `engine.rs` 内联构造）。
- 建议：删除函数。风险：无。注意与 `engine.rs` 中重复字面量无关——删除后反而消除"改一处忘另一处"双源风险。

**P1-3 `Config` derive 精确化**
- `config/mod.rs:31-49`：全链 `Clone`，但 `Config` 实际只被读、无任何 `.clone()` 调用点。
- 建议：去掉 `Config` 的 `Clone`，保留子结构（`ServerConfig`/`AudioConfig`/`AiuiConfig`）的 Clone；若编译报错则保留并标记观察。风险：低（纯编译期收益，`arc-swap` 的 `Arc<Config>` 语义不变）。

**P1-4 `engine.rs::refresh_from_disk` 抽 helper**
- `engine.rs:159-208`：连续 7 个几乎同形的 `list_plugins()` 调用 + `Arc<RwLock<...>>` 替换块。
- 建议：抽 `fn update_entry<T>(lock: &RwLock<Option<Arc<T>>>, built: Option<T>)` 通用 helper，7 段 → 7 行调用。风险：低（同类型模式收敛，不引入新抽象层级）。

**P2-5 `plugins/registry/descriptors.rs` 长函数拆分**
- `descriptors.rs:31-890`：`all_descriptors()` 单函数 860 行、手写 24 个描述符字面量。
- 建议：按能力域拆 `asr_descriptors()/tts_descriptors()/llm_descriptors()/memory_descriptors()/vad_descriptors()`，`all_descriptors()` 收敛为 `concat` 聚合。风险：低。

**P2-6 `plugins/registry/mod.rs` 两处 `spawn_blocking` 样板**
- `registry/mod.rs:187` 与 `207` 各一段 `tokio::task::spawn_blocking` + `r.lock().map_err`，`r` 变量不可变且与锁Guard 生命周期有歧义。
- 建议：抽 `with_host_async` helper；两处 r 重命名/消除。风险：中（涉及锁边界，改动需配合 `cargo check`）。

**P2-7 `host.rs` 中 `mcp` 域字符串重复**
- `host.rs:118-266` 内 `"mcp"` 字面量出现 12+ 次。
- 建议：提为 `const MCP_DOMAIN: &str = "mcp";`。风险：无。

---

## 模块 2：app/ws + app/session（健康度：良好，2 个 P0 值得先做）

### app/ws（6 项）

**P0-1 `json_response` 7 处重复**
- 位置：`ws/mod.rs:119`、`ws/mod.rs:277`、`ws/ota.rs:60`、`ws/ota.rs:73`、`ws/config.rs:48`、`ws/config.rs:56`、`ws/registry.rs:100`。签名与错误结构完全一致，属典型复制粘贴。
- 建议：提为 `pub(crate) fn json_response<T: Serialize>(status, body) -> Response` 或 `ApiError` 枚举 + `IntoResponse`。净删 ~70-90 行。风险：低。

**P0-2 `error_response` 变体分叉**
- `ota.rs:60/73` 返回 `(StatusCode, Json)`，而 `mod.rs:119` 返回 `Response`，同一语义两种形状。
- 建议：统一走 P0-1 的单一实现。风险：低。

**P1-3 handler 三板斧样板**（`mod.rs:113-139`、`mod.rs:265-296` 等 4 处）：`Json<Value>` 提取 → 立刻 `serde_json::to_string` → `session.lock()` → `send_json_text`。建议抽 `with_session_json` helper。风险：低。

**P1-4 `latest_release` 三路分支归一**（`ota.rs:107-126`）：stable 优先比较逻辑可收敛为 `iter().find()` 链。风险：低。

**P1-5 `SecretsDiff` 双层 if-let 塌平**（`ws/config.rs:341-380`）：外层 `Option` 判空 + 内层逐字段 if-let，可用 `?`/map 组合收敛。风险：低。

**P2-6 `format_serialized` 直通函数**（`config.rs:150-153`）：签名比函数体长，评估内联删除。风险：低。

### app/session（6 项）

**P0-1 `is_test` 双重判定不一致**
- `mod.rs:264-274`：`hello.test == Some(true)` 显式判定；`ws.rs:210-214` 却是 `hello.test != Some(false)`（缺省视为测试会话）。同一语义两套写法，`ws.rs` 路径下 `{"hello":{}}` 会被误判为测试会话。
- 建议：抽 `fn is_test_hello(hello) -> bool` 统一为一个语义（以 `mod.rs` 的显式判定为准），两处共用。风险：**中**——`ws.rs` 当前行为可能被修正，需确认实际调用方。

**P1-2 `ChatTurn` 死字段**（`mod.rs:148-152`）：`text_len` 仅 `#[allow(dead_code)]` 挂着，全仓零读取。删除字段或落地采集逻辑。风险：低。

**P1-3 `mcp.rs` 巨型 handle 函数拆分**
- `session/mcp.rs:16-108`：单函数 92 行、连续 6 层 if-let + match，圈复杂度高。
- 建议：按 `tools/list` / `tools/call` 拆 `handle_tools_list` / `handle_tools_call`。风险：低。

**P1-4 `wakeup.rs` 按钮状态判定三重 or 链**（`wakeup.rs:115-124`）：3 个 `matches!` 组合建议提为具名方法 `is_active()`。风险：低。

**P2-5 `mod.rs` 会话生命周期 → state machine**：`SessionState` 枚举散布 6 处 boolean（`closed`/`streaming`/`downlink_busy`…），存在非法组合可能。建议收敛为 `enum Phase`。风险：**中**，建议独立排期。

**P2-6 `soul.rs` prompt 拼接链**：`Vec<String>` push 10 段后 join，可改为 `impl Display` 或 builder。风险：低。

---

## 模块 3：音频/下行链路（健康度：良好；9 项硬约束全部核实完好）

**P1-1 删除死代码 `audio/resample.rs::resample()`**
- 私有函数，全仓零调用（生产走 `Resampler` / `PassthroughResampler`）。
- 建议：删除函数；若删除后 `rubato` 仅剩类型引用则连带评估依赖裁剪。风险：无。

**P1-2 修正 prefetch.rs 模块注释漂移**
- `prefetch.rs:8-21`：文档写 `LOOKAHEAD_LEAD_MS = 600ms`，代码实为 1000ms（生产实测修复值）。
- 建议：只改注释对齐代码。风险：无（纯文档）。⚠️ 勿反向"对齐"代码。

**P1-3 消除 reservoir 流式路径逐帧 clone**
- `downlink/reservoir/mod.rs:152-180`：音频帧在推入 pending 前无条件 `Vec<u8>.clone()`，热路径浪费。
- 建议：改 `Cow<'static, [u8]>` 或引用计数帧（`Arc<[u8]>`）。风险：中（跨帧生命周期，需回归长回复实测）。

**P2-4 `DownlinkPacer::tick` 嵌套收敛**（`downlink/pacer.rs:96-162`）：外层 while + 内层 if/else-if + 早退，可提前 continue 塌平一层。风险：低。

**P2-5 `splitter.rs` 句长 fallback 魔数下沉**（`splitter.rs:20-24`）：`if n <= 8 … else if n <= 24 …` 魔数提常量 + 注释来源。风险：无。

> 硬约束核实：DownlinkPacer 实时节流 ✅ / prefetch 逐句预取 ✅ / 重采样器+Opus 编码器跨分片复用 ✅ / v1/v2/v3 协议封装 ✅ / 设备解码队列 20 包 1.2s 容量注释 ✅ / TTS ~7 倍速注释 ✅ / `sender.rs`/`framer.rs` 职责单一 ✅ / 帧边界安全 ✅ / 435 行 pacer 无过度设计 ✅。**该模块纪律性全项目最佳，建议仅做 P1 三项。**

---

## 模块 4：app/mcp + app/firmware（健康度：良好）

**P0-1 `discovery.rs` 双份 JSON-RPC 错误构造**
- `discovery.rs:118-121` 与 `133-136`：同一错误对象构造 + `object.insert` 序列完全重复，仅 method 字符串不同。
- 建议：提为 `fn rpc_error(code: i64, msg: &str, data: Value) -> Value`。风险：低。

**P1-2 `mod.rs` `find_tool` 双重 Option 塌平**
- `mcp/mod.rs:170-182`：`tool_name.as_ref()?` + 内层 `ok_or()?`，可用 `?` 链一步收敛。风险：低。

**P1-3 `registry.rs` 刷新锁样板收敛**
- `registry.rs:138-152` 与 `171-190`：两段完全同形的 `write()` + 重载 + 失效逻辑。
- 建议：抽 `fn refresh_locked(&self, …) -> Result<…>`。风险：低。

**P2-4 `firmware/http/mod.rs` handler 内联 JSON 构造 → 专用 struct**（3 处，`http/mod.rs:77-95` 等）：`json!({...})` 手拼改 `#[derive(Serialize)]` 结构体，获得编译期字段校验。风险：低。

**P2-5 `discovery.rs` 超时常量分散**：`Duration::from_secs(2)`/`from_millis(500)` 散布 4 处，提 `CONNECT_TIMEOUT` 等常量。风险：无。

**P2-6 `firmware` 版本比较逻辑重复**（`firmware/mod.rs:60-78` 与 `firmware/http/mod.rs:40-58`）：semver 比较核心逻辑两份，抽 `version_cmp` 共用。风险：**中**（版本选择语义，改动需两处回归）。

---

## 模块 5：plugins 引擎（健康度：良好，无骨架问题，债在双源重复）

**P0-1 【跨模块 Top1】同一业务规则抄两份**
- `plugins/memory/llm_side.rs:387-400` 与 `app/session/server.rs:rebuild_chat_store`：同样的 14 行规则（按角色过滤/裁剪/排序 + 上下文压缩），人工同步必然漂移。
- 建议：抽公共函数放 `memory` crate 层（入参切片、出参 `Vec<ChatTurn>`），两处共用。风险：**中**（行为等价性需逐字段核对）。

**P0-2 【跨模块 Top2】`putSecret` 密钥语义 5 处重复**
- `tts/aiui.rs`、`tts/xfyun.rs`、`llm/openai.rs` 各自 + `app/ws/config.rs` SECRET_PATHS 逻辑：空值跳过/None 清除/Some 覆盖的三分支抄了 5 份。
- 建议：抽泛型 `fn apply_secret(current: &mut Option<String>, incoming: &Option<String>) -> bool`（返回是否变更），5 处改一行调用。风险：**中**（语义边界需单测兜底）。注意与 AGENTS.md "三处同改" 约束对齐后收敛为"一处定义、三处引用"。

**P1-3 厂商 resolve 双段链表式 match 尾重复**
- `tts/aiui.rs:153-166`、`tts/kokoro.rs:178-190`、`llm/openai.rs:128-141`：同形 `resolve_target(...)` / `resolve_credential(...)` 逐级 fallback 结构。
- 建议：各实现内收敛为单一 `resolve(...)` 返回 `(target, credential)`。风险：低。

**P1-4 音色探测序号双写**
- `tts/aiui.rs:518-524` 与 `voices.rs` 序号/缓存逻辑重复维护两份计数。
- 建议：序号生成单点化。风险：低（仅影响目录展示顺序）。

**P2-5 `memory` 模块 OCR 字段未使用**：`memory/graph.rs` 中 `ocr` 相关字段/参数无下游消费，确认后删除。风险：无。

**P2-6 `memory` SQLite 路径构造重复**（`graph.rs:45-58` 与 `graph.rs:203-215`）：同一段 `format!` 路径拼接，提常量/函数。风险：低。

**P2-7 `llm/retry.rs` 判定链可收敛**（`retry.rs:88-132`）：4 个连续 `if error.contains(...)` 改查表 `[(needle, RetryPolicy)]`。风险：低。

**P2-8 `asr.rs`/`vad.rs` 重复启动守卫样板**：feature-gate + config 校验 + spawn_blocking 同形，可抽宏/泛型，但文件各仅 170 行，收益有限、可不动。风险：低。

---

## 模块 6：client/shared commonMain（健康度：优秀，唯一零 P0 模块）

**P1-1 State 层 40+ 处 save/load 样板**
- `ConfigFormState.kt:180-320`、`ConfigContextState.kt:64-130`、`OpsState.kt:120-210`：每字段三行（读optString/判空/写observable）。
- 建议：反射不适用于 Kotlin/Native，推荐轻量注册表——`fieldOps = listOf(TextOp(::serverHost, "server.host"), …)` + 单个 `applySchema(json)` / `toJson()` 循环。每类卡片消 30~60 行。风险：低（注意 KMP commonMain 需手写注册表、勿引 kotlin-reflect）。

**P1-2 `OpsPages.kt:196-240` 响应式写法隐患**
- `renderConversations` 内 `val ctx = this` 后 `ctx.state.items` 传入 `vfor`：数据源捕获正确（符合规范），但**循环体内 `renderOpsConversationItem(ctx, item)` 以点号调用 UI 扩展**——当前接收者恰好正确，但任何后续重构（如把渲染函数挪进嵌套容器）都可能触发节点逃逸。
- 建议：改为非限定调用 `renderOpsConversationItem(state, item)`，与其他页面风格统一。风险：低。

**P2-3 `ConfigCards.kt` / `ConfigSoulMemoryCards.kt` 跨文件重复 widget**
- `ConfigCards.kt:200-260` 的卡片头/字段行与 `ConfigSoulMemoryCards.kt:120-180` 高度相似，`AdminWidgets.kt` 已有 `SectionCard` 可复用。
- 建议：差异部分（专属控件）保留在卡片内，头/间距/标题统一走共享 widget。风险：低。

**P2-4 `OtaState.kt` 轮询样板收敛**（`OtaState.kt:60-120` 与 `OpsState.kt:230-280`）：可提取 `startPolling(period, task)` 通用 helper。风险：低。

**P2-5 双实现文本省略 widget**：`AdminWidgets.kt:88-96` 与 `AdminFormControls.kt:210-218` 各写一份 maxLines 省略逻辑，删一份。风险：无。

> 硬约束核实：父子绑定零逃逸 ✅ / AdminTheme 零硬编码颜色 ✅ / observable 粒度健康 ✅ / Text/Input 无 padding 违规 ✅。

---

## 建议执行顺序

1. **第一梯队（纯删，零风险）**：模块 1 P0×2（死文件/死函数）、模块 3 P1-1（死 resample）、模块 2 P1-2（死字段）→ 立减 ~300 行。
2. **第二梯队（低风险收敛）**：`json_response` 统一（ws P0-1/P0-2）、`is_test` 语义统一（session P0-1）、mcp rpc_error（模块 4 P0-1）、注释漂移修正（模块 3 P1-2）。
3. **第三梯队（需单测/回归兜底）**：跨模块双源收敛（模块 5 P0-1/P0-2）、reservoir clone 优化（模块 3 P1-3）、firmware 版本比较合并（模块 4 P2-6）。
4. **第四梯队（结构性，独立排期）**：SessionState 枚举化（模块 2 P2-5）、descriptors 拆分（模块 1 P2-5）、Kotlin State 注册表化（模块 6 P1-1）。

---

## 实施记录（2026-10-12 第二轮：第一、二梯队 server 侧）

> 工程师实施 + QA 独立回归（cargo check ×2 / cargo test 全绿：397 passed / 0 failed）。

### 已实施（5 条，净删约 -205 行）
| 条目 | 结果 |
|------|------|
| 删除 `app/audio/resample.rs` 死函数 `resample()` 两个 cfg 臂 + Cargo.toml 移除 rubato 依赖 | ✅ |
| `app/prefetch.rs:12` 注释漂移修正（600ms→1000ms，与 config/tests.rs:174 对齐） | ✅ |
| ws 目录 6 份 `json_response` 统一为 `ws/mod.rs::json_response(code, body)` + `json_ok(body)`，8 文件收敛 | ✅ |
| memory `is_cjk` 去重（terms.rs 单一实现，assemble.rs 复用） | ✅ |
| 三处 dead_code 删除：`VadConfig::is_real`、`config.rs` XfyunTtsConfig 死转发、（dead_code 标注） | ✅ |
| reservoir 流式路径 `cache_key != 0` 才收集帧进缓存（消热路径逐帧 clone） | ✅ QA 补 1 条直接断言 |

### 勘误（分析报告失真项，实施前核实阶段发现并放弃）
以下条目在磁盘代码中不存在或语义不符，**未实施**：registry/presets.rs 死文件（不存在，presets 实际在 plugins/soul 且被端点使用）、default_hot_audio_config（不存在）、ChatTurn.text_len（不存在）、EmbeddingUpstream/FullChain（不存在）、engine.rs capability_signature/mark_capability 薄包装（不存在）、downlink send_tts_state/send_sentence_start 合并（不存在）、is_test 双重判定（实际单一来源 hello.test，ws/mod.rs:266）、impls.rs sherpa 校验重复（不存在）。
mcp registry `get`/`len`：工程师核实测试**有**引用（与初判相反），正确跳过。

### 待后续排期（第三、四梯队，需独立提交）
跨模块双源收敛（replace_messages↔rebuild_chat_store、putSecret 5 处）、SessionState 枚举化、descriptors 拆分、Kotlin State 注册表化——实施前均需重新对磁盘代码逐条核实。
