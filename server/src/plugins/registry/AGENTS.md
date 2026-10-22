# 注册表与宿主维护

[类型与查找](<mod.rs>)定义能力、描述符和 hot 档位；[描述符](<impls.rs>)负责选择/签名/校验/构建；[宿主](<../host.rs>)校验与上报；字段表负责 schema 投影。新增实现先看同能力现有描述符，不按历史伪代码照抄。

- id 使用 `<capability>.<impl>` 命名空间，唯一且可枚举；配置 engine 值对应去掉能力前缀的实现 id，别名统一规范化。
- 新增/改名实现同步能力模块的 `<CAP>_ENGINES` 表及描述符；[配置测试](<../../config/tests.rs>)守一致。
- 私有字段进实现子段、跨实现通用项留能力本体；单实现的既存 LLM/ASR/VAD 不强制搬家。字段路径必须真实存在于 Config。
- 三档 hot：live / next_session / restart_only。字段级优先，应用级 server/audio 段在 APP_SECTION_HOT 登记；遗漏不能靠文档补救。
- schema 的 common_fields 是 `[{fields_path, fields}]` 数组；改形状同步客户端，不是自动布局契约。
- enabled 与 phase 分离；校验文案含合法值和“怎么修”，多问题一次汇总。未知 engine 不静默回退，memory none 显式豁免。

回归：[注册表测试](<tests.rs>)、[保存影响测试](<../../gateway/ws/config/tests/impact.rs>)。

---

## 描述符与宿主设计（历史记录）

> **历史设计 / 实施记录**：由原《插件架构统一》长文按职责抽取，保留原章节编号及当时的结论，不代表所有提案均已落地。
> 原章节中的 `§N` 请通过[完整章节索引](<../../../../docs/designs/plugin-architecture-unification.md>)定位；当前行为先读本文件上半部分、源码与测试。

## 2. DSH 插件理念 → 我们要借的 6 条（机制层，非 Node 实现层）

> 完整 8 条与"不必照搬"清单见 `soul-and-graph-memory-plan.md` §11。这里只列**本项目能落地**的 6 条，并直接给出 Rust 形态。

| # | DSH 机制 | 证据（asar 内包/文件） | 本仓库落地形态 |
|---|---|---|---|
| 1 | **清单即能力声明**：宿主只读清单即可枚举插件，无需载入代码 | `dsh-plugin-manager` `reconcile()` 读包 `dsh.bundle` 并写入 `dsh.profile.bundles`；无 `dsh.bundle` 的包只警告"not a profile layer" | `plugins/registry/` 的 `&'static [PluginDescriptor]`：能力可枚举 → `GET /api/plugins` |
| 2 | **注册表 + 命名空间 + 重复即报错** | `cordis/src/registry.ts`、`dsh-llm` 重复路由 → `DUPLICATE_ADAPTER`；`dsh-persona` 全局挂载 *fails loud* | 注册表 `id` 唯一性单测（编译期常量表也能测）；同一能力同一 `engine` id 不可重复 |
| 3 | **声明式依赖（inject）**：全就绪才激活，任一缺失即失败（**而非静默降级**） | `graph-memory/dsh.ts` 的 `inject=[...]`；`fiber.ts resolveConfig()` 校验失败抛 `ValidationError` 拒绝加载 | `requires: &'static [Requirement]`，`PluginHost::validate()` 在构建前跑；错误信息必须可行动（指到缺失模型/环境变量） |
| 4 | **生命周期状态机 + `enabled` 与激活态分离** | `cordis/src/fiber.ts` `FiberState`：`PENDING→LOADING→ACTIVE/FAILED→UNLOADING→DISPOSED`；`dsh-settings` 分开上报两者 | `enum Phase { Disabled, Loading, Active, Degraded{reason}, Failed{error} }` + `PluginStatus.enabled`（配置意图）/`phase`（实际） |
| 5 | **资源统一登记为 effect → disposer**（逆序、幂等） | `cordis/src/events.ts`、`ctx.effect(execute) → Disposable` | `PluginHost` 持 `Vec<Box<dyn FnOnce() + Send>>`，卸载/重配/退出走同一条清理路径（`Drop` + `CancellationToken`） |
| 6 | **schema 驱动校验 + `.volatile()` 标记可热改字段** | `dsh-settings/src/schema.ts` 投影、`ValidationError`；⚠️ 但 `autoGenerate` **无随发布客户端使用**，表单是手写的 | `FieldSchema{key,label,kind,default,help,hot}` 常量表 → `GET /api/config/schema`（**用于校验/默认值/字段提示/密钥打码标记，不用于自动生成布局**，理由见 `soul-and-graph-memory-plan.md` §11.1） |

