# 接入 ESP 小智 · 整条语音流水线延迟与性能优化方案（重排版）

> **本版约束（用户 2026-10-02 明确）**
> 1. **ESP 兼容性视为已满足的前提**——不再把它当待办 backlog，仅保留事实备查。
> 2. **核心目标 = 优化整条流水线（ASR→LLM→TTS→下行）的延迟与性能**。
> 3. **不使用过期/作废技术方案**；**该用 MCP 就用 MCP**——取代 `iot` 与当前的 `mcp` 占位回显。
> 4. **`client/`（Mac app）定位 = ESP 性能对齐验证台**，不是独立产品；用来在桌面端复现 ESP 类约束并量测 TTFA/吞吐/打断，与真机实测比对。
>
> 本文档是方案/落地清单，**不动代码**；每条带 `file:line` 与固件佐证。协议事实来自
> `xiaozhi-esp32` 仓库 `main/protocols/websocket_protocol.cc`、`websocket_protocol.h` 与 `docs/websocket_zh.md`、`docs/mcp-protocol_zh.md`（2026-10-02 核对）。
> **协议版本以最新为准**：二进制协议 **v3** 为最新简化版（`u8 type | u8 reserved | u16 payload_size(LE) | payload`，无 version 字段，靠握手协商）；固件 `version_` 默认 **1** 但可经设备配置升到 **2/3**，且上行/下行严格共用同一 `version_`——server 下行必须跟随设备 `hello.version`，不得锁定老 v1。

---

## 0. 前提与边界（已锁定，不再作为待办）

### 0.1 ESP 兼容性视为已满足（事实备查，非行动项）

| 协议点 | 状态 | 出处 |
|---|---|---|
| 握手：设备先发文本 `hello`，服务器回 `type=hello`+`transport=websocket`+下行 `audio_params` | ✅ 已实现（`ws.rs`） | `websocket_zh.md` §3/§4.2 |
| **下行二进制版本跟随设备 `hello.version`**（固件 `version_` 默认 1、可配 2/3；v3 为最新简化版；server 一律跟随，不锁定老 v1） | ✅ 已修（RISKY 档，`wrap_downlink` 用入参 `v`） | `websocket_protocol.cc` 双向同 `version_`；`websocket_protocol.h: int version_ = 1` |
| `stt`/`llm`/`tts start·sentence_start·stop` 回包 | ✅ 已实现 | `websocket_zh.md` §4.2 |
| 设备 `audio_params` 决定下行采样率/帧长（24k/60ms/opus） | ✅ 已实现 | `ParseServerHello` |
| `abort`（打断）消息可被接收 | ✅ 能收，但**未贯穿全链路**（见 §3.4） | `websocket_zh.md` §4.1 |

### 0.2 已废弃 / 不再采用的技术方案（显式清单）

| 废弃项 | 出处/原因 | 替代 |
|---|---|---|
| **`iot` 消息类型** | 固件文档 §8.5「原有的 type:iot 方案已废弃」 | 一律走 `mcp`（JSON-RPC 2.0） |
| **`mcp` 占位回显** | 当前 `session.rs:157-166` 把设备发来的 `mcp` 原样 echo，未实现工具调用 | 真实 `tools/call` 闭环（§3.3） |
| **整段串行 LLM（`stream:false`）** | `llm.rs:52` 写死，等整段完成才出首字 | SSE 流式 + 按句 TTS（§3.1/§3.2） |
| **prompt 注入式 function calling**（让 LLM 用自然语言描述"调函数"） | 非标准、易错、难解析 | LLM 原生 `tools`/`tool_calls`（OpenAI 兼容标准） |
| **`chat/completions` 协议** | 用户指定升级：新式 Responses API（`instructions`+`input[]`+扁平 `tools`+事件式 SSE） | `/v1/responses`（已实现于 `llm.rs`） |
| **手写逐 token 轮询/多次请求拉 LLM** | 反模式、延迟高 | `reqwest::Response::bytes_stream()` 标准 SSE 解析 |
| **把 `llm.text` 当回复正文** | 最新协议 `llm` 消息仅用于 **emotion 表情同步**（`{"type":"llm","emotion":"happy","text":"😀"}`），回复正文走 `tts.sentence_start.text` | 正文统一经 `tts` 下发；`llm` 仅做表情/情绪 |
| **老的 `wake_word` 独立消息** | 最新固件改用 `listen` 的 `state:"detect"` + `text` 表示唤醒词已触发（`wake_word_detected`） | 识别 `listen.state=detect`（已在 `handle_text` 接收路径） |
| **`binary_protocol_version` 默认锁 v1** | 老默认会强制老协议；最新是 v3 | 改为**跟随设备 `hello.version`** + 配置默认值升到 **3**（最新推荐） |

