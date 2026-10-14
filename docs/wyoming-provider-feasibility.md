# Wyoming Provider 可行性评估 + 接入点设计

> 评估对象：为 `xiaozhi-server-rust` 增加 Wyoming Protocol 化的 ASR / TTS / WakeWord Provider。
> 定位：**可行性评估 + 接入点设计**，不是 PRD，也不是最终实现方案。
> 所有结论均带 `文件:行号` 证据；读不到的地方明确写「未验证」。
> 评估日期：2026-10-14；代码基线：`agents.md` 2026-10-13 版 / `docs/gap-implementation-design.md`

---

## ① 结论（8 行内）

**部分可行，且 TTS 是收益/成本比最高的一档；ASR 可行但缺热切换；WakeWord 在本仓库无处可插。**

1. TTS 侧**已有远程网络 provider 的成熟先例**（`plugins/tts/aiui.rs`，同步阻塞 WebSocket + PCM 分片回调），Wyoming TTS 是它的近亲，改造成本可控。
2. ASR 侧 trait 极简（`asr/mod.rs:29-31` 单方法），但 **`Engines.asr` 是不可变字段（`engine.rs:47`），没有热切换路径**——Wyoming 换实例只能重启（与 SenseVoice 同档，可接受）。
3. **PCM↔Opus 转码成本几乎为零**：引擎边界两侧本来就是 PCM f32，Opus 只存在于 ESP 链路两端的 `app/audio/opus.rs`，且**已有 `StreamingResampler` 与 `f32_to_i16_16k`**（`app/audio/pcm.rs:8`）——这是本次评估最反直觉的结论。
4. 最大阻碍 #1：**同步 trait × 长连接**。现有 ASR/TTS trait 是同步方法（靠 `spawn_blocking` 隔离），而 Wyoming 是长连接 TCP；照抄 `aiui.rs` 的「每次调用一条短连接」会丢掉进程级连接池的全部收益。
5. 最大阻碍 #2：**WakeWord 无接入点**。本仓库的 `[wakeup]` 是「唤醒后说『我在』」的话术插件（`plugins/wakeup/mod.rs:1-37`），**不是唤醒词识别引擎**；唤醒词识别在 ESP32 固件里，服务端根本看不到唤醒前的音频。
6. 最大阻碍 #3：agents.md §5.2b 的「新增供应商只需改两处」论断**在网络型 provider 上不成立**——`plugin-architecture-unification.md` 的设计目标是「无网络状态」的热切换模型（`signature` 字符串比对 + 会话开始重建），Wyoming 的进程级连接生命周期与之不同源。
7. 推荐路线：**(a) 同步 trait + 每次调用短连接**起步（与 `aiui.rs` 完全同构，零架构改动），把「连接池」列为第二期；**不要**在本期把 trait 改成 async。
8. `LlmProvider` 本期**不动**（详见 §7.4）。

---

## ② 现有能力抽象形态速查表

### 2.1 五大引擎 trait 的真实形态

| 能力 | trait 定义位置 | 方法签名 | async? | Send/Sync | 返回形态 |
|---|---|---|---|---|---|
| ASR | `plugins/asr/mod.rs:29-31` | `fn recognize(&self, samples: &[f32], sample_rate: u32) -> Result<String>` | ❌ 同步 | `Send + Sync` | **一次性 `String`**（整段语音 → 整句文本） |
| TTS | `plugins/tts/mod.rs:315-358` | `fn synthesize_stream(&self, text: &str, speed: f32, speaker: i32, chunk_cb: TtsChunkCallback) -> Result<()>`<br>`fn synthesize(...) -> Result<(Vec<f32>, u32)>`（默认实现，`mod.rs:329-347`）<br>`fn output_sample_rate(&self) -> u32`（默认 24_000，`mod.rs:350-352`）<br>`fn name(&self) -> &'static str`（默认 `"unknown"`） | ❌ 同步 | `Send + Sync` | **流式回调**：`TtsChunkCallback = Box<dyn FnMut(u32, &[f32]) -> bool + Send>`（`mod.rs:311`），回调返回 `false` = 取消 |
| LLM | `plugins/llm/mod.rs:163-174` | `fn chat_stream<'a>(...) -> Pin<Box<dyn Future<Output = Result<LlmTurnResult>> + Send + 'a>>` | ✅ **手动装箱 future** | `Send + Sync` | **流式事件**：`LlmEvent::{Text, ToolCall, Done}`（`mod.rs:136-143`） |
| VAD | `plugins/vad/mod.rs:26-31` | `fn accept(&mut self, samples: &[f32], cb: &mut dyn FnMut(Vec<f32>))`<br>`fn flush(&mut self, cb: &mut dyn FnMut(Vec<f32>))` | ❌ 同步 | **仅 `Send`**（`&mut self`，每会话有状态） | **流式回调**：切出的完整语音段 |
| Memory | `plugins/memory/mod.rs:69-102` | `fn recall<'a>(...) -> BoxFut<'a, RecallResult>`<br>`fn record / extract_pending / maintain / clear` | ✅ **async（`BoxFut` 别名）** | `Send + Sync` | 混合：`recall`/`stats` 返回数据，`record`/`maintain` 返回 `Result` |
| AIUI 全链路 | `plugins/aiui/mod.rs:136` 附近（`FullChainEngine`） | 有状态长连接，`Mutex` 供 `spawn_blocking` 互斥（`app/session.rs:131-132`） | ❌ 同步 | `Send` | — |
| Wake | **无 trait** | `plugins/wakeup/mod.rs:134-164` 是 `struct WakeGreeting`（`new/active/text/persist`），**不是引擎** | — | — | — |

> **关键观察 1**：`BoxFut<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>`（`plugins/memory/mod.rs:61`）——**本仓库已经有 async trait 的既定范式**，不引 `async-trait`。若将来要 async 化，MemoryProvider 是现成的抄写模板。
>
> **关键观察 2**：`AsrEngine` 是**五个 trait 里最薄的**（单方法、无状态、无 session 概念），也是唯一**返回一次性值**的引擎 trait。它的接口形状天然与「 Wyoming ASR 一次 `transcribe` 请求 → 一个 `transcript` 响应」同构。
>
> **关键观察 3**：TTS trait **已经是流式的**（`synthesize_stream` 是必实现方法，`synthesize` 只是默认收集器）。**这一点决定了 §5 的判断：Wyoming TTS 整段返回不构成问题。**

### 2.2 `plugins/wakeup/mod.rs` 的真实性质（用户方案的一处误判）

| 事实 | 证据 |
|---|---|
| `[wakeup]` 是**唤醒问候话术**插件，不是唤醒词识别引擎 | `plugins/wakeup/mod.rs:1`（"设备唤醒 → LLM **之前**的一道拦截"）；`DEFAULT_GREETING = "我在"`（`mod.rs:43`） |
| 注册表里它是 `Capability::Wake`，`build: None`、无共享实例 | `registry/impls.rs:556-570`（`WAKE_GREETING`） |
| 描述符注释明确写「**不是引擎**，只按 `[wakeup].enabled` 开关」 | `registry/mod.rs:92-95` |
| 唤醒信号来自 **ESP32 固件**，不是服务端音频 | `app/session/wakeup.rs:9-11`：固件发 `{"type":"listen","state":"detect","text":"<唤醒词>"}`（`Protocol::SendWakeWordDetected`，由固件 `CONFIG_SEND_WAKE_WORD_DATA=y` 启用） |
| 服务端**看不到唤醒前的音频** | `app/session.rs:491-511`（`handle_binary`）：上行二进制帧 → Opus 解码 → VAD → ASR。设备只在唤醒后才上行音频 |
| 没有 `WakeWordProvider` trait，也没有任何唤醒词模型/推理 | 全仓 grep 无命中；`ASR_ENGINES`/`TTS_ENGINES`/`LLM_ENGINES`（`asr/mod.rs:22`、`tts/mod.rs:49`、`llm/mod.rs:23`）均无 wake 相关项 |

**结论**：`WyomingWakeWordProvider` 在本仓库**没有接入点**。要做服务端唤醒词识别，前置条件是 ESP32 固件改成「常开上行音频流」（当前固件是唤醒后才上送），这是**固件级改造**，不在本仓库范围内。详见 §7.3。

### 2.3 注册表机制（`plugins/registry/`）

`PluginDescriptor` 字段（`registry/mod.rs:396-422`）：

| 字段 | 类型 | 说明 | 行号 |
|---|---|---|---|
| `id` | `&'static str` | 命名空间唯一，格式 **`<capability>.<impl>`**（如 `tts.kokoro`） | `mod.rs:397-398` |
| `capability` | `Capability` | 11 个变体的闭枚举 | `mod.rs:399` |
| `display` | `&'static str` | UI 展示名 | `mod.rs:400` |
| `local` | `bool` | 本地（离线）vs 远程（在线 API）——**UI 两级下拉的依据** | `mod.rs:402` |
| `hot` | `HotReload` | 该实现整体热生效语义，字段级可覆盖 | `mod.rs:404` |
| `requires` | `&'static [Requirement]` | 前置条件（`SherpaFeature`/`ModelFiles`/`Credentials`/`LlmUpstream`/`EmbeddingUpstream`） | `mod.rs:405`，枚举定义 `mod.rs:298-310` |
| `fields_path` | `&'static [&'static str]` | 字段所在配置路径，如 `["tts","xfyun"]`；`[]` = 无独立段 | `mod.rs:407` |
| `fields` | `&'static [FieldSchema]` | 字段元数据表 | `mod.rs:408` |
| `signature` | `fn(&Config) -> String` | **热切换判定：由实现自己声明哪些字段影响成败** | `mod.rs:409-410` |
| `validate` | `fn(&Config) -> Result<()>` | 构建前硬校验，错误文案**必须含「怎么修」** | `mod.rs:411-412` |
| `build` | `Option<fn(&Config) -> Result<Built>>` | 构建入口；`None` = 无共享实例（每会话构造 / 纯服务型能力） | `mod.rs:413-414` |
| `warn_if` | `Option<fn(&Config) -> Option<String>>` | **软性问题** → `Phase::Degraded` + 原因（不是错误） | `mod.rs:415-418` |
| `selected` | `fn(&Config) -> bool` | 当前是否被配置选中；同能力下多实现互斥 | `mod.rs:419-421` |