**明确不借**（Node/Cordis 特有，详见 `soul-and-graph-memory-plan.md` §11.3）：`!!js` 表达式、原型链 `ctx` 解析、`Symbol` realm、pnpm 安装器与 `dsh.bundle` 对账、typert 装饰器 RPC、客户端 HMR 版本戳。

---

## 3. 统一插件化架构设计

### 3.1 数据模型：三张表 + 一个宿主

```rust
// server/src/plugins/registry/{mod,fields,impls}.rs（新增，全部为 'static 常量，零分配、可单测）

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Capability { Asr, Vad, Tts, Llm, FullChain, Voices, Firmware, Memory, Soul }

/// 热生效语义（= DSH 的 .volatile()，但要区分三种粒度）
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum HotReload {
    Live,        // 保存即对**新请求**生效（LLM/记忆/灵魂/TTS）
    NextSession, // 保存后对**新会话**生效（VAD 阈值等；当前会话保持旧值）
    RestartOnly, // 必须重启（server.port / worker_threads / 下行音频参数）
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Requirement { SherpaFeature, ModelFiles, Credentials, LlmUpstream, EmbeddingUpstream }

#[derive(Clone, Copy)]
pub enum FieldKind { Str, Password, Int, Float, Bool, Enum(&'static [&'static str]), Path, Dir, Text }

#[derive(Clone, Copy)]
pub struct FieldSchema {
    pub key: &'static str,          // TOML 路径片段："api_key"
    pub label: &'static str,        // UI 标签（中文）
    pub kind: FieldKind,
    pub default_hint: &'static str, // 展示用默认值（真实默认值仍在 serde default 函数里）
    pub help: &'static str,         // 一句话说明 + 坑（例："留空则按 X 降级"）
    pub hot: HotReload,
    pub required: bool,
}

/// 构建结果：类型擦除但**保持各 trait 不变**
pub enum Built {
    Asr(Arc<dyn AsrEngine>),
    Vad(Box<dyn VadEngine>),
    Tts(Arc<dyn TtsEngine>),
    Llm(Llm),
    FullChain(Arc<dyn FullChainEngine>),
}

pub struct PluginDescriptor {
    pub id: &'static str,              // "tts.kokoro"（<capability>.<impl>，namespace 唯一）
    pub capability: Capability,
    pub display: &'static str,         // "Kokoro（本地离线）"
    pub local: bool,                   // 本地 / 远程（UI 两级下拉的依据）
    pub hot: HotReload,
    pub requires: &'static [Requirement],
    pub fields: &'static [FieldSchema],          // 该实现的私有字段（含 secret）
    /// 热切换判定：由**该实现自己**声明哪些字段影响成败（取代 runtime/mod.rs 手写的 tts_sig）
    pub signature: fn(&Config) -> String,
    pub build: fn(&Config) -> Result<Built>,
}

pub static REGISTRY: &[PluginDescriptor] = &[
    asr::SENSEVOICE, tts::KOKORO, tts::XFYUN, llm::OPENAI, vad::SILERO, aiui::FULL_CHAIN,
    voices::CATALOG, firmware::HOST, /* P4: memory::GRAPH, soul::PROFILE */
];
```