### 0.3 已具备、可直接复用的现代基础（勿重复造）

- Opus 流式编码器复用（`audio/opus.rs`）、重采样 `Cow` 零拷贝（`audio/resample.rs`）。
- 重推理 `spawn_blocking` 隔离（`session.rs::blocking_result`）、`Session` 上下文结构化（RISKY 档）。
- 下行 `pad_tail` 残帧补零、`wrap_downlink`/`unwrap_uplink` 为唯一二进制帧出口（`protocol.rs`）。
- `Config::real_audio()` / `is_sherpa()` / `is_mock()` / `VadConfig::is_real()` 单源判定（RISKY 档）。

### 0.4 `client/`（Mac app）的定位

- 它是**软件版 ESP**——在桌面端复现 ESP 类设备约束（Opus 16k/60ms 上行、`features.mcp`、受限 CPU/带宽），用于**持续验证 server 行为与 ESP 一致**，避免每次改完都烧硬件。
- 不把它当独立产品优化；它的价值在于**重放 ESP 会话样本**并量测：TTFA、VAD 尾点、逐句下发正确性、abort 中断延迟、MCP 往返延迟。

### 0.5 协议版本策略（以最新 v3 为准，server 跟随设备）

固件对二进制协议的处理（`websocket_protocol.cc` / `.h`）铁律：**上行发送与下行解码严格使用同一个 `version_`；`version_` 默认 1，设备可配置为 2 或 3；`version_==2` 用 16 字节头（含时间戳，供 server AEC），`version_==3` 用 4 字节头（最新最简），其余（v1）裸 Opus 无头。** server 因此**没有单方面指定下行版本的自由**——只能被动跟随设备 `hello.version`。

| 版本 | 字节布局（下行/上行一致，小端） | 状态 | server 处理 |
|---|---|---|---|
| **v1**（默认但老） | 裸 Opus（`payload` 即 Opus 帧，无头） | 兼容兜底 | 直接发/收裸 Opus |
| **v2** | `u16 version \| u16 type \| u32 reserved \| u32 timestamp(ms) \| u32 payload_size \| payload` | 用于 server AEC（时间戳） | 已支持封装/解析 |
| **v3**（**最新推荐**） | `u8 type \| u8 reserved \| u16 payload_size \| payload` | 最新简化版 | 已支持封装/解析（`BinVersion::V3`） |

**落地规则**
1. **下行版本 = 设备 `hello.version`**（RISKY 档已修：`wrap_downlink` 用入参 `v`，不写死）。`ws.rs` 握手保存 `params.downlink_bin_ver = hello.version`；绝不回退到固定 v1。
2. **`[audio].binary_protocol_version` 配置默认值 1 → 3**：该字段仅作「设备未协商时的兜底层」；设备已带 version 时以设备为准。把默认推向 v3 表达「尽量用最新协议」的取向，但仍尊重设备主导权。
3. **v3 无 version 字段**：v3 帧靠握手协商识别，server 下行 v3 帧不要写 version 字节（已符合 `BinVersion::version_code` 仅用于 v1/v2）。
4. **不为「兼容老设备」保留 v1 特例代码**——v1 路径已通用（裸 Opus），无需额外分支；任何「若设备是 v1 则……」的特殊逻辑都属于过度设计。

