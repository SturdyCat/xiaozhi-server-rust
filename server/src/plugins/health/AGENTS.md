# 引擎健康自检维护

[探测实现](<../health.rs>)按配置 plan 产出各能力状态，[宿主](<../host.rs>)聚合并上报，[健康端点](<../../app/ws/health.rs>)只做只读投影。探测只查本地文件与环境，不构建引擎、不请求外网，幂等无副作用；deviation 必须带原因与“怎么修”。回归见[测试](<tests.rs>)。

---

## 健康自检设计（历史记录）

> **历史设计 / 实施记录**：由原《P1 第一轮设计》长文按职责抽取，保留原章节编号及当时的结论，不代表所有提案均已落地。
> 原章节中的 `§N` 请通过[完整章节索引](<../../../../docs/p1-round1-design.md>)定位；当前行为先读本文件上半部分、源码与测试。

# P1 第一轮增量设计：P1-6 引擎健康自检 + P1-7 记忆多后端注册表

> 作者：架构师 高见远　日期：2026-10-12
> 上游输入：`docs/p1-features-prd.md` §3.2（P1-6）/ §3.3（P1-7）；`docs/overall-implementation-plan.md` §1 原则约束；`AGENTS.md` 工程约定。
> 代码现状已逐一核实（行号见文内），本设计为**增量设计**：不改既有 402 个测试的语义，不动 `MemoryProvider` trait 签名，不动 `Config::load` 核心链路（那是 P1-5 末轮的事）。

---

## 0. 总览

| 项 | P1-6 健康自检 | P1-7 记忆注册表 |
|---|---|---|
| 服务端新文件 | `plugins/health.rs`（探测）、`app/ws/health.rs`（端点）+ 两个 tests | `plugins/memory/store_only.rs`（+tests） |
| 服务端改动 | `host.rs`（聚合 helper 提取）、`ws/mod.rs`（路由） | `memory/config.rs`、`memory/mod.rs`、`memory/store.rs`、`memory/types.rs`、`registry/impls.rs`、`registry/mod.rs`、`registry/fields_context.rs` |
| 前端 | 新 `EngineHealthState.kt` / `EngineHealthCard.kt` + 三壳接线 | `ConfigContextState.kt` / `ConfigSoulMemoryCards.kt` 各加一处 |
| 新表/新列 | 无 | **无**（`gm_messages.extraction_state` 增加取值 `'stored'`，非 DDL） |
| 新依赖 | **0** | **0** |
| 核心风险 | 口径分叉（与 `/api/plugins` 不同源） | `active()` 语义扩展波及热切换/会话写入路径 |

两项功能代码面几乎不重叠（P1-7 不动 registry 的 Capability 枚举，P1-6 不动 memory 模块），服务端部分可全程并行。

---

## 1. 实现方案

### 1.1 P1-6 引擎健康自检

#### 1.1.1 现状与剩余工作

PRD 现状校准已确认：`REGISTRY` 12 条描述符齐全（`impls.rs:428-441` 对应 `registry/mod.rs:428-441`），`PluginHost::plan` 能对全部 11 个 capability 算 phase，`GET /api/plugins` 已按能力聚合输出（`host.rs:346 capabilities_json`）。**剩余工作 = 运行期探测聚合端点 + 管理台露出 + 跳转闭环**，不补描述符。

关键认知：`PluginDescriptor::validate`（`mod.rs:408`）是**纯配置校验、无 I/O**——`plan()` 正因此能在无 sherpa feature 的 `cargo test` 里全量断言状态逻辑。运行期探测（模型文件存在性 / db 目录可写）绝不能塞进 `validate`。

#### 1.1.2 取舍一：探测钩子放哪 —— 独立 `plugins/health.rs` 模块（**选**） vs `PluginDescriptor` 加 `healthcheck` 字段

**选独立模块，理由：**