**关键取舍**：`Built` 是一个枚举而不是 `Box<dyn Any>`。Rust 里类型擦除后向下转型会引入运行时 panic 面；枚举 + 按能力 `match` 保持 100% 编译期检查，且**五个引擎的 trait 定义一行都不用改**（这是本设计能低风险落地的根本原因）。

### 3.2 宿主：装配、校验、隔离、热切换、上报

```rust
// server/src/plugins/host.rs（新增）
pub struct PluginHost {
    cfg: Arc<Config>,
    states: RwLock<HashMap<&'static str, PluginState>>, // id -> {phase, last_error, built_at}
    disposers: Mutex<Vec<Box<dyn FnOnce() + Send>>>,
}

impl PluginHost {
    /// 启动装配：required 能力失败 → 快速失败；optional 失败 → 记 Degraded/Failed 继续启动
    pub fn boot(cfg: &Config) -> Result<Arc<Self>>;
    /// 依赖校验（inject 语义）：构建前把"缺什么"说清楚，而不是让引擎 ctor 抛底层错误
    fn validate(d: &PluginDescriptor, cfg: &Config) -> Result<()>;
    /// 统一热切换：逐个 descriptor 比 signature，变了才重建；失败**保留旧实例**并记 last_error
    pub fn refresh_from_disk(&self) -> Vec<(&'static str, Result<(), String>)>;
    /// 状态快照（GET /api/plugins）
    pub fn snapshot(&self) -> Vec<PluginStatus>;
}
```

四条硬规则：

1. **required vs optional 分级**：`Asr/Vad/Tts/Llm` 属 required（对齐现状：`Engines::new` 失败即退出）；`Voices/Firmware/Memory/Soul` 属 optional → **一个可选插件坏掉不能拖垮语音服务**（DSH 的"插件失败不致命"）。例：记忆库文件损坏 → `Phase::Degraded{reason:"记忆库不可写，已禁用记忆"}`，语音对话照常。
2. **校验前移**：把现在散落的硬编码校验（`runtime/mod.rs:60-63` VAD 路径、`:65-71` AIUI 三要素）统一成 `required` 字段 + `Requirement::{ModelFiles, Credentials}`，错误文案统一含"怎么修"（例：`[vad].model 文件不存在：/data/models/silero_vad.onnx（容器内缺失模型时入口脚本会自动下载；离线环境请预置）`）。
3. **热切换覆盖 LLM/记忆**：`Engines.llm` 由 `pub llm: Llm` 改为 `RwLock<Llm>`（与既有 `tts: RwLock<...>` 同模式，**不引新依赖**），`refresh_from_disk` 在**会话开始 / 测试台请求前**调用（现有 TTS 的调用点），从此改 `[llm].model` 不再需要重启。
4. **清理登记**：记忆库句柄、探测任务、后台抽取任务在构建时 `register_disposer(...)`；热切换与进程退出共用同一路径（避免"换了引擎旧连接泄漏"）。

### 3.3 配置约定（统一后的唯一规范）

```toml
[tts]
enabled = true                 # 新增：所有能力段统一有（缺省 true，兼容旧配置）
engine  = "kokoro"             # 新增：实现 id；`backend` 保留为 deprecated alias（serde alias）
num_threads = 4                # 通用项（跨实现）留本体
cache_entries = 200

[tts.kokoro]                   # 实现私有段（本地实现也进子段——统一规则）
model = "/data/models/kokoro-int8-multi-lang-v1_1/model.int8.onnx"
# ...
[tts.xfyun]                    # 远程实现私有段（既有形态，不动）
app_id = "..."

[llm]
engine = "openai"
[llm.openai]
api_base = "https://.../v1/responses"
api_key_env = "XIAOZHI_LLM_API_KEY"   # 密钥引用优先；api_key 明文保留但 UI 打码
[llm.routing]                  # 用途路由（P3，见 §4.4）
chat = "openai"
extract = "openai-mini"
embedding = "bge-m3"
```

迁移策略（**这是本文件唯一"破坏性"的部分，故单独说明**）：