> 协议单测已覆盖 v1/v2/v3 封装往返 + 下行 version 字节 == 设备 version（`protocol.rs`）。

---

## 1. 目标与指标体系（对齐 ESP 性能）

ESP 类设备是受限端（MCU、opus 固定 60ms 帧、带宽有限），server 的所有延迟优化必须以「ESP 实测表现」为基准，而非桌面无约束环境。

| 指标 | 当前（估计） | 目标 | 量测方式 |
|---|---|---|---|
| **TTFA**（用户说完 → 设备出首字音频） | VAD(~0.3s)+ASR(~0.5–1s)+LLM整段(2–8s)+TTS整段(8–20s) ≈ **最坏 ~30s** | VAD+ASR+LLM首token(~0.3–1s)+首句TTS(~1–2s) ≈ **3–5s** | `client/` 重放 ESP 样本 + 日志打点 |
| 长回复总播放时长 | = 整段合成后才开始 | 与剩余生成**重叠** | 日志 `t_send` 相对 STT 间隔 |
| 打断延迟 | 仅在 TTS 下发时响应（滞后 1–数秒） | LLM 思考/TTS 合成任意阶段即停 | abort→设备停播间隔 |
| CPU 预算 | docker `cpus:3.5`；worker 2 + ASR 2 + TTS 4 + VAD 1 | 维持 ≤4 核，不超标 | 压测监控 |
| MCP 往返 | —（回显，无意义） | 工具调用闭环 < 网络 RTT + 设备执行 | `client/` 模拟 device result 延迟 |

> 数值为工程估计，非承诺；验证以 ESP 真机 + `client/` 重放双向比对为准。

---

## 2. 全流程延迟拆解（现状 vs 目标）

### 2.1 现状（整段串行，过期方案）

```
用户说 → VAD 切句 → ASR 整段识别 → STT 回包
                              └→ LLM 整段(stream:false) ── 等整段 ──→
                                                              └→ TTS 整段合成(8–20s) → 整段下行
```
**瓶颈**：`llm.rs:52` 的 `stream:false` 让首字必须等 LLM 整段；TTS 整段合成（注释实测 8–20s）是绝对主导。这是「说完话后要等很久才开口」的根因。

### 2.2 目标（流式 + 按句流水线 + MCP 工具闭环）

```
用户说 → VAD 切句 → ASR → STT 回包
                └→ LLM 流式(SSE) 读 token ──┬─ 文本 delta → 按句切分 → channel
                                            └─ tool_calls delta → 收集
                       （LLM 读流 与 合成/下行 流水线重叠）
        channel ──→ 逐句: spawn_blocking TTS → Opus 编码 → 逐帧下行（sentence_start 逐句下发）
        若遇 tool_calls: 转 mcp tools/call → 设备执行 → mcp result 回填 → 再流式一轮
```

- **TTFA 降幅**：从「整段」降到「VAD+ASR+LLM 首 token+首句 TTS」。
- **顺序保证**：消费端按 channel 顺序逐句合成播放（TTS 不能乱序），但 **LLM 网络读取在独立 task** 与「合成+下行」并行。
- **工具调用不阻塞音频**：文本流先下发，工具调用在流结束后以 `mcp` 闭环补充，结果回填后继续流式——见 §3.3。

---

## 3. 优化方案（核心，按阶段）

### 3.1 LLM 流式（SSE）+ 原生工具调用 —— `llm.rs` ✅ 已落地（OpenAI Responses 协议）

> **协议决策（用户 2026-10-02 指定）**：LLM 使用 **OpenAI Responses API**（`POST /v1/responses`），
> 不用老的 `chat/completions`。已实现于 `server/src/llm.rs`。