**三个闭包的职责**（`selected` / `build` / `signature`）：

- **`selected`**：只回答「用户选没选它」。约定是「统一按 `engine` 键 = 描述符 id 去掉 `<cap>.` 前缀」（`registry/impls.rs:18`），典型实现 `sel_tts_kokoro`（`impls.rs:29-31`）。
- **`build`**：构建共享实例，返回闭枚举 `Built`（`registry/mod.rs:383-393`，变体 `Asr`/`Tts`/`Llm`/`Memory`/`FullChain`）。`Built` 是**闭枚举而非 `Box<dyn Any>`**，理由是保持编译期检查（`mod.rs:22-23`）。
- **`signature`**：返回内存比对用的签名串，**只用于热切换判定，绝不打日志（含密钥）**（`mod.rs:23`、`impls.rs:87`）。典型实现 `sig_tts_xfyun`（`impls.rs:88-94`）。

**`hot` 三档**（`registry/mod.rs:182-189`）：

| 档 | 语义 | 取值/文案 | 现有用例 |
|---|---|---|---|
| `Live` | 保存即对**下一次请求**生效 | `"live"` / "保存即生效" | `VOICES_CATALOG`(`impls.rs:418`)、`FIRMWARE_HOST`(`impls.rs:518`) |
| `NextSession` | 保存后对**新会话**生效 | `"next_session"` / "保存后新会话生效" | `TTS_KOKORO`(`impls.rs:348`)、`TTS_XFYUN`(`impls.rs:365`)、`LLM_OPENAI`(`impls.rs:382`)、`MEMORY_*` |
| `RestartOnly` | 必须重启进程 | `"restart_only"` / "需重启 server 生效" | `ASR_SENSEVOICE`(`impls.rs:314`)、`AIUI_FULLCHAIN`(`impls.rs:400`) |

严格程度序 `Live < NextSession < RestartOnly`，多次保存取**最严格**档（`mod.rs:209-223`）。未登记字段保守返回 `RestartOnly`（`mod.rs:265-267`）。

### 2.4 `PluginHost` 流程（`plugins/host.rs`）

| 阶段 | 函数 | 行号 | 做什么 |
|---|---|---|---|
| 1. 规划 | `PluginHost::plan(cfg) -> Arc<Self>` | `host.rs:117-173` | **只规划不构建**：遍历 `REGISTRY`，算每个实现的 `enabled`/`phase`/`last_error`。依据只有 `(d.validate)(cfg)`（无 I/O ⇒ 可在无 sherpa feature 的 `cargo test` 里全量断言）。未知 `engine` 值额外标 `Failed`（`host.rs:159-168`） |
| 2. 装配 | `PluginHost::boot(cfg) -> Result<(Arc<Self>, Booted)>` | `host.rs:182-269` | ① 先 `plan`；② **汇总全部**必需能力校验失败一次性报出（`host.rs:186-208`）；③ 遍历 `REGISTRY` 构建 `required()` 能力，同能力多实现被选中即报错（`host.rs:221-223`）；④ 可选能力走 `build_optional`（`host.rs:250-260`），失败只降级 |
| 3. 热刷新决策 | `decide_refresh(cap, old_sig, fresh) -> Result<Option<String>, String>` | **`engine.rs:535-560`** | **纯函数**（可在无引擎/无 feature 的 `cargo test` 直接断言）：<br>· 未知 engine → `Err`（**不** `Ok(None)`，否则静默保留旧引擎）<br>· `validate` 失败 → `Err`<br>· `warn_if` 有值 → `Err`（能力受限，**不重建**）<br>· 签名相同 → `Ok(None)`<br>· 签名不同 → `Ok(Some(sig))` |
| 4. 重建 | `refresh_tts` / `refresh_llm` / `refresh_memory` | `engine.rs:375-434` / `439-458` / `328-372` | 签名变 → **经 `descriptor.build` 重建**（`engine.rs:390-404`，注释明确「新增供应商只要加 1 条描述符，本函数不再分发」）→ 成功后热替换 + 清缓存 + 闸门对账；失败**保留旧实例**并写 `Phase::Degraded` |
| 5. 触发点 | `Engines::refresh_from_disk()` | `engine.rs:293-321`，调用点 `app/session/start.rs:46` | **会话开始**时读盘 → `host.set_config` → 逐能力 `refresh_*`。⚠️ **只覆盖 Tts/Llm/Memory 三个能力**（`engine.rs:311-319`） |

**⚠️ ASR 没有热切换路径**：`Engines.asr` 是**不可变 pub 字段**（`engine.rs:47`，对比 `tts: RwLock<Arc<dyn TtsEngine>>` 在 `engine.rs:49`），`refresh_from_disk` 的三个 `refresh_*` 里没有 ASR。这与 `ASR_SENSEVOICE.hot = RestartOnly`（`impls.rs:314`）一致。**所以 `[asr].engine` 切换本来就要重启，Wyoming ASR 不会比 SenseVoice 更差。**

---

## ③ 改动面清单

> 图例：**新增** = 新文件/新条目；**修改** = 改现有文件；**无需改** = 明确列出以证明边界。

### 3.1 ASR：`[asr].engine = "wyoming"`

| 文件 | 性质 | 具体改动 | 证据/约束 |
|---|---|---|---|
| `plugins/asr/wyoming.rs` | **新增** | `WyomingAsr` 实现 `AsrEngine`；`new(cfg)` 建连策略（首期：懒建连/每次调用短连接） | 对标 `plugins/tts/aiui.rs`（93-125）的实现形态 |
| `plugins/asr/mod.rs` | **修改** | ① `ASR_ENGINES` 加 `EngineSpec { id: "wyoming", aliases: &[] }`（现 22-25 行只有 sensevoice）<br>② 新增 `pub struct WyomingAsrConfig { host, port, ... }` + `#[serde(default)] pub wyoming: WyomingAsrConfig` 挂到 `AsrConfig`（现 34-58 行无私有段）<br>③ 新增 `pub fn build_wyoming(cfg) -> Result<Arc<dyn AsrEngine>>`<br>④ `impl Default for AsrConfig` 补 `wyoming` 字段（118-131）<br>⑤ `AsrConfig::normalize`（65-69）如需 clamp 新字段 | 私有段约定见 `config.rs:24-25`；`AsrConfig` 现在**没有**私有段，是本文件的结构性变化 |
| `plugins/registry/impls.rs` | **修改** | ① `sel_asr_wyoming(cfg)`（仿 `impls.rs:23-25`）<br>② `sig_asr_wyoming(cfg)`（host/port/language…）<br>③ `validate_asr_wyoming(cfg)`（host 非空 + **TCP 可达性**？）<br>④ `build_asr_wyoming_d(cfg)`<br>⑤ `pub const ASR_WYOMING: PluginDescriptor`（仿 309-323，`local: false`、`hot: RestartOnly`、`requires: &[]`）<br>⑥ 顶部 `use crate::plugins::asr::build_wyoming` | ⚠️ `validate_*` 的「错误文案必须含怎么修」约定（`impls.rs:136`） |
| `plugins/registry/mod.rs` | **修改** | `REGISTRY` 数组加 `ASR_WYOMING`（现 431-448） | 顺序即 UI 展示顺序 |
| `plugins/registry/fields.rs` | **修改** | 新增 `ASR_WYOMING_FIELDS: &[FieldSchema]`（host/port/language/…），`FieldKind::Str`/`Int` | 现 51 个 `FieldSchema` 初始化器；注意 600 行文件上限（`plugins/registry/fields.rs` 已 525 行 ⇒ **建议新开 `fields_asr.rs`**，仿 `fields_wake.rs`/`fields_command.rs` 的拆分惯例） |
| `plugins/registry/projection.rs` | **无需改** | `schema_json` 从 `REGISTRY` + `CAPABILITY_COMMON_FIELDS` 自动投影 | `projection.rs:1-4` 说明「从 mod.rs 拆出」 |
| `plugins/health.rs` | **修改（可选）** | `probe_asr`（现 `health.rs:91`）硬编码 `cfg.asr.model` 文件探测。wyoming 无本地模型 ⇒ 需按 `engine_id()` 分支（对齐 `probe_tts` 的 `health.rs:127-130` 写法） | ⚠️ **健康探测的硬纪律：禁止网络请求**（`health.rs:11-14`）——所以**不能**在这里做 TCP 探活，只能按 engine 分支跳过 |
| `config/tests.rs` | **无需改** | `declared_engine_ids_match_registry_implementations`（52-85）**自动覆盖**：它遍历 `<CAP>_ENGINES` 表断言双向一致。加了 `ASR_ENGINES` 条目 + 注册表描述符就绿 | 这正是该测试的设计意图 |
| `config.example.toml` | **修改** | 建议加 `[asr.wyoming]` 注释样例（不设 `engine`） | `config/tests.rs:119-128` 只断言默认值，加注释段安全 |
| `plugins/registry/tests.rs` | **无需改** | `descriptor_ids_are_unique_and_namespaced`（6-22）、`field_keys_exist_in_config`（193-）、`required_capabilities_have_selectable_impl`、`default_config_validates_when_feature_enabled` 全部自动覆盖新描述符 | |
| `server/Cargo.toml` | **修改** | 若用 `tokio::net::TcpStream`：tokio 已在 `features=["full"]`（无需改）。若引入 opus 解码给 Wyoming 用 ⇒ 见 §5.2 | |
| `app/session.rs` | **无需改** | `asr_recognize`（307-321）已是 `spawn_blocking` + 闸门，trait 形状不变 | |
| `engine.rs` | **无需改** | ASR 无热切换路径（§2.4），新 provider 同样只需重启 | |
| `client/.../ConfigFormState.kt` | **修改** | ① `buildConfigJson` 的 `put("asr", …)`（**702-709 行**）当前**硬编码 7 个 sensevoice 字段、不含 `engine` 键** ⇒ 需加 `put("engine", asrEngine)` + `put("wyoming", …)` 子段<br>② `ttsRemoteEngines`（**342 行**声明、**369 行** `addAll`）只有 TTS 侧；ASR 侧需新增同类列表<br>③ `ConfigFormFill.kt` 读 presence 分支<br>④ `ConfigCards.kt` ASR 卡字段区 | ⚠️ 若只改服务端不改客户端，**管理页保存一次就会把 `[asr]` 段覆盖回旧形状**（客户端 753-761 注释明说此坑） |
| `plugins/tts/*`、`plugins/llm/*` | **无需改** | — | |