- **只增不删**：`engine` 新增、`backend` 保留 alias；`[tts.kokoro]` 与旧的 `[tts].model` 双读——`serde(alias)` 做不到"两种位置都认"，因此实现方式为：`TtsConfig::normalize()` 在加载后把本体里的 Kokoro 字段搬进 `kokoro` 子结构（若子结构为空），保存时只写新结构；
- 未知键继续静默忽略（`config.rs:10-11` 的既有行为，不改）；
- 保存后 `config.toml` 重排为规范形态（管理页保存本来就丢注释，见 `ws/config.rs:2`，行为无变化）；
- **回滚**：`engine` 缺省时行为与今天逐字节一致（`engine_signature` 也保持旧公式），因此 P0（只加表与端点）**零行为变化**。

### 3.4 端点：能力可枚举、可诊断、可操作

| 端点 | 语义 | 借用点 |
|---|---|---|
| `GET /api/plugins` | 能力清单 + `enabled`/`phase`/`active_impl`/`hot`/`last_error` | DSH `pluginInventory.list()`（inflight 只读清单） |
| `GET /api/config/schema` | 按能力+实现返回 `FieldSchema[]`（含 `secret:true` 标记、`hot` 标记、`required`） | DSH `dsh-settings` 的 Config 投影（**但只做元数据，不做自动布局**） |
| `GET /api/config` | **打码** secret（`has_api_key: true`）+ 返回 `revision` | DSH secrets → presence marker |
| `POST /api/config` | 带 `expected_revision`；不一致 409；省略的 secret 字段**保留原值** | DSH `form.mutate(ops, expectedRevision)` 拒绝过期写入 |
| `POST /api/plugins/{id}/actions/{name}` | 统一动作：`voices.probe` / `tts.test` / `llm.test` / `memory.clear` / `memory.recall` / `plugin.reload` | DSH `ctx.commands.register` → `{kind:'success'\|'error', text}`；本项目已有的 `voices.probing` 轮询契约 |
| `GET /api/plugins/{id}/actions/{job}` | 长任务状态轮询（`running/done/failed` + 结构化结果） | 同上（probe 全量约 1 分钟，必须异步） |

**为什么动作要统一**：现在"探测音色目录"是 `voices.rs` 私有实现（`probing` 字段 + 自定义轮询），"测试合成"是 bench 的 `tts_test` 协议消息，两者语义相同（长任务 + 状态 + 结果）却两套契约。统一后 UI 只需要一套"动作按钮 + 状态徽标 + 结果区"组件。

### 3.5 迁移路径（每步可独立上线、可回滚）

| 步 | 内容 | 行为变化 | 回滚 |
|---|---|---|---|
| M0 | 加 `registry/`（mod/fields/impls）/`host.rs`/`PluginStatus` + `GET /api/plugins`（**只读、只报告，不接管装配**） | 无（纯新增端点） | 删端点 |
| M1 | `Engines::new` 改为经 `PluginHost` 装配（required 校验前移，错误文案升级） | 错误信息更清楚；其余不变 | 保留旧 `Engines::new` 分支 |
| M2 | LLM/记忆纳入热切换（`RwLock<Llm>` + 统一 `refresh_from_disk`） | `[llm]` 改动免重启 | 关闭开关即回旧行为 |
| M3 | 配置统一（`engine` + `[<cap>.<id>]` 私有段；TTS Kokoro 参数搬家） | 保存后的 TOML 结构变化（旧文件仍可读） | alias + `normalize()` 双读 |
| M4 | `voices`/`firmware` 纳入注册表与统一动作模型 | 端点兼容保留旧路径 | 旧路由不动 |
| M5 | 客户端「能力总览」页 + 每能力卡片改造（§5） | UI 变化 | 旧卡片保留 |

### 3.6 收益量化：扩展成本对比（本设计的 ROI 证明）