**已实现形态**
- 请求体：`model` + `instructions`（system prompt）+ `input[]`（多轮历史，`input_text`/`output_text` 结构化）+ 扁平 `tools[].{type,name,description,parameters}` + `stream`（`[llm].stream` 默认 `true`）。
- SSE 事件解析（`SseParser`，字节缓冲整行解码、跨块安全）：`response.output_text.delta`（文本增量）、`response.output_item.added`（注册 `function_call`：`call_id`/`name`，按 `output_index`）、`response.function_call_arguments.delta`（入参增量拼接）；`error`/`response.failed` 捕获上报。
- 非 SSE 回退：响应非 `text/event-stream` 或 `[llm].stream=false` 时整段解析 `output[]`（`message` 拼 `output_text`、`function_call` 收工具调用）。
- `chat_stream` 回调逐块回传 `LlmEvent::{Text,ToolCall,Done}`；`chat` 为聚合便捷入口（session 现走此路径，行为不变）。
- `MockLlm` 同步实现 `chat_stream`（单次 Text + Done）。
- 单测 9 项：SSE 多 delta 拼接、跨块切断（含多字节 UTF-8）、function_call 事件拼装、非 data 行/message 项忽略、error 事件、非流式解析、`build_input` 角色映射、mock 流。

### 3.2 逐句 TTS 流水线 —— `session.rs::stream_response` ✅ 已落地

**已实现形态（生产者/消费者）**
- **生产者 task**：`engines.llm.chat_stream`（回调返回 `false` 即取消——abort 时停止读 LLM 流）→ `SentenceSplitter` 按句切分 → `mpsc::unbounded_channel` 推句（文本量小无需反压）。
- **`SentenceSplitter`**：命中 `。！？；` 及 ASCII `! ? ; \n` 即出句（标点保留）；末尾 `flush` 残句；无标点超长按 50 字符兜底硬切（循环切块、对齐字符边界）。单测 7 项覆盖。
- **消费者（session task）**：`tts start`（一轮一次）→ 逐句 `spawn_blocking` 合成 → `send_sentence_audio`（`sentence_start(text)` + 重采样/Opus 分帧逐帧下行 + 逐帧轮询打断）→ `tts stop`。
- **TTFA 打点**：首句下发完成日志 `TTFA≈xxms`；本轮完成日志含 LLM 流耗时/字数/工具调用数/句数。
- **历史压入**：仅未中断且流正常结束时压入 `history`，避免脏历史。
- **`tts_test` 路径**：`send_tts_audio` 重构为 `send_tts_start`/`send_sentence_audio`/`send_tts_stop` 三段复用，测试台与设备流水线同一下行实现。

> `take_sentence` 已知 case：英文缩写 `Mr./Dr.` 误切——第一版接受，后续可加「标点后空格+小写则续」启发式，不阻塞。

### 3.3 MCP 工具调用闭环（取代回显 / 取代 iot）—— `session.rs` + `llm.rs`

**这是「该用 MCP 就用 MCP」的落点**：设备是工具服务器，server 是 LLM 编排者 + MCP 客户端。

> 固件 `GetHelloMessage()` **硬编码 `features.mcp: true`**（不再发 `iot`，`iot` 官方已废弃）——即所有最新固件设备都声明支持 MCP。server 读取 `features.mcp` 决定是否在 LLM 请求中带 `tools`；若设备未声明（极老固件），则 LLM 纯文本向后兼容。

**协议语义（固件确认）**
- Server→Device：`{"type":"mcp","payload":{"jsonrpc":"2.0","method":"tools/call","params":{"name":"self.light.set_rgb","arguments":{...}},"id":N}}`
- Device→Server：`{"type":"mcp","payload":{"jsonrpc":"2.0","id":N,"result":{"content":[{"type":"text","text":"true"}],"isError":false}}}`