**ASR 改动点合计：新增 1 文件 + 修改 7~8 文件（其中 2 个可选/拆分）**。

### 3.2 TTS：`[tts].engine = "wyoming"`

| 文件 | 性质 | 具体改动 | 证据/约束 |
|---|---|---|---|
| `plugins/tts/wyoming.rs` | **新增** | `WyomingTts` 实现 `TtsEngine`：`synthesize_stream` + `output_sample_rate` + `name` | 抄 `plugins/tts/aiui.rs:93-131`（含 `ActiveDeadline` 活跃超时，`aiui.rs:46-74`） |
| `plugins/tts/mod.rs` | **修改** | ① `TTS_ENGINES` 加 `EngineSpec { id: "wyoming", aliases: &[] }`（49-58）<br>② `pub struct WyomingTtsConfig` + `#[serde(default)] pub wyoming: WyomingTtsConfig` 挂到 `TtsConfig`（89-92 已有 `kokoro`/`xfyun` 两个私有段，加第三个）<br>③ `pub fn build_wyoming(cfg)`<br>④ `impl Default for TtsConfig`（405-426）补字段 | 私有段模式**已成熟**（`tts.kokoro`/`tts.xfyun`），比 ASR 侧改动更小 |
| `plugins/registry/impls.rs` | **修改** | `sel_tts_wyoming` / `sig_tts_wyoming` / `validate_tts_wyoming` / `build_tts_wyoming_d` / `pub const TTS_WYOMING`（仿 360-374） | |
| `plugins/registry/mod.rs` | **修改** | `REGISTRY` 加 `TTS_WYOMING` | |
| `plugins/registry/fields.rs` | **修改** | `TTS_WYOMING_FIELDS`（host/port/voice/speaker…） | 同 §3.1 的 600 行考虑 |
| `engine.rs` | **修改（关键）** | ① `tts_for(lang)`（**224-262 行**）：现逻辑对非 kokoro 直接返回默认引擎（`engine.rs:229-233` 含 `cfg.tts.engine_id() == "xfyun"` 硬编码分支）——**wyoming 天然落入该分支，零改动即正确** ✅<br>② `refresh_tts`（375-434）**经 `descriptor.build` 分发，无需改动** ✅（`engine.rs:390-392` 注释明说）<br>③ `sig_tts_wyoming` 变化会触发重建 + `tts_cache.clear()`（`engine.rs:426`）+ 清语言池（`engine.rs:423`）——**语义正确，无需改** | 这是注册表统一动作模型的**直接收益**：`agents.md:136-138` 的「不再需要中心化 match 分发」在此得到验证 |
| `app/session/bench.rs` | **修改** | 测试台 `tts_test` 有硬编码分支 **`app/session/bench.rs:140`** `if disk_tts.engine_id() != "xfyun"`（后续 145-155 附近走 xfyun 专属的 vcn 处理）。wyoming 会落入 else 分支，需确认该分支是否正确处理非 xfyun 引擎 | ✅ 行号已核实：`bench.rs:140` |
| `app/voices.rs` | **修改（可选）** | 音色目录探测**强耦合 xfyun**：`voices.rs` 读 `engines.config.tts.xfyun.clone()`（3 处），`engine_id() != "xfyun"` 时返回未配置错误。wyoming 音色需**另建探测路径或直接不做目录** | grep 证据：`app/voices.rs` 中 `tts.xfyun` 出现于 clone/错误文案分支 |
| `plugins/health.rs` | **修改（可选）** | `probe_tts`（127-153）对非 kokoro 直接 `ProbeReport::ok()`（128-130）⇒ **wyoming 零改动即正确** ✅ | |
| `app/ws/config.rs` | **无需改（若无密钥）** | `SECRET_PATHS`（67-75）只在有密钥字段时才需加。Wyoming TTS 通常无密钥 ⇒ **零改动** ✅ | 对比：xfyun 加了 3 条（`api_key`/`api_secret`/`app_id`） |
| `app/ws/config.rs::rewrite_legacy_paths` | **无需改** | 只处理 `tts.backend`→`engine`、`LEGACY_BODY_FIELDS`→`[tts.kokoro]`、`memory.backend`（123-164）。wyoming 无旧写法 ⇒ 不涉及 | |
| `config/overlay.rs` | **修改（可选）** | 设备 overlay 白名单（`overlay.rs:27-31`）目前允许 `["tts","xfyun","voice"]`；若要 per-device 选 wyoming 音色，需加 `["tts","wyoming","voice"]` | |
| `app/prefetch.rs` / `app/downlink*` / `app/session.rs` | **无需改** | TTS 流式回调契约（`prefetch.rs:372-387`）不变 | |
| `client/.../ConfigFormState.kt` | **修改** | ① `ttsRemoteEngines.addAll`（**369 行**）加 `"wyoming" to "Wyoming (本地服务)"`<br>② `buildConfigJson` 的 `put("tts", …)`（**716-740 行**）加 `put("wyoming", …)` 子段<br>③ `ConfigCards.kt` TTS 卡字段区（xfyun 字段区旁） | |
| `config.example.toml` | **修改** | 加 `[tts.wyoming]` 注释样例 | |

**TTS 改动点合计：新增 1 文件 + 修改 6~8 文件**（比 ASR 少，因为 TTS 私有段/注册表模式已跑通两轮）。

### 3.3 WakeWord：**无接入点，不建议纳入本期**

| 文件 | 性质 | 说明 |
|---|---|---|
| — | **不适用** | 见 §2.2：`[wakeup]` 是话术插件非引擎；唤醒词识别在 ESP32 固件；服务端看不到唤醒前音频 |
| `plugins/wakeup/mod.rs` | **无需改** | 它不持有引擎、不参与音频识别 |
| 若强行要做，需新增 | — | ① 新 `Capability::WakeWord` 变体（触发 `registry/tests.rs:44-52` 的**变体数硬断言 11 → 12**，`ALL_CAPABILITIES` 也要加）<br>② 新 trait（每会话有状态，`&mut self`）<br>③ 新描述符 + `impls_of` 分支<br>④ **前置条件：ESP32 固件改为常开上行音频流**<br>⑤ 新增音频路由分支（唤醒前音频 → wakeword，回调命中后再走 VAD→ASR），会改动 `app/session.rs:491-511` `handle_binary` 主路径 |

---

## ④ `agents.md` §5.2b「新增供应商只需改两处」在网络型 Provider 上是否成立

**结论：不成立，但差距是「2 处 → 8~10 处」，且差距全部落在「客户端 + 配置结构」而非「服务端分发逻辑」。**

`agents.md:149-150` 的原话：

> 新增/改名实现时必改的**只有两处**：`plugins/*/mod.rs` 的 `<CAP>_ENGINES` 表（含别名）与注册表描述符；`config/tests.rs::declared_engine_ids_match_registry_implementations` 会守住二者一致。

### 4.1 该论断在**哪些维度上成立**

| 维度 | 是否成立 | 证据 |
|---|---|---|
| 服务端引擎分发逻辑 | ✅ **完全成立** | `engine.rs:390-392`：`refresh_tts` 注释明说「构建入口**由注册表描述符提供**（新增供应商只要加 1 条描述符，本函数不再分发）」 |
| 枚举/签名/工厂的集中 match | ✅ 成立 | P4 已删除（`tts/mod.rs:29-30`「不再需要：枚举变体 / `backend_kind()` / `engine_signature()` / 中心化 `match` 分发」） |
| 字段元数据 | ✅ 自动获得 | `projection.rs` 从 `REGISTRY` 自动投影（`registry/mod.rs:11-14`） |
| 测试守护 | ✅ 自动覆盖 | `declared_engine_ids_match_registry_implementations`（`config/tests.rs:52-85`）双向断言；`field_keys_exist_in_config`（`registry/tests.rs:193-`）遍历 `REGISTRY` |

### 4.2 该论断**不成立**的维度（网络型 Provider 额外成本）