| 场景 | 现状（实测步骤数） | 统一后（目标值） | 证据 |
|---|---|---|---|
| 新增一个 **TTS 供应商** | **19 处**：`plugins/tts/mod.rs` 9 处（`TtsBackendKind` 变体/`backend_kind`/`engine_signature`/`TtsConfig` 字段/`mod`+`pub use`/`Default`/`build_tts`/`build_tts_with_lang`/文档）+ `config.rs` re-export + `config.example.toml` + `voices.rs` 与凭据测试需**新写一套** + 客户端 4 处（observable/注册表/`save` put/`fill` opt）+ 卡片 `vif` 区整段抄 | **≤5 处**：① 新文件实现 `TtsEngine`；② 描述符表加 1 条（含 `fields`/`signature`/`build`）；③ 凭据结构体；④（可选）专用交互卡片 | §1.3、盘点 D.2 |
| 新增一个 **ASR 实现** | **15 处**，且**必须先自己补** `backend` 字段与热切换骨架（ASR 既无 `backend` 也无 `RwLock`） | **≤5 处**（同上；热切换由 `PluginHost` 统一提供，不再每个能力各写一遍） | 盘点 D.1 |
| 新增一张**能力卡**（例 AIUI） | **5~8 处**：tab 列表 + 卡片函数 + observable + `save` put + `fill` opt +（可选）注册表/按钮/尺寸常量 | **1~2 处**：`SectionRegistry` 加 1 行 + 卡片函数（纯字段驱动时 1 行） | 盘点 E.2 |
| **改一个引擎并生效** | 必须**重启**（除 TTS 经测试台间接生效，见 §1.7 bug-2） | 保存即对**新会话**生效，`/api/plugins` 显示结果与失败原因 | §1.7 |

> ⚠️ 「统一后」是**目标值**，需要 P1~P4 全部落地；P1 单独落地只带来「能力可枚举 + 可诊断」这一半收益。别把目标值当成一步到位。

---

## 配置约定落地记录

> **历史设计 / 实施记录**：由原长文按职责抽取，保留原章节编号及当时的结论，不代表所有提案均已落地。
> 原章节中的 `§N` 请通过[完整章节索引](<../../../../docs/designs/plugin-architecture-unification.md>)定位；当前行为先读本目录维护文档与源码。

### 0.10 第六批：P4 配置约定统一（本文件 §3.3 的落地）

> 这是本设计**唯一破坏性**的部分（§7 风险 5），因此实现上贯彻"**只增不删 + 双读 + 补丁重写**"。

**三条约定**（`server/src/config.rs` 模块注释为权威）：

| 约定 | 形式 | 为什么 |
|---|---|---|
| ① `engine` 键选实现 | `[tts] engine = "kokoro"` ↔ 描述符 `tts.kokoro`（id 前缀即能力） | 让"哪个实现被选中"成为**可枚举/可校验**的配置事实，而不是散落在代码分支里 |
| ② 私有字段进子段 | `[tts.kokoro]` / `[tts.xfyun]` / `[memory.extractor]` | 新增供应商不再需要往能力本体加字段（本体只留跨实现通用项） |
| ③ 跨实现参数留本体 | `[tts] speed/speaker/cache_entries` | 判定规则**可机械执行**：被 `Trait::method` 当入参用的留在本体，其余进私有段 |

**落地清单**：