1. **保住 `plan()` 的无 I/O 性质**。模块注释（`host.rs:113-116`）明说 plan"不触碰任何引擎 ctor ⇒ 可在无 sherpa feature 的 cargo test 里断言全部状态逻辑"。描述符里任何一个允许 I/O 的钩子都会模糊这条边界——下一个工程师很可能在 healthcheck 里写网络探测，`cargo test` 依然绿（默认 feature 下探测分支不跑），上了真机才炸。
2. **探测是 capability 级，不是 impl 级**。11 个 capability 只有 4 个有探测项（asr/vad/tts/memory）；tts 的探测只在 kokoro 被选中时才有意义——这是"选中配置"的函数而非描述符的静态属性。塞描述符 = 12 个描述符多一个大多为 `None` 的字段。
3. **描述符已经四职**（validate/warn_if/signature/build），再加一职会让"新增实现要写哪些函数"的清单继续膨胀。
4. **capability 级 `match` 天然穷尽**——`Capability` 是闭枚举，health.rs 的 match 漏一个变体编译不过，这就是免费的漂移护栏，不靠测试兜底。

**预留演进**：P1-9 的 RAG 后端若需要"索引目录存在性"这类**实现私有**探测，届时再给描述符加 `healthcheck: Option<fn(&Config) -> Option<String>>`（与本模块的 capability 级探测合并取最坏）；本轮不做。

#### 1.1.3 服务端模块设计

**新文件 `server/src/plugins/health.rs`**（capability 级探测，全部只做本地 fs 检查）：

```rust
//! 引擎健康探测：capability 级的运行期轻量检查（仅本地文件系统，无网络、无引擎构建）。
pub enum ProbeState { Ok, Fail }          // 第一版只有两档；Warn 预留
pub struct ProbeReport {
    pub state: ProbeState,
    pub detail: Option<String>,           // Fail 时必填，含「怎么修」中文
}

/// 对某能力做运行期探测；None = 该能力无需探测（或未启用，探测只在 enabled 时跑）。
/// ⚠️ 纪律：只允许本地 fs 检查（metadata / 临时文件创建后立即删除），
///    禁止网络请求、禁止引擎构建——这是 PRD 验收 6（幂等无副作用）的硬约束。
pub fn probe(cfg: &Config, cap: Capability) -> Option<ProbeReport>
```

各 capability 的探测项（match 全量穷尽，无探测项的返回 `None`）：

| Capability | 探测内容 | 失败文案示例（含怎么修） |
|---|---|---|
| Asr | `cfg.asr.model` 文件存在 | `[asr].model 指向的文件不存在：…。请检查挂载卷；容器内缺失时入口脚本会自动下载，离线环境请先预置。` |
| Vad | `cfg.vad.model` 文件存在 | 同上口径 |
| Tts | 仅 `engine=kokoro` 时：`model/voices/tokens/data_dir` 四路径存在 | 对齐 `validate_tts_kokoro`（impls.rs:169-196）的文案风格 |
| Memory | `db_path` 父目录存在且可写（创建 `.health-probe` 临时文件 → 立即删除；`:memory:` 直接 Ok） | `[memory].db_path 所在目录不可写：…。请确认挂载卷权限（docker-compose 的 ./server-data 需可写）。` |
| Llm / FullChain / Voices / Firmware / Soul / Command / Wake | 无本地探测（凭据/上游连通属配置级，网络探测被验收 6 禁止） | — |

**新文件 `server/src/app/ws/health.rs`**（HTTP handler，薄）：

- `pub(super) async fn engines_health(State(engines)) -> Response`：
  1. `engines.host.snapshot()` 取**运行期真实状态**（含 boot 失败标记、热切换失败标记）——与 `/api/plugins` 完全同源；
  2. 逐 capability 调 `plugins::health::probe(&engines.host.config(), cap)`（仅 enabled 项）；
  3. phase 合并规则：`probe == Fail` 时，必需能力 → `Failed`、可选能力 → `Degraded`（取与 host 状态更坏者）；`last_error` 取 probe 文案（host 已有更坏状态时保留 host 的）；
  4. 附 `config_section` 字段（capability → 配置页 section id 映射，前端跳转锚点）：`fullchain→"aiui"`、`firmware→"ota"`、`voices→null`（无对应 section），其余同 id；
  5. 探测为纯本地 fs 快速操作（<1ms 量级），handler 内联执行即可，无需 spawn_blocking。