**闭环设计（关键）**
1. LLM 流式结束后若收集到 `tool_calls`：
   - 将每个 `tool_call.function.{name,arguments}` 映射为 `mcp` `tools/call`（`name`/`arguments` 透传，自增 `id`）。
   - 发送 `ServerMessage::Mcp{ payload }`；在 `Session` 登记 `pending_mcp: Mutex<HashMap<id, oneshot::Sender<Value>>>`。
   - **设备结果异步到达**：主 `run()` 循环收到 `ClientMessage::Mcp` 且 `id` 匹配 → 解析 `result`/`isError` → 通过 oneshot 唤醒等待中的工具调用 task（**注意：设备结果是 `run` 循环收的，不在 LLM task 内**，必须用请求/响应关联，不能用内联 await）。
2. 工具结果作为 `role:"tool"` 消息回填 `history`，**再进入一轮 `chat_stream`**（可能继续产出文本/更多 tool_calls，直到无 tool_calls）。
3. 文本流在每轮都按 §3.2 逐句下发，工具轮之间音频连续。

**废弃处理**：删除 `session.rs:157-166` 的 `mcp` 回显；`iot` 消息类型不新增（固件已废弃）。

**边界**：`features.mcp` 为 `false` 的设备 → 不启用 `tools`，LLM 纯文本（向后兼容）。

### 3.4 打断 / 保活 / 健壮性（贯穿全链路）✅ 已落地（除设备头记录）

- **abort 贯穿 ✅**：`Session.abort: Arc<AtomicBool>`。
  - `handle_text` 收 `Abort` 即置位；`poll_abort()` 非阻塞抽干 socket 中的 abort/关闭消息（播报期到达的上行音频丢弃——ESP barge-in 先发 abort 再 listen）。
  - LLM 读流：`chat_stream` 回调返回 `false` 即停止读流（返回已累积文本，不算错误）。
  - TTS：合成前后、逐帧下行均轮询 → 立即停；打断时仍发 `tts stop`（设备停播清队列）。
  - 一轮对话结束（`stream_response` 开头）重置。
- **保活 ✅**：stt 后早发 `llm` 状态（刷新设备 `last_incoming_time_`，防 ESP `IsTimeout()` 断连）；LLM 等待期每 10s 发 WS `Ping`（等待句间隙 200ms 粒度轮询打断 + 超时触发 ping）。
- **设备头可观测性 ⏳ 待办**：`ws.rs` 握手记录 `Device-Id`/`Client-Id`；`Protocol-Version` 与 `hello.version` 不一致 `warn`（随阶段三一并处理）。

---

## 4. 实施分期（重排：兼容已是前提，故无「兼容硬修复」独立阶段）

1. **阶段一 · LLM 流式 + 原生工具调用** ✅ 已完成（2026-10-02）
   - `llm.rs`：OpenAI **Responses 协议** `chat_stream`（SSE + 非 SSE 回退）+ 扁平 `tools`/`function_call`；`MockLlm` 同步；reqwest 启用 `stream` 特性。
   - 配置 `[llm].stream`（默认 `true`）；`api_base` 默认指向 `/v1/responses`。
   - 单元：9 项 llm 测试（SSE 增量/跨块/工具拼装/错误捕获/非流式/输入映射）。
2. **阶段二 · 逐句 TTS 流水线 + abort 贯穿** ✅ 已完成（2026-10-02）
   - `session.rs::stream_response` 改写：生产者（`chat_stream` 取消回调 + `SentenceSplitter` + unbounded channel）/ 消费者（逐句 spawn_blocking TTS + 逐帧下行）。
   - `Session.abort` 贯穿 LLM 流与 TTS（`poll_abort` 非阻塞轮询）；stt 后早发 `llm` 状态 + LLM 等待期 10s WS Ping。
   - `send_tts_audio` 拆分为 `send_tts_start`/`send_sentence_audio`/`send_tts_stop`，测试台同路径复用。
   - 单元：`SentenceSplitter` 7 项 + `find_sentence_end`。待办尾项：ws.rs 设备头记录（随阶段三）。