| 关注点 | 落地形态 |
|---|---|
| 规范化 | `Config::normalize()`（幂等）在**三个入口**调用：`load()` / `Default` / `merge_client_patch()`；`persist_config()` 再兜一次（防手搓 `Config` 直接落盘写成空 `engine`） |
| 旧写法双读 | `[tts] backend="sherpa"` → `engine="kokoro"`（`EngineSpec.aliases`）；`[tts].model/voices/tokens/data_dir/dict_dir/lexicon/lang/num_threads` → `[tts.kokoro].*`（旧字段 `#[serde(default, skip_serializing)]`，**保存只写新结构**）；`[memory].backend` → `[memory].engine`（serde `alias`） |
| 新位置优先 | 同一字段两处并存时新位置胜出（避免旧键覆盖新键）；与默认值等价的旧值不产生迁移噪音 |
| 旧客户端补丁 | `gateway/ws/config.rs::rewrite_legacy_paths`（在 `sanitize_patch` 内、merge 之前）：`tts.backend→engine`、`tts.<旧扁平字段>→tts.kokoro.<字段>`、`memory.backend→engine`；**同一补丁里显式的新位置优先**。没有这一步，旧 macApp 改 `model` 会被"新位置优先"静默丢弃 |
| 未知值不静默回退 | `registry::{unknown_engine_of, unknown_engine_hint}`：`PluginHost::plan` 把该能力全部实现标 `Failed` + 可选值原因；`boot` 汇总报错；`decide_refresh` 返回 `Err`（否则保存一个新 engine 值后热刷新会静默保留旧引擎） |
| 去中心化 | **删除** `TtsBackendKind` 枚举、`TtsConfig::engine_signature()`、`build_tts()` 中心分发；选择走描述符 `selected`、构建走描述符 `build`、签名走描述符 `signature`（`runtime/mod.rs::refresh_tts` 改为按描述符构建） |
| `asr`/`vad`/`llm`/`memory` | 也补上 `engine` 键（`sensevoice`/`silero`/`openai`/`graph|none`）；`<CAP>_ENGINES` 表是配置层权威，`config/tests.rs` 守"声明 id ↔ 注册表实现"一致 |
| 字段元数据 | `registry/fields.rs`：`engine` 进入各能力**公共字段**（含 enum 可选值），TTS 的 `speed`/`speaker` 从 Kokoro 私有表移入公共表（与约定③一致）；漂移护栏测试当场抓到过这次不一致（`tts.kokoro 的字段 speaker 在 Config 路径中不存在`） |
| 客户端 | `[tts]` 写 `engine` + `kokoro` 子段；`fill()` 读 `engine`（回落 `backend`）与 `kokoro`（回落旧扁平位置）；本地引擎 id 统一为 `kokoro`（`canonicalTtsEngine()` 兼容旧值 `sherpa`）；记忆卡读 `engine`、状态卡显示"引擎" |
| 样例 | `config.example.toml` 改为新约定（`engine` 键 + `[tts.kokoro]`），并加"旧写法仍可读"的注释；新增测试 `example_config_parses_and_normalizes` 守住"样例必须能解析" |

**偏差（逐条）**：

1. **`llm.openai` / `asr.sensevoice` / `vad.silero` 的字段仍留在本体**（设计 §3.3 的示例把 LLM 字段画进了 `[llm.openai]`）：这三个能力目前**只有一个实现且无第二实现计划**，搬家的收益为零，却要动 `SECRET_PATHS` 路径、`[memory.extractor]` 的逐字段覆盖以及约 30 处测试。按 §7 风险 7「不要为了统一而统一」，本次只把**机制**（子结构 + 旧字段只读 + `normalize` 双读 + 补丁重写）在 TTS 上完整落地并验证——它正是验收指标（新增 TTS 供应商成本）所指的能力。出现第二个 LLM/ASR 实现时按 TTS 同一套搬即可（机制与先例已在）。
2. **`[memory].engine = "none"` 不是实现 id**：它是"显式停用"的哨兵值（保留配置但不生效），与 `enabled=false` 等价。`unknown_engine_of` 对它显式放行，否则会把合法配置报成"未知实现"。
3. **`speed`/`speaker` 留在 `[tts]` 本体而非 `[tts.kokoro]`**：它们是 `TtsEngine::synthesize_stream` 的入参（讯飞也吃 `speed`），属约定③。原先打算放进 Kokoro 子段，实施中按"可机械执行的判定规则"纠正，并同步了字段元数据表。
4. **AIUI 用 `enabled` 而非 `engine`**：`[aiui]` 是"整条流水线的模式开关"（能力 `fullchain`），不是多实现互斥选择，保留布尔开关更直白；描述符 id 仍是 `fullchain.aiui`（配置段名为历史的 `[aiui]`，`config/tests.rs` 里作为**唯一例外**显式列出）。
5. **`sherpa` 成为 `kokoro` 的别名而非新 id**：`engine="sherpa"`（旧 `backend` 取值）与 `engine="kokoro"` 等价；好处是 `backend = "sherpa"` 的老配置零改动，且 `engine` 的值空间与描述符 id 严格对应。
6. **`TtsConfig.engine` 默认为空字符串**（不是 `"kokoro"`）：空 = 未指定，才能让"文件里只有旧 `backend`"这条路径走别名解析；`normalize()` 负责补全，因此 GET `/api/config` 与落盘文件永远是具体 id。
7. **`GET /api/memory/status` 的 `backend` 键改名为 `engine`**（客户端同步）：一个概念不留两个名字；旧字段值本来就是 `graph`/`none`，语义没变。
8. **未做统一动作模型**（设计 §3.4 的 `POST /api/plugins/{id}/actions/{name}` + job 轮询）：P4 的另一半（`voices.probe` / `tts.test` / `llm.test` / `memory.*` 归一到一套"动作 + 状态 + 结果"契约）本质是**新增端点与契约**，与配置约定无耦合，且需要真机验证探测/合成的长任务语义。本次先把配置约定（唯一破坏性部分）做完并单独验证。