| 额外成本 | 原因 | 是否网络型特有 | 量化 |
|---|---|---|---|
| **① 配置结构加私有段** | SenseVoice 的字段直接在 `[asr]` 本体（`asr/mod.rs:34-58`，无私有段）；Wyoming 需新增 `[asr.wyoming]` 子段 + `AsrConfig` 字段 + `Default` impl | ❌ 非网络特有（xfyun 也有 `[tts.xfyun]`），但 ASR 侧是**首次** | ASR：+1 struct、+3 处改 |
| **② 客户端表单三处同改** | `buildConfigJson` 硬编码字段（`ConfigFormState.kt:702-709` ASR、**716-740** TTS）；`fill()` 读 presence；下拉列表 `ttsRemoteEngines`（**342/369**）。`agents.md:139` 自己承认：「客户端仍需 `ConfigFormState.ttsRemoteEngines` 追加 + 卡片字段区（**P5 目标：字段元数据驱动后只剩 1~2 处**）」——**P5 尚未落地** | ❌ 非网络特有 | TTS：+2~3 文件 |
| **③ 不漏字段的整体回传坑** | 客户端每个段**整段回传**（`ConfigFormState.kt:753-761` 注释：「前端不回传就等于这个段在 UI 里不可配置」）。新增私有段若客户端漏回传，**保存一次就静默丢掉配置** | ❌ 非网络特有 | — |
| **④ 健康探测分支** | `health.rs:91` `Capability::Asr` 硬编码 `cfg.asr.model` 文件探测 | ⚠️ 半特有（远程 provider 无本地文件） | +1 分支 |
| **⑤ `warn_if` 降级语义** | 网络型 provider 需要「配置对但上游不可达」的**软性**表达（`impls.rs:415-418`）。SenseVoice 用 `validate` 硬失败即可 | ✅ **网络型特有** | +1 fn |
| **⑥ 音色目录（仅 TTS）** | `app/voices.rs` 强耦合 xfyun 真实 API 探测 | ✅ 特有 | +1 套探测逻辑或明确不做 |
| **⑦ 测试台硬编码分支** | `app/session/bench.rs` 有 `engine_id() != "xfyun"` 分支 | ✅ 特有 | 需核对 |
| **⑧ 签名语义不同** | 现有 `signature` 是**纯字段字符串**（`impls.rs:67-134`），进程内比对。对短连接 provider 够用；对**长连接 + 连接池** provider，「签名变了→重建引擎」意味着**丢弃连接池**，语义不匹配（详见 §6） | ✅ **网络型特有，且是设计层面的** | 需重新定义「重建」语义 |

**判定**：§5.2b 的「两处」是在**「本地模型 provider + 已跑通两轮的 TTS 私有段模式 + 字段元数据驱动的客户端(P5) 未落地」**这三个隐含前提下的陈述。Wyoming ASR 会同时踩中 ①②④；Wyoming TTS 只需 ②⑥⑦，服务端侧确实接近「两处」。**网络型 Provider 的真正额外成本不在「加一个 provider」，而在「长连接生命周期」与现有「会话开始按签名重建」模型的语义错位（§6）。**

---

## ⑤ PCM ↔ Opus 转码链路分析

### 5.1 结论：**几乎零成本**（本评估最重要的反直觉发现）

引擎 trait 的两侧**本来就是 PCM**：

```
                    ┌───────────── 服务端内部全部是 f32 PCM ─────────────┐
ESP32 --Opus--> [OpusFrameDecoder] --> VAD --> AsrEngine --> Llm --> TtsEngine --> [StreamingResampler] --> [OpusFrameEncoder] --Opus--> ESP32
                  app/audio/opus.rs    vad/     &[f32]              synthesize_stream    app/audio/resample.rs    app/audio/opus.rs
                  :112-118 (f32 out)          +sample_rate:u32      (u32, &[f32])        (sr→downlink_sr)       :53-66 (f32 in)
```

证据：
- `AsrEngine::recognize(&self, samples: &[f32], sample_rate: u32)`（`asr/mod.rs:30`）——**已经是 f32 + 显式采样率**。
- `TtsChunkCallback = Box<dyn FnMut(u32, &[f32]) -> bool + Send>`（`tts/mod.rs:311`）——**回调第一参数就是采样率**。
- `OpusFrameDecoder::decode_frame -> Vec<f32>`（`app/audio/opus.rs:112-118`）——上行解码**直接产出 f32**。
- `OpusFrameEncoder::encode_frame(&mut self, samples: &[f32])`（`app/audio/opus.rs:53-56`）——下行编码**直接吃 f32**（内部才转 i16，见 `opus.rs:55-60`）。
- `StreamingResampler::new(from_sr, to_sr)` + `push(&[f32]) -> Vec<f32>`（`app/audio/resample.rs:27-38`）——**跨分片保持相位连续**，且**无 feature gate**（`grep cfg(feature` 在该文件零命中）。

**所以 Wyoming 的 PCM（16k/16bit/mono）需求**：

| 方向 | 需要的转换 | 现有资产 | 是否新增代码 |
|---|---|---|---|
| **ASR：ESP 上行 → Wyoming** | Opus → f32 → **i16**（若上游要 s16le） | `OpusFrameDecoder`（已）+ `f32_to_i16_16k(pcm, uplink_sr)`（`app/audio/pcm.rs:8`，**已存在**，含非 16k 的线性插值重采样） | **零**（`f32_to_i16_16k` 现在的唯一调用方是 AIUI，见 `pcm.rs:5-6`；直接复用即可） |
| **TTS：Wyoming → ESP 下行** | i16 → f32 → 重采样到 downlink_sr(24k) → Opus | `i16→f32` 是 `opus.rs:117` 的 `s as f32 / 32768.0` 一行；`StreamingResampler`（已） | **零到少量**（在 Wyoming 客户端内部做 i16→f32，反向转换） |

⚠️ **唯一的真实约束**：`audio-chunk` 的 `width`/`rate`/`channels` 是 Wyoming header 里的**必填字段**（协议规定），所以 `WyomingAsr` 必须知道 `self.params.uplink_sr` 并在 header 里声明。上行采样率来自 ESP hello 协商（`SessionParams.uplink_sr`，`app/session.rs:83`），**不是配置项** ⇒ 描述符的 `signature` **不应**包含采样率，而应在每次调用时从 `recognize` 的 `sample_rate` 参数取（`asr/mod.rs:30` 已经把它传进来了 ✅ —— **这个参数的存在是幸运的**）。

### 5.2 `audiopus` 的 feature 门控问题

| 事实 | 证据 |
|---|---|
| `audiopus` 是 `optional`，只在 `sherpa` feature 下引入 | `server/Cargo.toml`：`sherpa = ["dep:sherpa-onnx", "dep:audiopus"]` |
| `default = []` | 同上 |
| **`app/audio/opus.rs` 整文件被 `#[cfg(feature = "sherpa")]` 门控**，非 sherpa 构建提供**返回空帧的占位实现** | `opus.rs:13`、`22`、`35`、`69-82`（encoder 占位）、`89`、`95`、`121-133`（decoder 占位） |
| **生产镜像永远带 sherpa** | `Dockerfile`：`cargo build --release --features sherpa --locked` |

**结论**：
1. **对 Wyoming provider 本身**：`audiopus` 门控**不构成污染**——因为引擎边界是 PCM，Wyoming client 完全不需要碰 Opus。上行 Opus 解码已经是必经之路（`session.rs:497`），下行编码也是（`opus.rs`），两者都在既有 feature 门控内。
2. **对「纯 Wyoming 部署」的场景**：如果用户想**不用 sherpa-onnx**（不要 SenseVoice/Kokoro/Silero，只用远端 Whisper/Piper），那么 `audiopus` 也会被关掉 ⇒ **ESP 链路直接断**（`opus.rs:69-82` 占位实现返回 `Vec::new()`，下行全部为空帧）。这是一个**独立于 Wyoming 的既有问题**，但会在「去 sherpa 化」时被暴露。
3. **建议**（不在本期范围，但需记录）：把 `audiopus` 从 `sherpa` feature 中**解耦**为独立 feature（如 `opus = ["dep:audiopus"]`，`sherpa = ["dep:sherpa-onnx", "opus"]`）。这是 3 行 Cargo.toml 改动，能让「纯 Wyoming + 无 sherpa」部署成立。**未验证**：`Dockerfile` 是否已装 `libopus` 系统包（决定解耦后运行时是否可用）。

### 5.3 N5105 上的编解码开销

| 项 | 判断 | 依据 |
|---|---|---|
| **新增开销 = 0** | Wyoming 替换的是「谁做推理」，编解码链路（`opus.rs` + `resample.rs`）**完全不变** | §5.1 |
| 若 Wyoming ASR 与本地 VAD 并存，VAD 仍在本地跑 | `plugins/vad/mod.rs` 只有 silero（`VAD_ENGINES`，`registry/impls.rs:26-28`），**Wyoming 不涉及 VAD** | ✅ |
| 若未来把 Whisper 也跑在 N5105 上 | 那是**部署问题**（另一台机器跑 whisper.cpp），不是本仓库的编解码开销问题 | — |
| 真正的资源变化 | `TTS 合成 4 线程`（`tts/mod.rs:287-289` 默认 4）会**释放**给别的服务；`docker-compose.yml` 的 `cpus: "3.5"` 限核仍在 | `docker-compose.yml` grep 确认 `cpus: "3.5"` |

---

## ⑥ trait async 化三方案对比

### 6.1 前提：现有 trait 的调用点**极少**