**改动 `server/src/plugins/host.rs`**（同源保证的关键，非可选）：

从 `capabilities_json`（host.rs:346-403）中把"单能力聚合"逻辑提取为公开 helper，两处共用：

```rust
/// 某能力的聚合视图（phase 取最坏 / active_impl / enabled / last_error / 实现明细）。
/// `GET /api/plugins` 与 `GET /api/health/engines` 共用本函数——同源不分叉是 P1-6 的硬要求。
pub struct CapabilitySummary {
    pub capability: Capability,
    pub phase: Phase,
    pub enabled: bool,
    pub active_impl: Option<&'static str>,
    pub last_error: Option<String>,
    pub impls: Vec<PluginStatus>,
}
impl PluginHost {
    pub fn capability_summary(&self, cap: Capability) -> CapabilitySummary { /* 现有 fold 逻辑原样搬入 */ }
}
```

`capabilities_json` 改为遍历调用该 helper（**行为逐字节不变**，既有测试 `capabilities_json_carries_every_key_the_overview_page_reads` 守护）。

**改动 `server/src/app/ws/mod.rs`**：`mod health;` + 路由 `.route("/api/health/engines", get(health::engines_health))` + 模块注释路由表补一行。（ws/mod.rs 里已有内联 `async fn health()`（:176）——Rust 中函数与模块分属不同命名空间，共存无冲突；为可读性 handler 命名为 `engines_health`。）

#### 1.1.4 前端设计

**新文件 `EngineHealthState.kt`**：继承 `LoadableState`（OpsState.kt:81），`endpoint = "/api/health/engines"`，`apply()` 解析 `capabilities[]` → `ObservableList<HealthRow>`（capability/display/phase/enabled/lastError/probeState/probeDetail/configSection/deviation）+ 摘要计数。完全比照 `McpToolsState` / `PluginMetaState` 范式。

**新文件 `EngineHealthCard.kt`**：两个渲染件（全部复用 AdminTheme 组件层，禁止自造样式）：

- `healthBanner(health, onJump)`：**就绪度横幅**（PRD P0「概览页横幅扩展」）。仅当存在 deviation 行时渲染——每行「{display} · {降级|异常}」可点击 → `onJump(config_section)`；正常时不占任何视觉（延续"只在偏离时打标"铁律）。
- `engineHealthCard(health, onJump)`：**能力健康卡**（PRD P1「配置页总览 section 能力健康卡」）。11 行 × phase 一览：deviation 行打 `statusBadge` + 原因展开 + 可点击；正常行弱化（textTertiary 一行文字带过，避免一片绿噪音）；行尾显示 active impl 展示名（呼应 PRD P1「capability 行展开可见其实现明细，数据同源」——数据即响应里的 `implementations[]`）。

**接线（三壳 + 两渲染点）**：

| 文件 | 改动 |
|---|---|
| `AdminPage.kt` | `val engineHealth = EngineHealthState(this)` + `pageDidAppear` 加 `engineHealth.load(this)`；`renderOpsOverview` 调用点增参 + 跳转 lambda `{ section -> selectedSection = "config"; form.configSection = section }` |
| `OpsPages.kt` | `renderOpsOverview(stats, meta, health, ctx, onJumpSection)` 增参；页面最顶部（stats 卡之上）插 `healthBanner`；能力总览卡之下插 `engineHealthCard` |
| `ConfigCards.kt` | `renderConfigLayout` 增参 `health: EngineHealthState`；`renderConfigSection` 的 `overview` 分支渲染 `engineHealthCard(health) { form.configSection = it }` |
| `AdminShell.kt`（macOS） | `renderConfigLayout` 调用点同步增参（数据源加载跟随该壳既有 baseUrl 模式） |
| `ConfigPage.kt`（独立配置页） | 同上增参 + 加载 |

`form.configSection` setter 已存在（P0 导航重组产物，同步 `configSectionList: ObservableList`），跳转链路零新机制。