**验收证据**（P4 的两条验收指标）：

| 验收项 | 证据 |
|---|---|
| 旧 `config.toml` 原样可读 | `full_test_config()` 的 fixture **刻意写成旧位置形态**（`[tts] backend/model/…`、`[memory] backend`），`legacy_toml_fixture_migrates_to_convention` 断言迁移到规范位置且**序列化后旧键消失**；`legacy_config_with_listen_key_parses` 覆盖"未知键静默忽略 + 旧 `[tts]` 扁平写法"；`raw_legacy_toml` fixture 同时驱动**全部既有往返/部分更新测试**（说明旧写法不只是"能解析"，而是全链路可用） |
| 旧客户端补丁不丢改动 | `legacy_client_form_shape_no_longer_resets_omitted_sections`：补丁写 `tts.backend` + `tts.model=/m/k2.onnx`，断言 `engine=xfyun` **且** `kokoro.model` 真的变成 k2（不是被"新位置优先"丢弃） |
| 新增 TTS 供应商成本 | 服务端需要改的只剩：① 新实现文件；② `plugins/tts/mod.rs` 的凭据/参数结构 + `build_<id>()`；③ 注册表描述符 1 条（`fields` 进 `registry/fields.rs`）；④ `<CAP>_ENGINES` 表 1 行。**枚举/工厂分发/签名函数/`match` 分支已全部消失**（原 19 处 → 服务端 ≤5 处；客户端的 `ttsRemoteEngines` + 字段区待 P5 由 schema 驱动收口） |
| engine 写错的可见性 | `unknown_engine_marks_capability_failed_with_hint`（`/api/plugins` 标 `Failed` + 列出可选值）、`boot_rejects_unknown_engine_with_actionable_message`、`memory_engine_none_is_not_an_unknown_engine` |

**验证命令与实际结果**：`cargo test` 默认 **195 passed / 0 failed / 3 ignored**；`cargo test --features sherpa` **195 passed / 0 failed / 3 ignored**；`./gradlew :apps:h5App:publishWeb` **BUILD SUCCESSFUL**（产物含 `kokoro`/`"engine"`/「engine（引擎）」）；文件规模 ≤600 行与"0 个内联测试块"校验均无输出；被改的 21 个 Rust 文件经 `rustfmt --check --skip-children` **0 diff**（`--skip-children` 是必需的：不加会把子模块既有漂移算到本文件头上，我第一轮统计就被它误导过一次）。

⚠️ **未做**：真实模型下的端到端联调（本机无 `/data/models`，跑不了 `mock_client.py`）；`/api/config` 的运行时形状冒烟（无模型无法启动进程，形状由单测覆盖）；统一动作模型（见偏差 8）。