精确 grep 结果（`recognize` / `synthesize_stream` / `chat_stream` 全仓）：

| 方法 | 调用点数 | 位置 |
|---|---|---|
| `AsrEngine::recognize` | **1** | `app/session.rs:318`（`asr_recognize` 内的 `spawn_blocking` 闭包） |
| `TtsEngine::synthesize_stream` | **5** | `app/session/bench.rs:217`、`app/prefetch.rs:372`、`app/bench_api.rs:224`、`plugins/tts/mod.rs:334`（trait 默认实现 `synthesize` 内部）、`plugins/tts/aiui.rs:154`（自检）+ 2 处测试 |
| `LlmProvider::chat_stream` | **4** | `app/session/producer.rs`、`app/session/bench.rs`、`plugins/llm/retry.rs:159`（装饰器）、`plugins/memory/graph.rs` + 2 处测试 |

**这是本评估最重要的有利条件**：ASR 只有 **1 个** 生产调用点，TTS 只有 **3 个**（不含 trait 默认实现与测试）。

### 6.2 三方案对比

| 维度 | (a) trait 内桥接同步<br>（`Handle::block_on` / crossbeam） | (b) trait 改 async | (c) 新增 async 专用 trait，与旧 trait 并存 |
|---|---|---|---|
| **做法** | trait 签名不变（同步）；Wyoming client 内部用 `tokio::runtime::Handle::current().block_on(...)` 或 `crossbeam::channel` 把 async 网络 IO 同步化 | `AsrEngine::recognize` / `TtsEngine::synthesize_stream` 改成 `Pin<Box<dyn Future>>`（抄 `MemoryProvider` 的 `BoxFut`，`memory/mod.rs:61`） | 新增 `AsyncAsrEngine` / `AsyncTtsEngine`，注册表 `Built` 枚举加变体，`PluginHost`/`Engines` 加并行分支 |
| **改动面（服务端）** | **0**（调用点全不动） | ASR：1 个 trait 定义 + 1 个调用点（`session.rs:307-321`）+ 2 个实现（sensevoice/wyoming）<br>TTS：trait + 3 个调用点 + 3 个实现 + `TtsEngine::synthesize` 默认实现（`tts/mod.rs:329-347`）<br>连带：`app/prefetch.rs` 的 `spawn_blocking` 包装（369-390）要拆、`app/inference.rs` 闸门的 `spawn_blocking` 假设（`inference.rs:1-16`）要重读、**`spawn_blocking` 纪律整体作废**（`asr/mod.rs:7-9`、`tts/mod.rs:37-39`、`engine.rs:7-8` 三处模块注释） | **最大**：注册表 `Built` 闭枚举（`registry/mod.rs:383-393`）加 2 变体 → `host.rs:220-246` 的 match 加臂 → `engine.rs` 的 `refresh_*` 加分支 → `session.rs`/`prefetch.rs`/测试台各加「同步 or 异步」分派（约 +8~10 处） |
| **改动面（客户端/配置）** | 0 | 0（配置层不变） | 0 |
| **`Handle::block_on` 死锁风险** | ⚠️ **高**。`spawn_blocking` 里调 `Handle::block_on` 在 tokio 1.x 是**已知危险模式**（blocking 线程允许 block_on，但若 future 内部又用到同一 runtime 的资源则可能耗尽 blocking 池——tokio blocking 池默认上限 512，`engine.rs:15` 已提到这个数字） | 无 | 无 |
| **能否支撑长连接** | ⚠️ 勉强。`block_on` 每次调用都把 future 跑完，连接可跨调用复用（client 内部持 `Mutex<TcpStream>`），但**取消语义弱**：`synthesize_stream` 的 `chunk_cb` 返回 `false` 时如何从 `block_on` 内部退出需要小心设计 | ✅ **最好**：`chunk_cb` 可直接 `async`、取消可在 `.await` 点自然生效（配合 `tokio::select!` 查 `abort` 标志） | ✅ 最好（同 b） |
| **与既有纪律的冲突** | 违反 `agents.md` 的「重推理必须 `spawn_blocking`」硬约定（`asr/mod.rs:7-9`、`tts/mod.rs:37-39`、`engine.rs:7-8`、`app/inference.rs:5-6`）——但 **xfyun 已经是网络 provider 且同样走 `spawn_blocking`**（`aiui.rs:13`：「同步阻塞实现（网络 IO + 分帧读取），调用方必须 `spawn_blocking` 隔离」），**所以并不新增冲突** | **废除**该约定。`app/inference.rs` 整章的推理闸门设计前提（`spawn_blocking` 无并发上限 ⇒ 需要 Semaphore）依然成立（闸门本身与 async 无关），但注释需大面积重写 | 新增分支，但旧路径保留 ⇒ **两套心智模型共存** |
| **测试影响** | 0 | `plugins/tts/tests.rs`、`aiui/tests.rs`、`prefetch/tests.rs`、`prefetch/tests/gates.rs`（都有 `fn synthesize_stream` 桩）需加 `Box::pin(async move {…})` | 旧测试不动，新 trait 另写 |
| **可参考的先例** | `plugins/tts/aiui.rs`（**已在生产跑的网络 provider，形态几乎一致**） | `plugins/memory/mod.rs:61-102` 的 `BoxFut` 模式（**本仓库既有 async trait 范式**）；`plugins/llm/mod.rs:166-173` 的手动装箱 future（注释明说「与 `Transport` 同风格，**不引 async-trait**」） | 无 |

### 6.3 推荐：**先 (a)，不主动做 (b)/(c)**

**理由**：

1. **有生产验证的同构先例**：`plugins/tts/aiui.rs` 是「同步 trait + 同步阻塞网络 IO + `spawn_blocking` 隔离 + 流式 PCM 回调」，与 Wyoming TTS 的需求**几乎逐条对应**（流式分片、活跃超时、取消回调）。抄它的风险最低，且注释/测试/超时记账（`ActiveDeadline`，`aiui.rs:46-74`）都是现成的。
2. **改动面差一个数量级**：(a) 服务端 0 改动 vs (b) 约 10+ 处（含 3 处模块级 `//!` 注释纪律作废）。
3. **(b) 的收益在第一期拿不到**：(b) 的核心价值是「长连接 + 精细取消」，但第一期若采用「每次调用一条短连接」（§7.2 路线 A），(a) 与 (b) 的行为差异为零。
4. **(c) 明确不推荐**：它让注册表 `Built` 闭枚举分裂（`registry/mod.rs:22-23` 明确写「闭枚举而非 `Box<dyn Any>`，类型擦除后向下转型会把编译期错误变成运行时 panic」），违背本仓库的核心设计决策。

**触发 (b) 的时机（决策门）**：当且仅当满足以下**全部**条件时重新评估——
- 需要真正的**进程级连接池 / keep-alive 复用**（短连接的握手开销成为实测瓶颈）；
- 需要 `synthesize_stream` 的取消在**分片间隙**精确生效（当前 `aiui.rs` 的做法是回调返回 `false` → 关连接止损，已够用）；
- 长连接带来跨调用状态（如 Piper 的 speaker 会话状态）。

即便如此，**优先尝试 (a) + 更细的连接管理**（见 §7.2 路线 B），因为 (b) 的成本主要在「废除 spawn_blocking 纪律」这一组织性成本。

---

## ⑦ crate 边界草案与抽/不抽决策

### 7.1 两种 crate 边界草案

#### 草案 A：`wyoming-protocol`（纯编解码，无 IO）

```
crates/wyoming-protocol/
├── src/
│   ├── lib.rs
│   ├── header.rs      # Header { type, data: Value, data_length, payload_length }
│   ├── reader.rs      # Reader: 从 &[u8] 增量解出 (Header, additional_data, payload)
│   ├── writer.rs      # Writer: 写 header 行 + data_length 段 + payload
│   ├── types.rs       # AudioChunk / AudioStart / AudioStop / Transcribe /
│   │                 # Transcript / TranscriptStart / TranscriptChunk /
│   │                 # TranscriptStop / Synthesize / Info / Ping
│   └── error.rs
└── Cargo.toml         # 依赖：serde, serde_json（无 tokio、无 opus、无本仓库类型）
```

- **纯同步 + 纯内存**：`Reader`/`Writer` 工作在 `&[u8]` / `Write`，**不做 socket IO** ⇒ 可在 `cargo test` 里用 `Vec<u8>` 全量断言，无需起服务。
- 依赖面极小：`serde` + `serde_json`（两者已在 `server/Cargo.toml`）。

#### 草案 B：`wyoming-client`（含 TCP 客户端）

```
crates/wyoming-client/
├── src/
│   ├── lib.rs
│   ├── transport.rs   # TcpTransport（同步 std::net::TcpStream + BufReader）
│   ├── asr.rs         # WyomingAsrClient: transcribe(stream) -> Result<String>
│   ├── tts.rs         # WyomingTtsClient: synthesize_stream(text, cb)
│   ├── wake.rs        # WyomingWakeWordClient（本期不用，预留）
│   └── reconnect.rs   # 断线重连 + 指数退避
└── Cargo.toml         # 依赖：tokio(仅 sync/rt) 或 std + serde_json + tracing
```

#### 本仓库的放置形态（无论抽不抽 crate）

```
server/src/plugins/wyoming/          ← 或 server/src/plugins/asr/wyoming.rs + tts/wyoming.rs
├── mod.rs          # 协议常量、消息类型、feature 门控注释
├── proto.rs        # header 编解码（= 草案 A 的全部）
├── client.rs       # 同步 TCP 连接 + 请求/响应配对
├── asr.rs          # WyomingAsr: impl AsrEngine
├── tts.rs          # WyomingTts: impl TtsEngine
└── tests.rs
```