3. **阶段三 · MCP 设备工具闭环**
   - 删除 `mcp` 回显；`pending_mcp` 关联 + `tools/call` 往返 + 结果回填再流式。
   - 单元：MCP 往返关联（mock device result）。
4. **阶段四 · `client/` 作为 ESP 性能对齐验证台**
   - 重放 ESP 会话样本（Opus 16k/60ms 上行），量测 TTFA/吞吐/打断延迟。
   - 断言 server 行为对齐 ESP：`sentence_start` 逐句、版本一致下行、abort 即时、MCP 往返。
   - 与真机实测双向比对，固化为回归基准。

> 阶段间、`mock` 与 `sherpa` 两路径都要跑通（`cargo check --features sherpa` + 默认 feature + `cargo test`）。

---

## 5. 验证计划（以 ESP 对齐为核心）

| 层级 | 方法 | 断言 |
|---|---|---|
| 协议单测 | `cargo test` | v1/v2/v3 二进制封装往返（已含）；下行 version 字节 == 设备 version（已含） |
| 流式 | `llm.rs` 单测 | SSE 多 `data:` 行拼接正确；非 SSE 回退不崩；`tool_calls` 增量拼接正确 |
| 逐句 | `session` 单测 + `client/` | 设备视角：`stt` → **多个** `tts sentence_start`（按句）+ 二进制帧 + `tts stop` |
| 打断 | `client/` 模拟 mid-TTS abort | 设备停播延迟 ≪ 整段；abort 在 LLM 思考期也能停 |
| MCP | `session` 单测（mock device result） | `tools/call` 发出 → 关联 `id` 收 `result` → 回填后再流式一轮 |
| ESP 对齐 | `client/` 重放样本 + 真机 | TTFA/吞吐/打断与真机双向比对，固化回归基准 |

---

## 6. 配置变更汇总

| 配置 | 变更 |
|---|---|
| `[llm].stream` | **新增**（默认 `true`）；`false` 退回整段（兼容不支持 SSE 的端点） |
| `[llm].tools`（或设备 `features.mcp` 驱动） | **新增**：设备能力 schema 来源；`false`/不支持时 LLM 纯文本 |
| `[audio].binary_protocol_version` | **默认值 1 → 3**（最新推荐）；实际下行版本以设备 `hello.version` 为准，此字段仅作设备未协商时的兜底 |
| `[vad].min_silence_duration` | 可选下调以更快切句（权衡误切），不改默认值 |

---

## 7. 风险与回滚

- **SSE 解析脆弱**：不同 LLM 提供商 SSE 格式/错误包体有差异 → 用「非 SSE 自动回退整段」+ `[llm].stream` 开关兜底，可一键关回。**风险：中低。**
- **按句切分误切（英文缩写）**：第一版接受，已知 case。**风险：低。**
- **MCP 请求/响应关联竞态**：`pending_mcp` 用 `Mutex<HashMap<id, oneshot>>`，配对即清除；id 自增避免碰撞；超时（设备未回）需设上限并降级为「工具失败」回填。**风险：中。**
- **abort 竞态**：共享 `AtomicBool` + channel 关闭 + 各阶段轮询，逻辑收敛。**风险：低。**
- **历史截断时机**：仅未中断且流正常结束压入，避免脏历史。**风险：低。**
- **回滚**：阶段一~四各自独立，出问题可单独 revert；提交由用户决定（不自动提交）。

---

## 8. 与现有文档的关系

- `architecture.md` / `protocol.rs` 的协议描述按本方案更新为「流式 + 按句 + MCP」形态。
- `iot` 相关描述（若存在）标注废弃、指引至 `mcp`。
- 其余现状描述（§0.3 流水线图）作为已具备基础保留。