### 7.2 `sherpa` feature 的隔离问题

| 方案 | feature 影响 | 判断 |
|---|---|---|
| 放 `server/src/plugins/wyoming/`（**不抽 crate**） | ✅ **零 feature 影响**。Wyoming provider 无 `#[cfg(feature = "sherpa")]`（与 `tts/aiui.rs` 同款——远程实现无 feature 门槛，见 `plugins/mod.rs:21-22`：「厂商实现文件中的 `#[cfg(feature = "sherpa")]` 仅圈定本地引擎；**远程/云厂商实现无 feature 门槛**」） | ✅ |
| 抽独立 crate + 默认 feature 包含 | ⚠️ 需在 `server/Cargo.toml` 加 `wyoming = { path = "...", optional = true }`。若设为**非 optional**（默认启用），则 `default = []` 的构建会多编译 serde_json（已存在）+ 新 crate 本体（无原生依赖）⇒ **成本可忽略** | ✅ 可行 |
| 抽独立 crate 且引入 opus | ❌ 会把 `audiopus`（需系统 libopus）传染到 crate 边界 | ⚠️ **建议 crate 内不做 opus**，opus 编解码留在 `server/src/app/audio/`（§5.1 已证明不需要跨界） |
| 抽 crate 且 crate 内用 tokio async | ⚠️ `server/Cargo.toml` 已有 `tokio = features=["full"]`，无冲突；但会诱使实现走 async（§6 推荐不这么做） | ⚠️ 与 §6 结论冲突 |

### 7.3 抽 / 不抽的决策

**推荐：先在仓库内做（`server/src/plugins/wyoming/`），不立即抽 crate。**

| 判据 | 抽出 crate | 留在仓库内 |
|---|---|---|
| 是否有第二个消费者 | 需要（如另一个 Rust 项目也想用 Wyoming） | ❌ 当前无 |
| 本仓库的既有约定 | ❌ **本仓库是单 crate**（`server/Cargo.toml` 唯一 Cargo.toml，workspace 根无 `Cargo.toml`）⇒ 抽 crate 需**先引入 workspace 结构**（新 `Cargo.toml` + 改 Dockerfile `COPY` 路径 + 改 `agents.md` 构建命令） | ✅ 与 §5.10「逻辑文件 ≤ 600 行」拆分惯例一致（照 `registry/fields_*.rs`、`session/*.rs` 的做法分文件即可） |
| 600 行上限压力 | proto 编解码约 300-400 行 + 3 个 client 约 400 行 = 单模块可能超限 ⇒ 抽 crate 有帮助 | 拆 `plugins/wyoming/{proto,client,asr,tts}.rs` 即可，每个都远小于 600 行 |
| 「Rust Server 完全不需要知道 Whisper/Piper 怎么运行」这个目标 | ⚠️ **抽 crate 并不达成该目标**。该目标由「provider trait + 注册表」达成（`registry/mod.rs:14`：「新增实现 = 表里加 1 条」），与 crate 边界无关。抽出 crate 后 `server` 仍然知道 `WyomingAsr: impl AsrEngine` | 同左 |
| 版本/依赖管理 | 引入跨 crate 版本协调（协议版本 vs 本仓库版本） | ✅ 无 |
| 未来迁移成本 | 现在抽 = 零迁移成本；将来抽 = 一次机械搬移（`git mv` + 加 `pub use`），**成本很低**（因为协议层无本仓库类型依赖） | — |

**决策**：**先在仓库内做，但按「未来可抽」的边界写代码**——即 `proto.rs` **不得引用任何 `crate::` 下的类型**（只用 `serde_json::Value` + 标准库），这样将来 `git mv crates/wyoming-protocol/` 是纯机械操作。这是零成本换取未来灵活性。

**唯一现在就值得抽的判据**：如果团队**同时**在做第二个 Rust Wyoming 消费者（未在本次需求中提及 ⇒ **未验证**）。

### 7.4 `LlmProvider` 本期**不动**

| 判据 | 依据 |
|---|---|
| 用户需求聚焦 ASR/TTS/WakeWord | 需求原文：「另 --HTTP--> FunASR / CosyVoice」——**FunASR 是 ASR、CosyVoice 是 TTS**，都不是 LLM |
| `LlmProvider` 已经是 async | `llm/mod.rs:166-173` 手动装箱 future；`llm/mod.rs:14-15` 注释：「LLM 是**异步 HTTP 调用**，无需 `spawn_blocking` 隔离」⇒ **没有同步/异步错位问题** |
| Wyoming **没有 LLM 服务** | 协议的服务清单是 `asr`/`tts`/`wake`/`stt`/`intent`/`mic`/`snd`/`satellite`（Wyoming README 的 `describe` 响应字段），**无 LLM/对话类服务** | 
| 现有 HTTP 路径足够 | `reqwest` + rustls（`Cargo.toml`）已满足 FunASR/CosyVoice 的 HTTP 调用；LlmProvider 只实现了 OpenAI Responses 一种方言（`llm/mod.rs:23-26`），加 FunASR/CosyVoice 属于**HTTP provider 接入**，与 Wyoming 无关 |

**结论**：`LlmProvider` 与本期正交，**不动**。若后续要接 FunASR/CosyVoice，那是「HTTP 型 ASR/TTS provider」，**恰好可以作为 Wyoming 之外的对照组**，验证「同一能力两种传输」是否真的只需加 1 条描述符。

---

## ⑧ 分期路线建议

### 8.1 三条候选路线的取舍

| 路线 | 内容 | 收益 | 成本 | 风险 |
|---|---|---|---|---|
| **R1：Wyoming TTS 先行** | 只做 `[tts].engine="wyoming"`（Piper） | ✅ **最高**：直接替换掉 N5105 上最重的 4 线程本地合成（`tts/mod.rs:287-289`），把 CPU 让给 ASR；Piper 音质/多音色优于 Kokoro；现有 `aiui.rs` 模式可 1:1 抄 | 新增 1 文件 + 修改 6~8（§3.2） | 低。`engine.rs:224-233` 的 `tts_for` 对非 kokoro 自动走默认引擎分支，**零改动即正确** |
| **R2：Wyoming ASR 先行** | 只做 `[asr].engine="wyoming"`（whisper） | 中：ASR 精度/多语种提升；释放 sherpa 的 2 线程 | 新增 1 文件 + 修改 7~8（§3.1）；客户端 ASR 卡改动更大（`ConfigFormState.kt:702-709` 硬编码 7 字段） | 中。ASR 私有段是**首次**新建（无先例）；ASR 无热切换（本来就要重启） |
| **R3：Wyoming WakeWord** | 服务端唤醒词 | ❌ **不可行**（§3.3） | — | 阻塞于固件改造 |

### 8.2 推荐分期

| 期 | 范围 | 交付物 | 验收 |
|---|---|---|---|
| **P0（本期）** | **仅 Wyoming TTS**，**路线 A：每次调用一条短连接**（与 `aiui.rs` 完全同构，零架构改动） | `plugins/tts/wyoming.rs` + 注册表 1 条 + `[tts.wyoming]` 私有段 + 客户端 TTS 卡 + `config.example.toml` | ① `[tts].engine="wyoming"` 启动成功、`/api/plugins` 显示 active<br>② macApp 测试台 `tts_test` 首片延迟不劣于 xfyun<br>③ **长回复（≥10 句）真机不断续**（这是 `agents.md:152` §5.2c 的核心验收，必须实测）<br>④ 打断（abort）在 200ms 内停止 |
| **P1** | Wyoming ASR，同样路线 A | `plugins/asr/wyoming.rs` + `[asr.wyoming]` + 客户端 ASR 卡 | ① `asr_test` 整段识别正确<br>② 延迟对比 SenseVoice 基线<br>③ 中文/中英混说实测 |
| **P2（可选）** | **路线 B：长连接 + 连接池**。此时才重新评估 §6 的 (b) async 化 | 连接复用 + 断线重连 | 握手开销从 ~100ms 降到 ~0 |
| **P3（可选）** | 抽 `crates/wyoming-protocol`（若出现第二个消费者） | 纯编解码 crate | `proto.rs` 零 `crate::` 依赖（第一期就保证） |
| **不做** | Wyoming WakeWord | — | 阻塞于 ESP32 固件常开音频流 |

**为什么 TTS 先于 ASR**：

1. **注册表模式已跑通两轮**（`tts.kokoro` + `tts.xfyun`），`TtsConfig` 已有两个私有段的现成模式（`tts/mod.rs:89-92`）；ASR 侧是**首次**建私有段（`AsrConfig` 现在是平铺的，`asr/mod.rs:34-58`）。
2. **`engine.rs` 的 TTS 热切换路径完整可用**（`refresh_tts`，`engine.rs:375-434`），改完立刻能热切换；ASR 无此路径（§2.4）⇒ ASR 的每次调整都要重启验证，迭代慢。
3. **收益更直接**：省掉 N5105 上 4 线程本地合成（`tts/mod.rs:287-289` + `docker-compose.yml` `cpus:"3.5"` 限核背景），直接缓解 §5.2c 的断续问题根因（agents.md:172-175 第 6 条「上游持续略慢于实时」）。
4. **风险更低**：TTS 的下游有三段缓冲兜底（`prefetch.rs` 下载缓冲 + `downlink/reservoir.rs` 水库 + `DownlinkPacer`，见 `agents.md:205-215`），单句失败不会立刻毁掉整轮；ASR 失败直接丢整段话（`session.rs:345-348` 只 `continue`）。

---

## ⑨ 未决问题（需要用户拍板）

| # | 问题 | 选项 | 我的建议与依据 |
|---|---|---|---|
| **1** | **本期先做 TTS 还是 ASR？** | (a) 先 TTS（推荐）<br>(b) 先 ASR<br>(c) 两个都做 | **(a) 先 TTS**。理由见 §8.2 四条：TTS 私有段/注册表/热切换三条路径均已跑通两轮，ASR 侧是首次；收益更直接（省 4 线程）；风险更低（三段缓冲兜底）。 |
| **2** | **Wyoming 上游进程由谁管？** | (a) 用户自己在别的主机上跑 whisper.cpp / Piper（**推荐**：本仓库只做 client）<br>(b) 本仓库负责拉起/守护子进程<br>(c) docker-compose 增加 whisper/piper 服务容器 | **(a)**。用户诉求原文「Rust Server 完全不需要知道 Whisper/Piper 怎么运行」正好对应 (a)；(b) 会引入进程守护/日志/健康检查的全新职责，与本仓库「能力抽象 + 状态上报」的边界不符。⚠️ 但 (a) 需要用户确认**目标部署形态**（是否打算把 whisper/piper 与本 server 放同一台 N5105？若是，CPU 竞争会让 §8.1 的 TTS 收益打折）。 |
| **3** | **WakeWord 是否接受「本期不做」？** | (a) 接受，本期只做 ASR+TTS<br>(b) 要求一并做服务端唤醒词 | 需要向用户说明**硬阻塞**：`[wakeup]` 是话术插件（`wakeup/mod.rs:1`），唤醒词识别在 ESP32 固件里（`app/session/wakeup.rs:9-11`），服务端看不到唤醒前音频（`session.rs:491-511`）。做服务端唤醒词 = **固件先改造成常开音频上行**。建议 (a)，并把「服务端唤醒词」列为独立需求（关联固件仓库）。 |
| **4** | **是否要抽独立 `wyoming` crate？** | (a) 先在仓库内做，按可抽边界写（**推荐**）<br>(b) 立即抽 workspace crate | **(a)**。本仓库当前是**单 crate**（只有 `server/Cargo.toml`），抽 crate 需先引入 workspace + 改 Dockerfile。且「Server 不知道 Whisper/Piper 怎么运行」这个目标由 provider trait + 注册表达成，**与 crate 边界无关**。建议要求实现时 `proto.rs` 零 `crate::` 依赖，将来 `git mv` 是机械操作。 |
| **5** | **`LlmProvider` 是否纳入？** | (a) 不动（**推荐**）<br>(b) 一并抽象 | **(a)**。需求里的 FunASR/CosyVoice 是 ASR/TTS 不是 LLM；Wyoming 协议本身**没有 LLM 服务**；且 `LlmProvider` 已是 async（`llm/mod.rs:166-173`），无同步/异步错位。 |

**补充需要用户确认（次要，但会影响实施）**：

| # | 问题 | 说明 |
|---|---|---|
| 6 | Wyoming 上游是否需要**双向流式 ASR**（`transcript-start`/`transcript-chunk`/`transcript-stop`）？ | 本仓库 `AsrEngine` 是「整段 → 整句」（`asr/mod.rs:30`），**架构上无法消费流式转写**。若上游 whisper.cpp 只能开流式 transcribe，需在 provider 内收集完再返回（可行，但浪费了流式的低延迟优势）。 |
| 7 | 是否需要 `language` 参数透传？ | `AsrConfig` 已有 `language`（`asr/mod.rs:43-44`，默认 `"auto"`）；Wyoming `transcribe` 请求支持 `language` 字段。**未验证** whisper.cpp 的 Wyoming 服务端对 `auto` 的处理（可能只接受具体语言码）⇒ 可能需要 normalize 映射表。 |
| 8 | TTS 音色（speaker）如何映射？ | `TtsEngine::synthesize_stream` 有 `speaker: i32` 跨实现参数（`tts/mod.rs:322`）；Wyoming `synthesize` 用字符串 `speaker-id`。需要一个 id 映射/透传约定。**未验证** Piper 的 Wyoming 服务端 `speaker-id` 取值规则。 |
| 9 | `[audio].downlink_sample_rate` 与 Piper 输出采样率的关系？ | Piper 通常输出 16k/22.05k；`TtsEngine::output_sample_rate()` 默认 24_000（`tts/mod.rs:350-352`）。`StreamingResampler`（`resample.rs:27`）会做线性插值重采样 ⇒ **技术上没问题**，但 16k→24k 升采样会引入轻微质量损失，需实测是否可接受（xfyun 也是 16k，`aiui.rs:127-129`）。 |

---

## 附录 A：证据索引（本评估引用的全部代码位置）

### A.1 Trait 定义
| 内容 | 位置 |
|---|---|
| `AsrEngine`（单方法同步） | `server/src/plugins/asr/mod.rs:29-31` |
| `ASR_ENGINES`（仅 sensevoice） | `server/src/plugins/asr/mod.rs:22-25` |
| `AsrConfig`（**无私有段**，7 个平铺字段） | `server/src/plugins/asr/mod.rs:34-58` |
| `AsrConfig::normalize` | `server/src/plugins/asr/mod.rs:65-69` |
| `TtsEngine`（流式 + 3 个默认方法） | `server/src/plugins/tts/mod.rs:315-358` |
| `TtsChunkCallback`（`(u32, &[f32]) -> bool`） | `server/src/plugins/tts/mod.rs:311` |
| `TTS_ENGINES`（kokoro + xfyun） | `server/src/plugins/tts/mod.rs:49-58` |
| `TtsConfig`（已有 2 个私有段） | `server/src/plugins/tts/mod.rs:63-117`（`kokoro`:89、`xfyun`:92） |
| `LEGACY_BODY_FIELDS` | `server/src/plugins/tts/mod.rs:124-133` |
| `LlmProvider`（手动装箱 future） | `server/src/plugins/llm/mod.rs:163-174` |
| `LlmEvent` | `server/src/plugins/llm/mod.rs:136-143` |
| `VadEngine`（仅 `Send`，`&mut self`） | `server/src/plugins/vad/mod.rs:26-31` |
| `MemoryProvider`（**async，`BoxFut`**） | `server/src/plugins/memory/mod.rs:69-102` |
| `BoxFut` 定义 | `server/src/plugins/memory/mod.rs:61` |
| `WakeGreeting`（**struct，非引擎**） | `server/src/plugins/wakeup/mod.rs:134-164` |
| `DEFAULT_GREETING = "我在"` | `server/src/plugins/wakeup/mod.rs:43` |

### A.2 注册表与宿主
| 内容 | 位置 |
|---|---|
| `Capability`（11 变体闭枚举） | `server/src/plugins/registry/mod.rs:73-96` |
| `ALL_CAPABILITIES` | `server/src/plugins/registry/mod.rs:99-111` |
| `HotReload` 三档 | `server/src/plugins/registry/mod.rs:182-189` |
| `FieldSchema` | `server/src/plugins/registry/mod.rs:365-376` |
| `Built` 闭枚举 | `server/src/plugins/registry/mod.rs:383-393` |
| `PluginDescriptor`（12 字段） | `server/src/plugins/registry/mod.rs:396-422` |
| `REGISTRY`（14 条描述符） | `server/src/plugins/registry/mod.rs:431-448` |
| `impl_id`（`<cap>.<impl>` 拆分） | `server/src/plugins/registry/mod.rs:466-471` |
| `unknown_engine_hint` | `server/src/plugins/registry/mod.rs:501-509` |
| `signature_of` | `server/src/plugins/registry/mod.rs:512-517` |
| `sel_*` / `sig_*` / `validate_*` / `build_*` 全集 | `server/src/plugins/registry/impls.rs:20-305` |
| `ASR_SENSEVOICE`（`hot: RestartOnly`） | `server/src/plugins/registry/impls.rs:309-323` |
| `TTS_KOKORO` / `TTS_XFYUN` | `server/src/plugins/registry/impls.rs:343-374` |
| `WAKE_GREETING`（`build: None`） | `server/src/plugins/registry/impls.rs:556-570` |
| `PluginHost::plan` | `server/src/plugins/host.rs:117-173` |
| `PluginHost::boot` | `server/src/plugins/host.rs:182-269` |
| `PluginHost::build_optional` | `server/src/plugins/host.rs:275-299` |
| `Phase` 四档 | `server/src/plugins/host.rs:33-42` |

### A.3 引擎生命周期
| 内容 | 位置 |
|---|---|
| `Engines.asr`（**不可变 pub 字段**） | `server/src/engine.rs:47` |
| `Engines.tts`（`RwLock`，可热替换） | `server/src/engine.rs:49` |
| `Engines::tts_for`（**xfyun 硬编码分支**） | `server/src/engine.rs:224-262`（分支 229-233） |
| `Engines::refresh_from_disk`（**只覆盖 Tts/Llm/Memory**） | `server/src/engine.rs:293-321` |
| `Engines::refresh_tts`（**经 `descriptor.build` 分发**） | `server/src/engine.rs:375-434` |
| `decide_refresh`（纯函数热切换决策） | `server/src/engine.rs:535-560` |
| `refresh_from_disk` 调用点（会话开始） | `server/src/app/session/start.rs:41-46` |

### A.4 调用点（async 化波及面）
| 内容 | 位置 |
|---|---|
| `AsrEngine::recognize` **唯一**调用点 | `server/src/app/session.rs:318`（在 `asr_recognize` 307-321 的 `spawn_blocking` 内） |
| `blocking_result`（`spawn_blocking` 包装） | `server/src/app/session.rs:66-76` |
| `recognize_segments`（ASR 失败只 `continue`） | `server/src/app/session.rs:324-369`（失败分支 345-348） |
| `TtsEngine::synthesize_stream` 调用点 ×3 | `app/session/bench.rs:217`、`app/prefetch.rs:372`、`app/bench_api.rs:224` |
| `prefetch` 的闸门 + `spawn_blocking` + 回调 | `server/src/app/prefetch.rs:352-390` |
| `InferenceGates::asr/tts/reconfigure_tts` | `server/src/app/inference.rs`（`pub fn asr` / `tts` / `reconfigure_tts`） |
| `LlmProvider::chat_stream` 调用点 | `app/session/producer.rs`、`app/session/bench.rs`、`plugins/llm/retry.rs:159`、`plugins/memory/graph.rs` |

### A.5 音频链路
| 内容 | 位置 |
|---|---|
| `OpusFrameDecoder::decode_frame -> Vec<f32>` | `server/src/app/audio/opus.rs:112-118` |
| `OpusFrameEncoder::encode_frame(&[f32])` | `server/src/app/audio/opus.rs:53-66` |
| `OpusFrame*` 的 `#[cfg(feature="sherpa")]` 门控 + 占位实现 | `opus.rs:13/22/35/69-82/89/95/121-133` |
| `f32_to_i16_16k`（**已存在，AIUI 专用**） | `server/src/app/audio/pcm.rs:8-27` |
| `StreamingResampler`（**无 feature gate**） | `server/src/app/audio/resample.rs:27-38` |
| `handle_binary`（上行 Opus→VAD→ASR） | `server/src/app/session.rs:491-511` |
| `SessionParams.uplink_sr`（来自 hello 协商） | `server/src/app/session.rs:83` |
| `audio/mod.rs` 三子模块 | `server/src/app/audio/mod.rs` |

### A.6 配置层
| 内容 | 位置 |
|---|---|
| `Config` 12 个段 | `server/src/config.rs:115-150` |
| `Config::normalize`（唯一入口，幂等） | `server/src/config.rs:425-435` |
| `EngineSpec` / `canonical_engine`（唯一权威） | `server/src/config.rs:83-113` |
| 私有段约定（文档） | `server/src/config.rs:20-30` |
| `SECRET_PATHS`（6 条密钥路径） | `server/src/app/ws/config.rs:67-75` |
| `rewrite_legacy_paths`（tts/memory 旧路径重写） | `server/src/app/ws/config.rs:123-164` |
| `merge_client_patch` | `server/src/app/ws/config.rs:191-204` |
| 设备 overlay 白名单 | `server/src/config/overlay.rs:27-31` |
| `declared_engine_ids_match_registry_implementations` | `server/src/config/tests.rs:52-85` |
| `descriptor_fields_paths_follow_convention` | `server/src/config/tests.rs:88-` |
| `example_config_parses_and_normalizes` | `server/src/config/tests.rs:119-` |
| `field_keys_exist_in_config` | `server/src/plugins/registry/tests.rs:193-` |
| 变体数硬断言（11） | `server/src/plugins/registry/tests.rs:44-52` |

### A.7 客户端
| 内容 | 位置 |
|---|---|
| `buildConfigJson` 的 `put("asr", …)`（**硬编码 7 字段，无 engine**） | `client/shared/src/commonMain/kotlin/com/xiaozhi/admin/ConfigFormState.kt:702-709` |
| `buildConfigJson` 的 `put("tts", …)`（含 kokoro/xfyun 子段） | 同上 `:716-740` |
| `buildOverlayJson` 的 `put("tts", …)` | 同上 `:861` |
| `ttsRemoteEngines` 声明 / `addAll` | 同上 `:342` / `:369` |
| `xfyunCredsReady()` | 同上（`grep -n 'xfyunCredsReady'` 命中定义 + 2 处调用） |
| `asrModel` 表单字段声明 | 同上（`grep -n 'asrModel'` 命中声明 + `put("model", asrModel)`） |
| 整段回传的坑（注释） | 同上 `:753-761` |
| `ConfigFormFill.kt` / `ConfigCards.kt` | `client/shared/src/commonMain/kotlin/com/xiaozhi/admin/` |

### A.8 网络型 Provider 先例（最关键的可抄模板）
| 内容 | 位置 |
|---|---|
| **AIUI TTS（同步阻塞网络 + 流式 PCM 回调）** | `server/src/plugins/tts/aiui.rs:93-131` |
| ├ 模块注释「同步阻塞实现，调用方必须 `spawn_blocking`」 | `aiui.rs:13-14` |
| ├ 「每次调用一条短连接（oneshot 语义），不持有跨请求状态」 | `aiui.rs:14` |
| ├ `ActiveDeadline`（**活跃超时 vs 墙上时钟**） | `aiui.rs:46-74` |
| ├ `TOTAL_TIMEOUT=30s` / `READ_TIMEOUT=15s` | `aiui.rs:38-40` |
| ├ `output_sample_rate() = 16_000` | `aiui.rs:127-129` |
| └ 凭据结构 + HMAC 签名助手 | `plugins/tts/xfyun.rs` |
| `aiui::test_credentials`（凭据探测，`spawn_blocking`） | `app/voices.rs:309-310` |
| `app/voices.rs` 强耦合 xfyun | `grep 'tts.xfyun'` 命中 3 处 `clone()` + 错误文案分支 |
| `plugins/mod.rs`：「远程/云厂商实现无 feature 门槛」 | `plugins/mod.rs:21-22` |

### A.9 构建与部署
| 内容 | 位置 |
|---|---|
| `sherpa = ["dep:sherpa-onnx", "dep:audiopus"]`，`default = []` | `server/Cargo.toml` |
| `tokio features = ["full"]` | `server/Cargo.toml` |
| Dockerfile 恒带 `--features sherpa` | `Dockerfile`（`cargo build --release --features sherpa --locked`） |
| `cpus: "3.5"` 限核 | `docker-compose.yml` |
| 推理闸门/线程预算 | `server/src/engine.rs:4-17`（模块注释） |
| TTS 4 线程默认 | `server/src/plugins/tts/mod.rs:281-289` |
| 「文件规模 ≤ 600 行」硬约定 | `agents.md:598` |
| §5.2b「只需改两处」 | `agents.md:149-150` |
| §5.2b「客户端仍需 ttsRemoteEngines + 卡片字段区（P5 目标）」 | `agents.md:139` |
| §5.2c 下行节流 + 逐句预取 | `agents.md:152-` |
| §5.2c 三段缓冲链路 | `agents.md:205-215` |
| §5.2c「在线合成总超时不得用墙上时钟」 | `agents.md:222-224` |
| §5.10 文件规模 | `agents.md:598` |

---

## 附录 B：Wyoming 协议要点（外部资料，非本仓库证据）

来源：<https://github.com/OHF-Voice/wyoming> README（Open Home Foundation 标准，Rhasspy / Home Assistant 生态共用）。

| 项 | 值 |
|---|---|
| 传输 | 裸 TCP（**无认证、无加密**，设计前提是可信局域网） |
| 帧格式 | JSON header 单行（`\n` 结尾）+ 可选 `data_length` 附加数据 + 可选 `payload_length` 二进制负载 |
| header 字段 | `type`（必填）、`data`（可选对象）、`data_length`、`payload_length` |
| 音频 | `audio-start` / `audio-chunk` / `audio-stop`；`audio-chunk` 必填 `rate`(Hz) / `width`(采样字节数) / `channels`，`timestamp` 可选；负载为**裸 PCM** |
| ASR | 请求 `transcribe`（`name` / `language` / `context` 可选）；响应 `transcript`（`text` 必填 / `language` / `context`）。流式：`transcript-start` → `transcript-chunk`(`text`) → `transcript-stop`（**必须仍发一次 `transcript` 以兼容旧版**） |
| TTS | 请求 `synthesize`（`text` / `voice-id` / `speaker-id` / `speed`）；响应 `audio-start` → `audio-chunk`×N → `audio-stop` |
| 服务发现 | `describe` 请求 → 响应含 `asr`/`tts`/`wake`/`stt`/`mic`/`snd`/`satellite`/`handle`/`intent` 数组，元素含 `models`/`speakers`/`attribution`/`supports_*_streaming` 等 |
| Wake | `wake` 服务存在；实现有 openWakeWord / porcupine1 / snowboy / microWakeWord |
| **无 LLM** | 服务清单中**没有** LLM/对话类服务 |

**对本仓库的三点映射**：

1. `audio-chunk.rate/width/channels` 必填 ⇒ `WyomingAsr` 必须用 `recognize` 的 `sample_rate` 参数（`asr/mod.rs:30`）填 `rate`，`width=2`、`channels=1` 为常量。✅ 幸运：trait 已经把采样率传进来了。
2. ASR 的 `transcript` 响应是**一次性**的 ⇒ 与 `AsrEngine::recognize -> Result<String>` **完全同构**，不需要流式改造。流式 `transcript-chunk` 可忽略（但 `supports_transcript_streaming` 可作为 `warn_if` 的信息位）。
3. TTS 的 `synthesize` 响应是**流式 audio-chunk** ⇒ 与 `TtsEngine::synthesize_stream` 的回调契约**完全同构**，且 Wyoming 生态的 Piper 支持 `supports_synthesize_streaming`。**这推翻了「Wyoming TTS 可能整段返回」的顾虑**（我在初判时曾这么假设）。
