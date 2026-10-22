# Wyoming Provider 调研与评估（合并归档）

> 状态：**过期调研记录（2026-10-14），提案未采纳**。当前代码无 Wyoming 实现（`server/Cargo.toml` 无相关依赖、注册表无 wyoming 描述符、无 `[asr.wyoming]` / `[tts.wyoming]` 配置段）。本记录保留供未来重启该方向时参考，不代表当前能力；当前事实以源码、测试与[插件层维护](<../../server/src/plugins/AGENTS.md>)为准。
> 来源：两份同日期文档合并——PM 许清楚《Wyoming 协议事实卡 + xiaozhi-server-rust 接入需求清单》（协议与生态调研，637 行）+ 架构师 高见远《Wyoming Provider 可行性评估 + 接入点设计》（对照代码现状的评估，597 行）。两部分结论互补；其中对 PCM↔Opus 转码成本的判断相反，**以第二部分代码核实结论为准**（见 §5.1）。
> 阅读顺序：先读「结论速览」；协议字节布局、生态与竞品查第一部分；接入成本、代码影响与分期查第二部分。

---

## 结论速览（合并）

| 问题 | 结论 |
|---|---|
| 适用域 | 仅「Rust Server ↔ 远程推理服务」（Whisper / Piper 类流式 PCM 服务）；ESP32↔Server 段固件已锁定 xiaozhi 协议 + Opus，Wyoming 不适用也不需要 |
| 优先级 | **TTS 先行**（可 1:1 抄生产在跑的 `plugins/tts/aiui.rs` 同步网络 provider 范式），ASR 次之，WakeWord 无接入点 |
| 形态 | 同步 trait + 每次调用短连接起步；**不主动改 async、不抽独立 crate**；连接池与 async 化为二期决策门 |
| 转码成本 | **≈0**：引擎边界两侧本就是 PCM f32，`StreamingResampler` / `f32_to_i16_16k` 已存在（第二部分 §5.1 代码核实，修正第一部分 R2 的「高成本」假设） |
| 运维代价 | 上升：Rust + N 个 Python 推理容器的编排、健康检查、模型下载需外部负责（第一部分 R1） |
| 当前状态 | 未实施。若重启，从第二部分 §⑨ 未决问题与 §8.2 分期表入手 |

---

# 第一部分 · 协议与生态调研（PM 许清楚，2026-10-14）

> 本部分只做调研与需求定义，不含实现方案与 Rust 代码。

## 0. 调研结论摘要（5 行）

1. **协议本身简单到可以手写**：Wyoming 就是 `JSON行 + 可选data块 + 可选payload块`，**没有 4 字节长度前缀**，纯 `\n` 分隔JSON。Rust 侧用 `tokio::net::TcpStream` + `serde_json` 手写约 400–600 行即可，无需外部 crate。
2. **但有一个致命阻塞点**：Wyoming **只支持裸 PCM，完全不支持 Opus/压缩音频**（协议层无 `format` 字段，`AudioChunk` 只有 `rate/width/channels`）。而本仓库 ESP↔Server 链路全链路是 Opus，**必须引入 PCM↔Opus 转码**，且 `audiopus` 当前被 `#[cfg(feature = "sherpa")]`  gate 住（`default` feature 下不可用）。
3. **协议归属已变更且生态在收缩**：仓库已从 `rhasspy/wyoming` 转到 **Open Home Foundation Voice（`OHF-Voice/wyoming`）**，MIT，1.11.0，仍活跃；但**卫星端 `wyoming-satellite` 已归档**（2026-01），官方明确推荐迁移到 **ESPHome 协议**（Linux Voice Assistant）。Wyoming 正被定位为「服务端之间的内部协议」，而非端到端协议。
4. **对本次诉求的真实价值**：不在「ESP32 ↔ Server」这一段（那段 ESP32 固件已锁定 xiaozhi 协议，且是 Opus），而在「**Rust Server ↔ 远程推理服务**」这一段——这一段确实需要一个跨语言、跨进程的稳定契约，Wyoming 是不错的选择。
5. **建议**：P0 只做 **ASR + TTS 两个 Wyoming provider**（价值明确、风险可控），**WakeWord 抽象降级为 P2**（本仓库 `[wakeup]` 只是问候语插件，根本没有唤醒词检测能力，新建 trait 属于「为不存在的能力造抽象」）；独立 crate 亦降为 P2。

---

## 1. Wyoming 协议事实卡

### 1.0 归属与维护状态（用户前提需修正）

| 项 | 用户前提 | **实际核实结果** | 来源 |
|---|---|---|---|
| 主仓库 | `rhasspy/wyoming` | ✅ `rhasspy/wyoming` 会 **301 重定向到 `OHF-Voice/wyoming`**（Open Home Foundation Voice）。不是 `sywhb/wyoming`——该仓库**不存在** | [GitHub API](https://api.github.com/repos/rhasspy/wyoming)、[OHF-Voice](https://github.com/OHF-Voice) |
| 是否归档 | 「已归档到 sywhb/wyoming」 | ❌ **未归档**，`archived: false`，最后 push `2026-09-29`，MIT，404 stars，活跃维护 | [GitHub API](https://api.github.com/repos/rhasspy/wyoming) |
| 当前版本 | — | **1.11.0**（CHANGELOG 顶部）；Python 库，命名空间 `wyoming` | [CHANGELOG.md](https://raw.githubusercontent.com/OHF-Voice/wyoming/main/CHANGELOG.md) |
| 规范文件形态 | 「协议文档在 `wyoming/proto.yaml`」 | ❌ **不存在 `proto.yaml`**，也没有 `.proto`。**协议规范就是 README + Python 源码本身**（`asr.py`/`tts.py`/`wake.py`/`audio.py`/`info.py`）。仓库文件树无任何 schema 文件 | [文件树 API](https://api.github.com/repos/rhasspy/wyoming/git/trees/main?recursive=1) |
| examples 目录 | 「看看examples 目录里有几种语言」 | ❌ **不存在 `examples` 目录**。参考实现在 `tests/`（pytest）与 `wyoming/http/`（HTTP 网关） | 同上 |

> ⚠️ **PM 提醒**：用户描述中「`proto.yaml`」「examples 目录」「sywhb」三处均与实际不符。架构师若照此去`proto.yaml` 找schema 会扑空，请直接读 `wyoming/*.py` 的 dataclass 定义（本文档即按此提取）。

### 1.1 传输层与确切字节布局

**传输**：TCP（也支持 `unix://` 域节点与 `stdio://` 标准输入输出，见 `server.py::AsyncServer.from_uri`）。URI 形如 `tcp://0.0.0.0:10300`。
来源：[server.py](https://raw.githubusercontent.com/OHF-Voice/wyoming/main/wyoming/server.py)

**帧格式（关键，与用户假设不同）**：**没有 4 字节大端长度前缀**。确切布局为：

```
┌─────────────────────────────────────────────┐
│ ① JSON header 行（UTF-8，以单个 \n 结尾）    │   ← 用 readline() 读，直到 \n
│    必含: "type": <string>                    │
│    可选: "data": {...},                      │
│          "data_length": <int>,              │
│          "payload_length": <int>,           │
│          "version": <string>                │
├─────────────────────────────────────────────┤
│ ② additional data（可选，恰好 data_length B）│   ← 也是一行 JSON，会被 merge 进 data
├─────────────────────────────────────────────┤
│ ③ binary payload（可选，恰好 payload_length B）│  ← 通常是 PCM 裸字节
└─────────────────────────────────────────────┘
```

字节级细节（来源：[event.py](https://raw.githubusercontent.com/OHF-Voice/wyoming/main/wyoming/event.py)）：

- ① 用 `reader.readline()` 读到 `\n` 为止，`json.loads()` 解析。
- ② 若 `data_length > 0`：`readexactly(data_length)` 读出，**再 `json.loads()` 成 dict 并 `update()` 合并回 `data`**（即 header 的 `data` 与 additional data 是**深度合并**关系，不是二选一）。
- ③ 若 `payload_length > 0`：`readexactly(payload_length)` 读出即 payload。
- **写出方向（`async_write_event`）会自动填充**：`version`（库版本号字符串）、`data_length = len(json.dumps(data))`、`payload_length = len(payload)`。**实现者只需自己算这两个长度并按序写出即可。**
- ⚠️ **无长度前缀意味着**：实现必须严格按序读写，且 `data`/`payload` 内的原始字节绝不能包含会被误读为 header 的内容。Rust 侧 `BufReader::read_until(b'\n')` + `read_exact` 即可，不要用 `read_line()`（会丢二进制 payload 中的 `\n`）。

**安全模型（架构师需知）**：官方 README 明确 **Wyoming 无认证、无加密**，设计上假定可信内网，**不应直接暴露到公网**；可选的 `wyoming.http.*` HTTP 网关是**未认证的 Flask dev server**。来源：[README.md#Security](https://github.com/OHF-Voice/wyoming)、[SECURITY.md](https://raw.githubusercontent.com/OHF-Voice/wyoming/main/SECURITY.md)

### 1.2 握手

**握手流程（`info` 服务描述，required）**：

```
客户端 → describe          （无 data 的裸事件）
服务端 → info              （列出 asr/tts/handle/intent/wake/mic/snd/satellite 能力）
```

来源：[README.md#Event Flow](https://github.com/OHF-Voice/wyoming)、[info.py](https://raw.githubusercontent.com/OHF-Voice/wyoming/main/wyoming/info.py)

**可选的程序选择（1.10.0 新增）**：`select-program`（`{"name": "<program name>"}`），连接建立后、首个请求事件前发送，按名字锁定本次连接使用的实现；不发送则用 `info` 里该domain的第一个 program。**服务端对无法识别的事件必须静默丢弃**，因此对老服务端发送它是 no-op —— 这是一条很好的前向兼容设计。

**`info` 完整 schema**（来源：[info.py](https://raw.githubusercontent.com/OHF-Voice/wyoming/main/wyoming/info.py)）：

顶层 `Info.data`：

| 字段 | 类型 | 说明 |
|---|---|---|
| `asr` | `AsrProgram[]` | 语音识别服务列表 |
| `tts` | `TtsProgram[]` | 语音合成服务列表 |
| `handle` | `HandleProgram[]` | 意图处理 |
| `intent` | `IntentProgram[]` | 意图识别 |
| `wake` | `WakeProgram[]` | 唤醒词检测 |
| `mic` | `MicProgram[]` | 音频输入 |
| `snd` | `SndProgram[]` | 音频输出 |
| `satellite` | `Satellite?` | 卫星信息（可含 `area`/`has_vad`/`active_wake_words`/`max_active_wake_words`/`supports_trigger`） |

所有 `*Program` 均继承 `Artifact`，公共字段：

| 字段 | 类型 | 必填 | 说明 |
|---|---|---|---|
| `name` | string | ✅ | 服务/模型唯一 id |
| `attribution` | `{name: string, url: string}` | ✅ | 作者与来源 |
| `installed` | bool | ✅ | 是否已安装 |
| `description` | string? | — | 人类可读描述 |
| `version` | string? | — | 版本 |

domain 专有扩展：

- `AsrProgram`: + `models: AsrModel[]`（`AsrModel` = `Artifact` + `languages: string[]`）、`supports_transcript_streaming: bool = false`、`requires_external_vad: bool = true`、`prefers_auto_gain_enabled: bool = true`、`prefers_noise_reduction_enabled: bool = true`
- `TtsProgram`: + `voices: TtsVoice[]`（`TtsVoice` = `Artifact` + `languages: string[]` + `speakers?: TtsVoiceSpeaker[]`）、`supports_synthesize_streaming: bool = false`
- `WakeProgram`: + `models: WakeModel[]`（`WakeModel` = `Artifact` + `languages: string[]` + `phrase?: string`）

> 💡 **对本仓库的直接价值**：`info` 是**能力发现机制**。`WyomingAsrProvider` 启动时发一次 `describe`，即可自动得知远端支持哪些模型、哪些语言、是否支持流式转写——这比硬编码配置文件优雅得多，且天然适配 `select-program`。

### 1.3 音频约定（⚠️ 对本仓库最关键）

来源：[audio.py](https://raw.githubusercontent.com/OHF-Voice/wyoming/main/wyoming/audio.py)

**三个音频事件**：

| 事件 | data 字段 | payload |
|---|---|---|
| `audio-start` | `rate: int`(Hz)✅、`width: int`(字节/采样)✅、`channels: int`✅、`timestamp?: int`(ms) | 无 |
| `audio-chunk` | `rate`✅、`width`✅、`channels`✅、`timestamp?: int`(ms) | 裸 PCM ✅ |
| `audio-stop` | `timestamp?: int` | 无 |

**明确回答用户的三个问题**：

1. **是否要求 16-bit PCM？** 协议层面 `width` 是**任意字节数的采样宽度**（`width=2` 即 16-bit，`width=1` 即 8-bit）。但**生态事实标准是 `rate=16000, width=2, channels=1`**——openWakeWord / whisper / piper 全按此配置。`audioop` 转码器支持 1/2/4 字节宽度互转。**实现上应按 16k/16bit/mono 作为唯一目标格式。**
2. **是否支持 Opus 压缩音频？** ❌ **完全不支持。** 协议中**不存在** `format`/`codec`/`encoding` 字段；`AudioChunk` 的 docstring 明确写着 "Chunk of raw PCM audio"，`payload` 是 "raw PCM audio samples"。参考实现里唯一与压缩相关的 `webrtc-noise-gain` 是**独立配套项目**（C++），不属于协议本身。`AudioChunkConverter` 只能做 **rate/width/channels** 三者转换（重采样/位深/声道），**没有任何编解码能力**。
3. **是否支持 timestamp_start / timestamp_end？** ❌ 不存在。只有单个 `timestamp?: int`（毫秒，标记该 chunk 的时间位置）。

> 🔴 **PM 判断**：xiaozhi-server-rust 与 ESP32 之间是 **Opus**（`media/opus.rs`，`OpusFrameEncoder`，Voip 模式，有状态流式编码器），而 Wyoming 是 **PCM**。PM 据此推断接入 Wyoming 意味着必须新增一路 PCM↔Opus 转码（风险 R2）。**架构师对照源码后修正该判断：引擎边界两侧本来就是 PCM f32，转码成本≈0（见第二部分 §5.1）。**

### 1.4 ASR 服务消息集

来源：[asr.py](https://raw.githubusercontent.com/OHF-Voice/wyoming/main/wyoming/asr.py)

⚠️ 用户清单写的 `start`/`stop`/`result` **在真实协议中不存在**。实际消息集：

**客户端 → 服务端**

| 事件 | data 字段 |
|---|---|
| `transcribe` | `name?`:模型名、`language?`:语言、`context?`:object、`vad_sensitivity?`:`"default"｜"relaxed"｜"aggressive"`、`transcript_names?`: string[]（高权重热词）、`transcript_terms?`: string[]（低权重领域词） |

随后紧跟 `audio-start` + `audio-chunk`+ + `audio-stop`。

**服务端 → 客户端**

| 事件 | data 字段 | 说明 |
|---|---|---|
| `transcript` | `text: string`✅、`language?`、`context?` | **一次性最终结果** |
| `transcript-start` | `language?`、`context?` | 流式转写开始（1.7.0+） |
| `transcript-chunk` | `text: string`✅ | 流式中间结果 |
| `transcript-stop` | — | 流式转写结束 |

**是否支持流式 partial result？** ✅ 支持（1.7.0 引入），但**语义在 1.11.0 发生了破坏性变更**：

> CHANGELOG 1.11.0 原文：*"`transcript-chunk` now carries the transcript so far instead of a fragment to append — Each chunk replaces the previous one, so an ASR system can revise words it has already reported. Services advertising `supports_transcript_streaming` must send whole transcripts from this version on."*

即：**`transcript-chunk` 是「到目前为止的完整文本」，后一个替换前一个**（可纠错），**不是增量片段**。且需在 `info` 里通过 `supports_transcript_streaming: true` 声明能力。

> 💡 本仓库的 SenseVoice 是**离线整段识别**（`recognize(&[f32], u32) -> String`），恰好对应 Wyoming 的 **one-shot `transcribe` + `transcript`** 模式，**不需要 partial**。所以 `WyomingAsrProvider` 走 one-shot 路径即可，`supports_transcript_streaming=false`。

### 1.5 TTS 服务消息集

来源：[tts.py](https://raw.githubusercontent.com/OHF-Voice/wyoming/main/wyoming/tts.py)

⚠️ 用户清单写的 `state`（含 `voice`/`sampling_rate`/`num_channels`）和 `sample_offset` **在真实协议中都不存在**。

**模式 A：一次性（one-shot）**

| 方向 | 事件 | data 字段 |
|---|---|---|
| C→S | `synthesize` | `text: string`✅、`voice?`: `{name?,speaker?}` 或 `{language?}`、`text_format?`:`"text"｜"ssml"`、`context?` |
| S→C | `audio-start` → `audio-chunk`+ → `audio-stop` | 同 §1.3 |

**模式 B：流式（text streaming，1.7.0+）**

| 方向 | 事件 | data 字段 |
|---|---|---|
| C→S | `synthesize-start` | `voice?`、`text_format?`、`context?` |
| C→S | `synthesize-chunk` | `text: string`✅（LLM 流式吐字，边到边合成） |
| C→S | `synthesize-stop` | — |
| S→C | （`audio-start`/`audio-chunk`+/`audio-stop`，可在每个 `synthesize-chunk` 后立即来一轮） |
| S→C | `synthesize-stopped` | —（流式响应结束） |

**TTS 是流式分片还是一次性返回？** ✅ **两者都支持，由服务端的 `TtsProgram.supports_synthesize_streaming` 声明**。

**对 `wyoming-piper` 的实测源码核查**（这是用户 P0 风险点的关键）：来源：[wyoming-piper/handler.py](https://raw.githubusercontent.com/OHF-Voice/wyoming-piper/main/wyoming_piper/handler.py)、[__main__.py](https://raw.githubusercontent.com/OHF-Voice/wyoming-piper/main/wyoming_piper/__main__.py)

- 默认 `--samples-per-chunk 1024`（即每片 1024 采样点），合成的 WAV 被**切成 1024 采样的 `AudioChunk` 逐片 `await write_event()` 发出** —— **是分片流式发送**。
- 但**流式的粒度是「句子」而非「采样」**：内部用 `sentence_stream.SentenceBoundaryDetector`（SBD）在 `synthesize.text` 中检测句子边界，**攒够一句才合成这一句**，然后一次性把该句的音频切成 1024 采样片发出。
- 有 `--no-streaming` 开关可关闭句子级流式。
- `info` 中 `supports_synthesize_streaming=(not args.no_streaming)`。

> ✅ **结论：`wyoming-piper` 默认是「按句流式」，首句音频延迟 = 首句合成耗时，优于整段返回。** 这**不会**破坏 `DownlinkPacer` 的首片即出声假设，但会引入**「首句文本越长，首包延迟越大」**的新行为——LLM 吐出第一句若是一段长文本，首包延迟随之线性增长。这是需要在验收里量化的（见 P0-6）。

### 1.6 WakeWord 服务消息集

来源：[wake.py](https://raw.githubusercontent.com/OHF-Voice/wyoming/main/wyoming/wake.py)

⚠️ 用户清单写的 `detection` 字段 `topics`/`score`/`timestamp_start`/`timestamp_end` **都不存在**。

**客户端 → 服务端**

| 事件 | data 字段 |
|---|---|
| `detect` | `names?`: string[]（模型名列表，**None = 任意**）、`context?` |

随后 `audio-start` + `audio-chunk`+ + `audio-stop`。

**服务端 → 客户端**

| 事件 | data 字段 |
|---|---|
| `detection` | `name?`: 模型名、`timestamp?`: 检测所在 chunk 的毫秒时间戳、`speaker?`:说话人、`context?` |
| `not-detected` | `context?`（音频流结束仍未命中） |

**无 score/topics 字段** → **拿不到置信度分数**。若产品需要「唤醒置信度」做门限控制，Wyoming 协议层面给不了，需自行扩展（服务端额外字段）或改用其他方案。

### 1.7 错误处理

来源：[error.py](https://raw.githubusercontent.com/OHF-Voice/wyoming/main/wyoming/error.py)

极简，**只有两个字段**：

```json
{"type": "error", "data": {"text": "<人类可读错误信息>", "code": "<机器可读错误码，可选>"}}
```

`text` 必填，`code` 可选。**协议未定义任何标准 `code` 取值**（无枚举），需按实现自定义。
对比：1.10.2 CHANGELOG 提到「HTTP servers no longer return exception details to the client」——**HTTP 网关会吞掉异常细节**，排障时只能看到 `text`。

### 1.8 官方/第三方服务端实现现状

来源：[OHF-Voice org](https://api.github.com/orgs/OHF-Voice/repos?sort=updated)、[rhasspy repos](https://api.github.com/users/rhasspy/repos?sort=updated)

| 服务 | 仓库 | 语言 | 许可证 | 状态 | 最后 push | stars |
|---|---|---|---|---|---|---|
| 协议库 | [`OHF-Voice/wyoming`](https://github.com/OHF-Voice/wyoming) | Python | **MIT** | ✅ 活跃 | 2026-09-29 | 404 |
| TTS (Piper) | [`OHF-Voice/wyoming-piper`](https://github.com/OHF-Voice/wyoming-piper) | Python | **MIT** | ✅ 活跃 | 2026-09-11 | 216 |
| ASR (Faster Whisper) | [`OHF-Voice/wyoming-faster-whisper`](https://github.com/OHF-Voice/wyoming-faster-whisper) | Python | **MIT** | ✅ 活跃 | 2026-09-29 | 396 |
| ASR (Vosk) | [`rhasspy/wyoming-vosk`](https://github.com/rhasspy/wyoming-vosk) | Python | MIT | ✅ 活跃 | 2026-09-28 | 22 |
| WakeWord | [`rhasspy/wyoming-openwakeword`](https://github.com/rhasspy/wyoming-openwakeword) | Python | **Apache-2.0** | ✅ 未归档（更新较少） | 2025-10-30 | 207 |
| WakeWord (micro) | [`rhasspy/wyoming-microwakeword`](https://github.com/rhasspy/wyoming-microwakeword) | Python | Apache-2.0 | ✅ 未归档 | 2025-10-30 | 13 |
| ASR (whisper.cpp) | [`rhasspy/wyoming-whisper-cpp`](https://github.com/rhasspy/wyoming-whisper-cpp) | C++ | MIT | ❌ **已归档** | 2024-02-29 | 32 |
| 卫星端 | [`rhasspy/wyoming-satellite`](https://github.com/rhasspy/wyoming-satellite) | Python | MIT | ❌ **已归档 2026-01** | 2026-01-24 | 1241 |
| Porcupine1 / Snowboy / mic-external / snd-external / SDL2 / handle-external / OpenTTS | `rhasspy/wyoming-*` | — | MIT | ❌ **全部已归档** | 2023-11~ 2024-04 | 低 |
| Add-on 集合 | [`rhasspy/wyoming-addons`](https://github.com/rhasspy/wyoming-addons) | Dockerfile | MIT | ✅ 活跃 | 2026-08-06 | 149 |

**许可证结论：全部 MIT 或 Apache-2.0，商用友好，无 copyleft 风险。**（注：`OHF-Voice/piper1-gpl` 是 GPL-3.0，但那是 Piper 引擎本体，与 Wyoming 协议封装分属不同仓库；若直接分发 Piper 二进制需单独评估。）

**社区（非官方）sherpa-onnx Wyoming 服务**—— 对本仓库**最相关**：

| 项目 | 说明 | 来源 |
|---|---|---|
| `ptbsare/sherpa-onnx-tts-stt` | 通过 Wyoming 对接 HA，同时暴露 OpenAI 兼容 API；已集成 Kokoro TTS | [社区文章](https://bbs.hassbian.com/thread-28546-1-1.html) |
| `zaolin/wyoming-serpha-onnx` | Wyoming 协议 ASR+TTS，sherpa-onnx 后端，**明确列出支持 SenseVoice** 与 Kokoro/VITS/Matcha；用 `wyoming:` host/port 配置接入 HA | [GitHub](https://github.com/zaolin/wyoming-serpha-onnx) |
| `xiasi0/wyoming-sherpa-onnx` | Wyoming ASR server，Qwen3-ASR + GTCRN 降噪 + 说话人门控 | [HA 社区](https://community.home-assistant.io/t/make-your-assistant-listen-only-to-your-voice/1003960) |
| `luuquangvu/wyoming-vietnamese` | 单容器同时提供 STT+TTS，sherpa-onnx 后端 | [开源榜](https://kaiyuanbang.cn/zh-cn/repo/luuquangvu-wyoming-vietnamese.html) |

> 💡 **战略含义**：已有多个「sherpa-onnx + Wyoming」的社区服务。若未来要把本仓库的 SenseVoice/Kokoro 剥离成独立服务供多客户端复用，**Wyoming 是现成已被验证的契约**，无需自创协议。这强化了 §5 场景 B 的价值。

### 1.9 Rust 生态现状

**crates.io 上确实已有一个 `wyoming` crate，但它基本没用**（来源：[crates.io/crates/wyoming](https://crates.io/crates/wyoming)、[zzjakub/wyoming](https://github.com/zzjakub/wyoming)）：

| 项 | 实测值 |
|---|---|
| 全名 / 版本 | `wyoming` 0.2.1（0.1.0 → 0.2.0 → 0.2.1，全部发布于 2025-01-02~ 01-04） |
| 作者 | `zzjakub`（单人） |
| 许可证 | Apache-2.0 |
| **代码量** | **148 SLoC / 9.29 KiB**（lib.rs 0.1.0 仅 12 SLoC） |
| **依赖** | `bigdecimal ^0.4`、`serde ^1.0`、`serde_json ^1.0`、`thiserror ^2.0` —— **无 tokio、无任何网络库、无 feature** |
| 定位 | 作者自述 **"Sans I/O style"**、**"HTTP client independent"** |
| 下载量 | 累计 2459，近 90 天 11 —— 几乎无人使用 |
| 最后提交 | 2025-01-07（**近 2 年未更新**） |
| stars | 2 |

**结论**：
- ❌ **没有可用的 Rust TCP 客户端/服务端实现**。该 crate 只是一个 Sans-IO 的消息类型定义，**连接管理、重连、超时、`readexactly` 全都没有**。
- ⚠️ 依赖里的 `bigdecimal` 用于表达采样率/时间戳这类浮点语义，**对本仓库是纯负担**（我们的采样率是 `u32`，时间戳是 `i64` 毫秒）。
- ✅ **但它证明了「协议类型层用 serde 表达」是可行的**，可作为消息类型定义的**参考起点**（尤其字段命名与 optional 处理），而非直接依赖。

**现有 Rust 实现者的选项**：**纯手写 `tokio::net::TcpStream` + `BufReader` + `serde_json::Value`（或自定义 derive）**。协议本身没有任何需要代码生成的复杂度（无 protobuf、无 IDL），手写成本可控。唯一可参考的第三方实现是 Python 官方库本身。

### 1.10 Home Assistant 集成与生态成熟度

**证据链**（来源：[HA 官方文档](https://www.home-assistant.io/voice_control/voice_remote_local_assistant/)、[OHF-Voice](https://github.com/OHF-Voice)）：

1. HA 有一个**内置集成**名为 "Wyoming Protocol"，可在「设置 → 设备与服务 → 添加集成」中直接添加，也可通过 zeroconf/mDNS 自动发现（`wyoming/zeroconf.py` 实现了 `HomeAssistantZeroconf`）。
2. HA 官方 **App Store（ formerly add-on）** 提供 Speech-to-Phrase、Whisper、Piper 等 Wyoming 服务安装，Assist 语音管线通过「语音助手」配置页绑定 STT / TTS / 唤醒词引擎。
3. **`wyoming` Python 库是 Home Assistant 的正式依赖** —— 有 Debian 打包记录（`python-wyoming 1.5.4-1`，由 Home Assistant Team 维护，见 [Debian bug #1077152](https://bugs.debian.org/cgi-bin/bugreport.cgi?bug=1077152)）。**这意味着协议稳定性有组织级背书。**
4. `linux-voice-assistant`（655 stars, Apache-2.0, 活跃）是官方推荐的**卫星端替代方案**，但它用的是 **ESPHome 协议**而非 Wyoming。

**⚠️ 生态方向的关键风险（PM 必须在决策时知晓）**：

`wyoming-satellite` 已于 2026-01归档，其 README 的弃用声明原文：

> **NOTE: This project is no longer maintained as it has been replaced by Linux Voice Assistant that uses the ESPHome protocol, which supports the newest features (e.g., media player, stop wake word, start/continue conversation, and timers)**
> （来源：[wyoming-satellite](https://github.com/rhasspy/wyoming-satellite)）

维护者 synesthesiam 在 [rhasspy 社区](https://community.rhasspy.org/t/2026-the-real-future-of-rhasspy-the-end/5872/1) 与 [linux-voice-assistant#219](https://github.com/OHF-Voice/linux-voice-assistant/discussions/219) 中的表态：

> *"personally I do not think that makes sense to pit continued development efforts on the old Wyoming satellite framework if would mean developers would spend time on maintaining a dead-end solution instead of spending time on what will hopefully become a mainstream solution in the future."*
>
> *"Wyoming protocol and the voice subset of the ESPHome protocol are doing a lot of what Rhasspy did... I'm doing my best to ensure that the various Wyoming services are available as regular Docker images outside of Home Assistant so that Docker compose can be used to recreate what the Rhasspy web UI used to do."*

同讨论中他还指出一个**现存的协议缺口**：Wyoming 的 `snd` 能力「only ever built for short voice-interaction audio (wake/done/timer sounds, TTS replies), not sustained music playback; that's a real protocol gap」。

**PM 解读**：
- ✅ **服务端侧（ASR/TTS/WakeWord 服务）Wyoming 仍然活跃且被官方 Docker 化推进**——这正是本仓库要用的方向。
- ❌ **端到端/卫星侧 Wyoming 已被官方判定为 dead-end**，ESPHome 协议（TCP over ESPHome API，支持 Opus、可加密、自带媒体播放器能力）才是方向。
- 📌 **因此：把 Wyoming 用于「Rust Server ↔ 推理服务」是顺势而为；把 Wyoming 用于「ESP32 ↔ Rust Server」是逆流而行**（且技术上不成立，因为 ESP32 固件已锁定 xiaozhi 协议 + Opus）。用户诉求恰好只涉及前者，**方向正确**。

---

## 2. 竞品/替代方案对比

### 2.1 对比总表

| 方案 | 跨语言统一程度 | 流式能力 | 音频格式约束 | 部署复杂度 | N5105 资源占用 | 生态成熟度 | 许可证风险 |
|---|---|---|---|---|---|---|---|
| **Wyoming** | ★★★★★ 纯 JSON，任何语言 20 行即可实现；官方 Python + 4 个 Rust 社区服务已验证 | ★★★★ ASR 有 partial（1.7.0+，1.11.0 语义变更）；TTS 有 text-streaming + 分片音频 | ❌ **仅 PCM**，不支持 Opus；`rate/width/channels` 三元组 | ★★★★☆ 需另起进程/容器（Docker Compose 编排）；**无认证** | ★★★★★ 服务可独立部署到别的机器；本机只需一次 TCP | ★★★★ MIT/Apache；HA 官方依赖 + Debian 打包；但**卫星侧已归档** | ✅ 无（MIT/Apache-2.0） |
| **OpenAI Realtime API / WebRTC** | ★★ 绑定 OpenAI SDK（官方 Python/Node.js） | ★★★★★ 端到端多模态，最低延迟 | ★★★★ 原生 WebRTC 音频 + g711/opus | ★★ 纯 API，零部署 | ★★★★★ 本地几乎不占资源（云端算力） | ★★★★★ 商业级 | ❌ **闭源服务 + $0.06–0.12/min**，成本与合规风险 |
| **LiveKit Agents** | ★★★ Python 插件生态，15+ ASR/TTS | ★★★★ 内置 VAD、语义断句 | ★★★★★ WebRTC 音轨（天然压缩） | ★★★ 需部署 LiveKit SFU + TURN/STUN | ★★★★ 本地需跑 SFU，**N5105 吃力** | ★★★★ Apache-2.0 | ⚠️ 部分绑定 LiveKit 基础设施 |
| **Pipecat** | ★★ **Python-only**，Rust 服务端无法用 | ★★★★★ 60+ provider，管道式流式 | ★★★★★ 传输无关（Daily/Twilio/WebRTC/custom） | ★★★ Python 运行时 + 编排层 | ★★★★ 需Python 运行时 | ★★★★ BSD-2-Clause，60+ adapters | ✅ 无，但**引入 Python 栈**与本仓库纯 Rust 形态冲突 |
| **ESP32 内置（ESP-SR）** | ★★ 仅 ESP32（乐鑫芯片） | ★★★ 芯片内唤醒词 + 离线命令词 | ★★★ 芯片内固定格式 | ★★★★★ **固件内，无需服务端** | ★★★★★ 服务端零开销 | ★★★ 官方支持，但**只对 ESP32 有效**，且能力受限 | ⚠️ 乐鑫 SDK 许可（非 MIT） |
| **WebSocket + 自定义 JSON**（现状） | ★★ 每对端一套方言 | ★★★★★ 与现有链路一致 | ★★★★★ **原生 Opus，零转码** | ★★★★★ 已在跑 | ★★★★★ 现有开销 | ★★ 需自己维护，**无生态** | ✅ 无 |
| **gRPC** | ★★★★ 强跨语言 + 代码生成 | ★★★★ 双向流天然支持 | ★★★ 容器内可塞任意字节（但**规范上不描述音频语义**） | ★★★ 需 protoc 工具链 + .proto | ★★★ protobuf 序列化开销小，但 HTTP/2 长连接在低功耗 CPU 上有成本 | ★★★★ 通用但**非语音专用** | ✅ Apache-2.0 |
| **云厂商 Speech**（阿里/讯飞） | ★★★ SDK 成熟但各家方言 | ★★★★★ 各家自有流式协议 | ★★★ 格式各家不同（pcm/alaw/opus） | ★★ 零部署 | ★★★★★ 本地零开销 | ★★★★★ 商业级 | ❌ **按量计费 + 数据出境合规风险**（本仓库已有讯飞 TTS，可作对照） |
| **HA Assist pipeline 协议** | — | — | — | — | — | — | ❌ **不是对外协议**，是 HA 内部管线； Wyoming 才是它的对外接口 |
| **ESPHome voice 协议** | ★★★ YAML + ESPHome API | ★★★★★ 支持打断/媒体播放器/定时器（**Wyoming 缺失这些能力**） | ★★★★★ **TCP + 可加密 + Opus** | ★★★★ 固件 OTA 便利 | ★★★★ ESPHome 侧省电，**但服务端要实现 ESPHome API**（偏重） | ★★★★ 官方主推，HA 原生 | ⚠️ 面向 ESPHome 生态，**非通用推理服务契约** |

> 表中 ESPHome 行原表已含 ESP-SR，我合并展示两行信息；ESPHome 协议本身是「端侧协议」，其服务端侧对齐成本较高，且它是为 ESPHome 客户端 API 设计的，不是为「Rust 服务端连推理服务」设计的。

### 2.2 特别回答：有没有比 Wyoming 更适合的现成标准？

分两段回答，因为两段的答案**完全不同**：

#### 段A：ESP32 ↔ Rust Server 之间→ **没有，且不该换**

| 候选 | 是否比Wyoming 更合适 | 理由 |
|---|---|---|
| ESPHome voice 协议 | ❌ 对本场景**不可行** | ESP32 固件是 `xiaozhi-esp32`，已锁定小智 WebSocket 协议 + Opus 帧格式。改协议 = 重刷固件 + 重写客户端，与「服务端解耦」的目标自相矛盾。 |
| OpenAI Realtime / WebRTC | ❌ | 需要 ESP32 侧跑 WebRTC 协议栈，ESP32 内存吃紧（Voice PE 级硬件才有余量），且强绑定云。 |
| gRPC | ❌ | ESP32 上跑 gRPC/HTTP2 不现实。 |
| **结论** | **Wyoming 也不是合适选择** | **这一段的协议已经存在、已经在跑、且是固件约束的结果。不动。** |

#### 段 B：Rust Server ↔ 远程推理服务 之间 → **Wyoming 是当前最优选择之一**

评估「Rust Server 需要一个跨进程契约去调用 Whisper/Piper/FunASR/CosyVoice」时的候选排序：

| 排名 | 方案 | 评估 |
|---|---|---|
| 🥇 **1** | **Wyoming** | 协议极简（手写无成本）、语音语义内建（ASR/TTS/Wake 分域清晰）、PCM 生态成熟、已有 4 个 Rust 社区 sherpa-onnx 服务可对接、MIT 无风险、HA 组织级背书。**缺点：仅 PCM。** |
| 🥈 2 | **gRPC** | 跨语言与代码生成确实更强，但 **proto 定义音频语义要自己写**，且引入 protoc 工具链 + HTTP/2 长连接开销，对 N5105 上的低延迟小包场景偏重。**没有现成的语音语义生态。** |
| 🥉 3 | **OpenAI 兼容 HTTP** | 本仓库 LLM 侧已在用 OpenAI 兼容 Responses API，**ASR/TTS 侧也已有成熟的 OpenAI 兼容约定**（`POST /v1/audio/transcriptions`、`/v1/audio/speech`，multipart/form-data）。**这其实是最小改动方案**——多数推理服务（含本仓库已有的讯飞、本地 sherpa）都能加一层薄适配。**缺点：OpenAI 兼容接口通常不支持流式返回完整音频**，首包延迟较高；且各家实现的字段细节不统一。 |
| 4 | WebRTC | 能力最强（天然压缩 + 回声处理 + 打断语义），但对「服务端连服务端」是杀鸡用牛刀，依赖重。 |

> 📌 **PM 结论（供架构师参考，非决策）**：
> Wyoming 与「OpenAI 兼容 HTTP」是**互补而非竞争**关系——
> - **流式音频/首包延迟敏感** → Wyoming（分片 `audio-chunk`）
> - **实现最简单、生态最通用** → OpenAI 兼容 HTTP
>
> 用户诉求中「FunASR / CosyVoice 走 HTTP」这部分本来就该用 OpenAI 兼容/HTTP，**不需要 Wyoming**。Wyoming 的适用面精确地是「**要流式 PCM 音频的本地推理服务**」，即 Whisper/Piper 这一类。这与用户设想的拓扑**完全吻合**。

---

## 3. 需求清单（P0/P1/P2）

### 3.0 前置事实（架构师必须知道的两条硬约束）

以下两条是从本仓库源码实测得出的、**直接决定实现形态**的事实：

**约束①：`AsrEngine` / `TtsEngine` 都是同步 trait。** 来源：[plugins/asr/mod.rs](file:///Users/admin/Desktop/notip.com.cn/xiaozhi-server-rust/server/src/plugins/asr/mod.rs)、[plugins/tts/mod.rs](file:///Users/admin/Desktop/notip.com.cn/xiaozhi-server-rust/server/src/plugins/tts/mod.rs)

```rust
pub trait AsrEngine: Send + Sync {
    fn recognize(&self, samples: &[f32], sample_rate: u32) -> Result<String>;   // 同步！
}

pub type TtsChunkCallback = Box<dyn FnMut(u32, &[f32]) -> bool + Send>;
pub trait TtsEngine: Send + Sync {
    fn synthesize_stream(&self, text: &str, speed: f32, speaker: i32,
                         chunk_cb: TtsChunkCallback) -> Result<()>;          // 同步！
    fn output_sample_rate(&self) -> u32 { 24_000 }
}
```

**约束②：现有调用点已用 `spawn_blocking` 兜住同步阻塞。** 来源：[workflow/session/mod.rs:63-71](file:///Users/admin/Desktop/notip.com.cn/xiaozhi-server-rust/server/src/workflow/session/mod.rs)

`asr_recognize()` 已经把 `asr.recognize(&pcm, sr)` 包在 `tokio::task::spawn_blocking` 里（并处理了任务取消与 panic）。TTS 侧 `bench_api.rs:207` 同样用 `spawn_blocking` 跑流式合成。

> 🎯 **对架构师的直接含义**：**不需要把 `AsrEngine`/`TtsEngine` 改成 async。** 网络 IO 天然是异步的，硬塞进同步 trait 会迫使 `spawn_blocking` 里再套一层阻塞等待，反而浪费 blocking 线程池。有两条合理路线（请架构师评估）：
> - **路线 1（推荐，零侵入）**：在**同步 trait 内部**用 `tokio::runtime::Handle::current().block_on(...)` 或独立的内部 runtime 驱动连接。风险：在 async 上下文里 `block_on` 是明确禁止的（会 panic），**只能在 `spawn_blocking` 闭包内安全使用**——现有调用点恰好满足此条件。
> - **路线 2（更干净）**：新增 `WyomingAsrEngine` 内部持有 `Arc<TcpStream>` 双向通道，`recognize()` 只负责把 PCM 推入已建立的连接并读取响应，连接/重连/握手在 `build_asr()` 阶段（同步、非 async 上下文）完成。**握手与IO 分离**，是最干净的方案。

---

### 3.1 P0 — 必须有（本迭代交付）

#### P0-1 `WyomingAsrProvider`（ASR 接入 Wyoming）

| 项 | 内容 |
|---|---|
| **验收标准** | 配置 `[asr] engine = "wyoming"` + `[asr.wyoming] host/port` 后，服务器能从 Wyoming 服务端取得转写文本，语义与现有 SenseVoice 路径完全等价（同一句测试音频→ 同一文本） |
| **细节** | ① 新增 `engine = "wyoming"` 到 `ASR_ENGINES` 权威表；`[asr.wyoming]` 段含 `host`(默认 `127.0.0.1`)、`port`(默认 `10300`)、`model`(可选，对应 `transcribe.name`)、`language`(可选)、`connect_timeout_ms`(默认 5000)、`read_timeout_ms`(默认 30000) |
| | ② 握手：`build_asr()` 阶段完成 TCP 连接 → 发`describe` → 收`info` → 可选发 `select-program`；**任一步失败必须返回明确的 `Err`**，由`PluginHost` 诊断（复用 `registry::unknown_engine_hint` 同类文案路径） |
| | ③ 每次 `recognize()`：发 `transcribe`(name/language) → `audio-start` → 若干 `audio-chunk` → `audio-stop` → 读 `transcript` 取 `text`。**必须能识别 `error` 事件并转成本仓库 `anyhow::Error`**，不能静默返回空串 |
| | ④ **格式转换**：`recognize()` 收到的是 `&[f32]` + `sample_rate`，需转成 Wyoming 的 `i16` PCM（`width=2`），并按目标 `rate` 重采样。**若本地已是 16k 可跳过重采样；否则需引入采样率转换** |
| | ⑤ **不做 partial**：`info.supports_transcript_streaming=false` 时只等 `transcript`。若远端声明支持，可先只记录 capability 不实现（避免 1.11.0 语义变更踩坑） |
| **不做** | 不做 TCP 重连（首次连接失败即启动失败，交给 Docker `restart` 策略） |

#### P0-2 `WyomingTtsProvider`（TTS 接入 Wyoming）

| 项 | 内容 |
|---|---|
| **验收标准** | `[tts] engine = "wyoming"` 后，首句回复的**首包音频延迟不劣于**当前 Kokoro 路径（需给出实测数字，见 P0-6），且 `DownlinkPacer` 不出现新的队列溢出告警 |
| **细节** | ① 新增 `engine = "wyoming"` 到 `TTS_ENGINES`；`[tts.wyoming]` 段含 `host`、`port`(默认 `10400`)、`voice`、`connect_timeout_ms`、`read_timeout_ms` |
| | ②实现 `synthesize_stream`：**必须按分片回调**（`TtsChunkCallback(rate, &[f32]) -> bool`），不能收集完整段再返回。原因见 P0-6 |
| | ③ 回调返回 `false`（打断）时**立即停止读取并 return `Ok(())`**，与现有引擎语义一致；同时主动关闭或归还连接（**连接池设计见 P0-5**） |
| | ④ 收到 `error` 事件必须中断当前合成并返回 `Err`，不能当成 audio-chunk 继续 |
| | ⑤ `output_sample_rate()` **不能沿用默认 24000**：必须返回 `info` 里 TTS voice 的真实采样率（由 `audio-start.rate` 动态学到并缓存），否则下行重采样会错 |
| | ⑥ `speed` / `speaker` 参数：Wyoming 协议**无对应字段**。需明确映射策略：`speaker` → `voice.name`（可配置映射表）；`speed` → **不支持，需在配置校验期报错或警告** |

#### P0-3 转码层（PCM ↔ Opus）——**这是 P0 里最容易被低估的一条**

| 项 | 内容 |
|---|---|
| **验收标准** | `default` feature（无 `sherpa`）下可编译通过；`sherpa` feature 下转码正确（往返位误差在容差内） |
| **细节** | ① Wyoming 侧只吃 **PCM i16**，ESP32 侧只吃 **Opus**。新增 provider 后链路上出现两种格式，需要明确转码点位置（建议：**在下行 pacer 之前保持 PCM，只在最后一跳编码 Opus**，沿用现有 `OpusFrameEncoder`） |
| | ② ⚠️ **`audiopus` 当前是 `sherpa` feature 可选依赖**（`Cargo.toml`: `sherpa = ["dep:sherpa-onnx", "dep:audiopus"]`），而 `media/opus.rs` 全部类型都带 `#[cfg(feature = "sherpa")]`。**如果 Wyoming provider 在 `default` feature 下也要工作，就必须在 `default` 中启用 `audiopus`**，或接受「Wyoming provider 仅在 `--features sherpa` 下可用」并明确报错。**这是需要架构师决策的第一个岔路口。** |
| | ③ Wyoming TTS 输出可能是 **22050/24000/16000 Hz**，`audiopus` 仅支持 8k/12k/16k/24k/48k（见 `to_sample_rate`）。**22050 必须先重采样到 24k**，否则编码失败 |

> 架构师修正（第二部分 §5.1）：P0-3 的「新增一路转码」假设不成立——引擎边界两侧本就是 PCM，需要的是复用已有 `f32_to_i16_16k` / `StreamingResampler`，而非新建转码层。但 `audiopus` feature 门控问题（②）本身仍然存在。

#### P0-4 配置面与热切换语义

| 项 | 内容 |
|---|---|
| **验收标准** | 修改 `[asr.wyoming].host` 并 hot-reload 后，**下一个会话**使用新地址；当前会话不中断 |
| **细节** | ① 热切换语义对齐现有约定：ASR/TTS 段建议 **`HotReload::NextSession`**（新建连接有握手开销，不适合 RestartOnly 也不适合 hot） |
| | ② `runtime/mod.rs` 的**引擎缓存键必须包含 host:port**，否则改地址后会命中旧引擎缓存（这是极易漏的坑） |
| | ③ `canonical_engine()` 别名表加入 `wyoming`，并确认 `unknown_engine_hint` 对拼错的 `wyoming`（如 `woming`）有友好提示 |
| | ④ `signature()`（用于 `tts_cache` 缓存键）必须包含 Wyoming 的 host/port/voice——**否则切换 Wyoming 服务端后会命中旧引擎的缓存音频** |

#### P0-5 连接生命周期与并发

| 项 | 内容 |
|---|---|
| **验收标准** | N 个并发会话共用一个 `Arc<TtsEngine>` 时，无连接交叉污染 |
| **细节** | ① **并发冲突**：`AsrEngine`/`TtsEngine` 是 `Send + Sync` 的**共享单例**。但 Wyoming 是**请求/响应式协议，一条 TCP 连接同一时刻只能服务一个请求**。因此**必须有连接池或请求串行化** |
| | ② 现有 `[asr].max_concurrent`（默认 1）提供了天然串行化点——但那是**跨会话排队闸门**，不是连接保护。**建议：TTS 侧用 `tokio::sync::Mutex` 或 `semaphore` 保护连接，ASR 侧沿用 `max_concurrent` 闸门** |
| | ③ 需定义清楚：`build_asr()` 建连接失败 → 启动失败（fail-fast）；运行中断连 → 本次请求 `Err`，**不自动重连**（避免掩盖问题），由下一次请求重建或交由 Docker 重启 |

#### P0-6 延迟与 `DownlinkPacer` 回归验证（量化验收，不可省）

用户提出的核心担忧经源码核查**部分不成立，但换成了另一个问题**：

> **担忧**：「Wyoming Piper 是 stop 后整段返回，会破坏 `DownlinkPacer` 的首片即出声假设」

**核查结论：担忧不成立。** `wyoming-piper` 默认按句流式，`--samples-per-chunk 1024` 分片 `await write_event()`（源码见 [handler.py](https://raw.githubusercontent.com/OHF-Voice/wyoming-piper/main/wyoming_piper/handler.py)）。

> **真实风险（替换为）**：因为按**句子边界**触发合成，**首包延迟 = 首句文本的合成耗时**。LLM 流式输出时，若第一句是长句（如「让我查一下今天北京天气怎么样」），首包延迟将随首句长度线性增长。而 `DownlinkPacer`（`workflow/downlink`，已有 `lead_ms` 预充设计，默认 `lead_ms=240`，见 `tests.rs`）**假设音频是连续稳定到达的**——句子间可能出现**不规则空隙**，导致节拍器时间表频繁重排。

| 项 | 内容 |
|---|---|
| **验收标准** | 必须给出**实测数字**而非定性结论：① 首句 5字 / 15 字 / 30 字三种长度下的**首包延迟 p50/p95**；② 与当前 Kokoro 路径的对比；③ 10 分钟连续对话中 `PacerStats` 的 `vacuum`（真空期）告警次数 |
| **判定标准** | 若 p95 首包延迟劣化 > 200ms，**退回 one-shot `synthesize` 模式**（牺牲首包延迟换简单性），或要求远端 Piper 用更小的 `--samples-per-chunk` |

### 3.2 P1 — 应该有（次迭代）

| ID | 需求 | 验收标准 | 备注 |
|---|---|---|---|
| **P1-1** | `info` 能力发现与启动自检 | `build_asr()` / `build_tts()` 阶段读取 `info`，校验目标 domain 非空、模型存在；不匹配时启动即失败并打印远端实际能力 | 顺带解决「配错模型名」这类难查问题 |
| **P1-2** | `select-program` 支持 | 配置可指定 program 名；不指定时用第一个 | 前向安全（老服务端静默忽略） |
| **P1-3** | `transcript` 流式（partial）能力 | 当 `info.supports_transcript_streaming=true` 时，暴露 partial 结果给上层（**按 1.11.0 语义：整体替换，非追加**） | 当前 SenseVoice 是整段识别，**P0 不需要**；为将来换 Whisper 流式服务预留 |
| **P1-4** | `error` 事件完整映射 + 错误分类 | 远端 `error` 能区分「连接失败/ 超时 / 远端推理失败 / 协议不兼容」四类，错误信息带 host:port 上下文 | 只报裸文本无法排障 |
| **P1-5** | 与 `tts_cache` / `hot_audio` 的签名隔离 | 已在 P0-4④ 要求，此处单列为验收项：`signature()` 含连接标识，切换引擎后缓存不串味 | 静默的错音 bug，极难发现 |
| **P1-6** | 健康检查接入 `[plugins.health]` | 定期探测 Wyoming 服务可用性，纳入既有健康端点 | 现网已有 `plugins/health.rs` |
| **P1-7** | 可观测性 | 为 Wyoming provider 增加结构化日志/指标：连接建立耗时、首包延迟、错误计数，与现有 `observability/metrics` 对齐 | 没有指标无法运维 |

### 3.3 P2 — 可以有（暂不做）

| ID | 需求 | 为什么降到 P2 |
|---|---|---|
| **P2-1** | `WyomingWakeWordProvider` | 🚫 **本仓库目前根本没有唤醒词检测能力**——`src/plugins/wakeup/`（167 行）从模块文档看是「唤醒问候语」插件：`WakeConfig{enabled, greeting, persist}`，`WakeGreeting` 走 `speak_canned` 预制话术通道，**不做本地唤醒词计算**。唤醒词命中信号来自**固件主动上报**的 `{"type":"listen","state":"detect","text":"<唤醒词>"}`。为一个「不存在的能力」造 `WakeWordProvider` trait 属于**为想象中的需求造抽象**。若未来真要做，先回答：① 唤醒词在 ESP32 端还是Server 端检测？② 若在 Server 端检测，音频链路是 Opus，如何喂给 Wyoming（PCM-only）？ |
| **P2-2** | 抽出独立 `wyoming` crate | **收益远小于成本**。理由：① 现有 crates.io 的 `wyoming` crate 只有 148 SLoC、无网络层，我们 90% 的代码是 tokio TCP + serde 读写，这部分**抽出去也无法复用给别的项目**；② 独立 crate 需要处理 workspace/版本/发布/CI 全套，收益是「其他项目也能用」——但**目前只有一个消费者**；③ 与本仓库耦合点：必须依赖 `tokio`+`serde`，若将来引入 `sherpa` feature 会造成**污染**（协议库不该有推理依赖）。**建议：先在仓库内做`src/wyoming/` 模块（内部可复用、零发布成本），待出现第二个真实消费者再抽 crate。** 若架构师仍决定抽，务必**禁止**该 crate 依赖 `sherpa-onnx` / `audiopus`，只允许 `tokio`+`serde`+`thiserror`。 |
| **P2-3** | Wyoming 作为对外服务（Server → Wyoming 服务端） | 若未来要让别的客户端用 Wyoming 访问本仓库的 SenseVoice/Kokoro，是反向工作。参考 `zaolin/wyoming-serpha-onnx` 的做法，成本可控，但当前无需求。 |
| **P2-4** | Wyoming HTTP 网关模式 | 协议库自带 `wyoming.http.*`（Flask dev server，**未认证**），本仓库已有 `reqwest`，用 HTTP 调远端 Wyoming 服务更简单，但**首包延迟更高**。作为 P2 备选。 |
| **P2-5** | `LlmProvider` 抽象改造 | 用户未提改动。**建议不动**——LLM 侧已用 OpenAI 兼容 Responses API，本身就是跨语言标准，没有替换动机。**明确不纳入本需求。** |

---

## 4. 风险与盲点（PM 职责范围）

### 🔴 R1（高）——「Rust Server 完全不需要知道 Whisper/Piper 怎么运行」这个目标**只完成了一半**

Wyoming 解决的是**「怎么和它说话」**，完全没解决**「谁来把它跑起来」**。接入后Rust Server 仍然需要知道：

| 问题 | 现状 | 需要谁来解决 |
|---|---|---|
| 谁启动/重启 `wyoming-faster-whisper`？ | 未知 | **必须外部编排**：Docker Compose / systemd / k8s sidecar |
| 谁做健康检查与重启？ | 无 | Docker 的 `restart: unless-stopped` + `healthcheck`，**或** Rust 侧实现（P1-6） |
| 模型文件从哪来？ | 未知 | Wyoming 服务首次启动自动下载（社区服务普遍如此），**但需要数据卷 + 网络** |
| 端口冲突/多设备怎么办？ | 未知 | 静态端口（10300/10400），多实例需端口池 |
| 进程崩溃后的会话怎么办？ | 未知 | 当前请求 `Err` → 上层怎么处理？**需定义降级策略**（回落到本地 sherpa？还是直接报错给用户？） |

> 📌 **PM 结论**：**「解耦」的收益是「Rust 代码不写 Whisper 逻辑」，不是「运维变简单」**。实际上运维复杂度是**上升**的——从「一个 Rust 进程」变成「Rust + N 个 Python 推理容器」。用户可能低估了这一点。**建议在方案里明确写出部署拓扑与 compose 文件，否则 P0 做完了也无法落地。**

### 🔴 R2（高）——PCM ↔ Opus 转码是必须付出的代价，且 N5105 上有真实开销

> ⚠️ **本节为 PM 依协议事实的推断，已被第二部分 §5.1 的代码核实修正（结论：≈0 成本）。保留原文以记录推断与核实的差异。**

- Wyoming **只支持 PCM**（已核实：`audio-chunk` 无 `format` 字段，payload 是裸 PCM）。
- 本仓库 ESP32 链路**全链路 Opus**（`media/opus.rs` 有状态 `OpusFrameEncoder`，Voip 模式）。
- **代价**：
  1. 上行：ESP32 的 Opus → 本地解码为 f32（**已在做**，`OpusFrameEncoder` 的注释明确写了「上行：设备发来的 Opus 帧 → 解码为单声道 f32 PCM → VAD/ASR」）→ 现在需要**再编码回 i16 PCM** 发给 Wyoming。这一步是**纯浪费**：ESP32 发来的 Opus 本可以直接喂给 Wyoming 的 Whisper，只需在 Wyoming 服务端解一次。
  2. 下行：Wyoming 回 PCM → 需转 f32 → 编码 Opus。**这一步是必要的**，且现有链路已在做。
- **CPU 开销**（N5105，4×Jasper Lake @1.1GHz，无AVX2 单元）：
  - 上行**多出一次 Opus 编码**（16000Hz mono Voip，单帧 20ms）。Opus 编码是整数运算为主，**不依赖 AVX**，N5105 可承受；但这是**纯新增的、零收益的**开销。
  - **缓解方案（需架构师评估）**：若上行 Wyoming ASR 与本地 VAD 都存在，考虑**直接把 PCM 送VAD、Opus 送 Wyoming**（但 Wyoming 不吃 Opus，此路不通）；或**让 Whisper 服务端接受 Opus**（不可控）。
  - 实际影响：**每路会话新增约 1–3% 单核占用**（估算，需实测）。相比 sherpa 推理本身的消耗占比不高，但**在 `max_concurrent` 拉满时是雪崩因素**。
- **`audiopus` feature gate 问题**：`default = []` 下 `audiopus` 不编译，`media/opus.rs` 的全部类型都被 `#[cfg(feature = "sherpa")]` 屏蔽。**「 Wyoming provider 能否在 `default` feature 下工作」是一个必须先回答的问题。**

### 🟠 R3（中）——抽 crate 的成本 > 收益，且有 feature 污染风险

- 现有 crates.io `wyoming` crate（148 SLoC、无网络层、近 2 年未更新、累计 2459 下载）**证明了这个协议「不值得单独造轮子」**——因为真正的工作量在 tokio TCP 与连接管理，而这部分与业务强耦合。
- **污染风险**：若抽出的 crate 依赖了 `sherpa-onnx`（例如为了复用音频转换工具），则「协议库」会拖上「推理引擎」依赖树，任何使用者都要下载 sherpa-onnx 预编译库（本仓库 `.gitignore` 里有 `target/sherpa-onnx-prebuilt/`，说明预编译产物很大）。**必须在 crate 的 `Cargo.toml` 里用 `[dependencies]` 明确禁止。**
- **建议**：**P0 不抽 crate**。内部模块化即可（`src/wyoming/`），零发布成本。

### 🟠 R4（中）——`WakeWordProvider` 抽象可能是「为不存在的能力造抽象」

- 实测：`src/plugins/wakeup/`（167 行）是**唤醒问候语**插件（`WakeConfig{enabled, greeting, persist}` → `WakeGreeting` → `speak_canned`），**不做唤醒词检测**。
- 唤醒信号来自**固件上报**的 `listen/state=detect` 消息。
- **盲点**：用户可能误以为仓库里已有本地唤醒词实现。若真要在 Server 端做唤醒词检测，需先回答三个问题：① 检测放在 Server 端的意义是什么（ESP32 端已能检测）？② 音频是 Opus，Wyoming 只吃 PCM，如何喂？③ `detection` 没有 score 字段，做不了门限。
- **建议**：**P2，甚至考虑不做。**

### 🟡 R5（中）——`LlmProvider` 用户未提改动，但需要明确「不动」

用户诉求只涉及 ASR/TTS/WakeWord。`LlmProvider` 抽象已存在且走 OpenAI 兼容 Responses API——**这本身就是跨语言标准，没有替换动机。建议明确不动**，避免范围蔓延。若用户其实想让 LLM 也走 Wyoming，需澄清：Wyoming **没有 LLM domain**（只有 asr/tts/handle/intent/wake/mic/snd/satellite），协议层面不支持。

### 🟡 R6（中）——同步 trait × 异步网络的形态选择，是最容易埋坑的地方

- 现有 `AsrEngine::recognize(&self, ...)` 与 `TtsEngine::synthesize_stream(&self, ...)` 都是**同步**，`&self`（非 `&mut self`），实现可被 `Arc` 共享。
- 但 Wyoming 是网络请求/响应，**天然异步**。
- **风险**：在 `spawn_blocking` 闭包里调`Handle::block_on()` 虽然可行（现有调用点满足此条件），但**极易被后续重构破坏**（一旦有人在 async 上下文直接调 `recognize()`，tokio 会 panic）。
- **必须做的防御**：在 `WyomingAsrProvider` / `WyomingTtsProvider` 的文档注释里**显著标注「同步 trait，不得在 async 上下文直接调用，必须经 `spawn_blocking`」**，并考虑加一个 `debug_assert!` 检测是否在 tokio runtime 上下文中（`tokio::runtime::Handle::try_current()`）。

### 🟡 R7（低）——协议版本与前向兼容

- `describe`/`info` 流程是 **required**，但具体事件的**可选字段在持续演进**：1.7.0 加流式、1.9.0 加 `user-event`、1.10.0 加 `select-program`/`vad_sensitivity`/`text_format`、1.10.2 加 `connect_timeout`/`read_timeout`、**1.11.0 改了 `transcript-chunk` 语义（破坏性）**。
- **策略建议**：
  - 客户端**只发最小必要字段**，读到未知字段忽略（serde `deny_unknown_fields` **务必不要开**）。
  - 不要依赖 `info` 里「看起来一定有」的字段（很多 `*Program` 字段有默认值且旧服务端不填）。
  - 用 `info` 的 capability flag（`supports_transcript_streaming` / `supports_synthesize_streaming`）做能力协商，**而不是靠版本号判断**。

### 🟡 R8（低）——无认证带来的部署约束

Wyoming **设计上无认证无加密**，官方明确「不应暴露到公网」。Docker Compose 编排时**必须保证只在内部网络暴露**（不映射到 `0.0.0.0`，或至少限制在受信任的 compose network）。若用户想复用现有 xfyun 那类公网调用模式，需明确这是**不同的安全模型**。

---

## 5. 场景适配性判断（对用户两类拓扑的直接回答）

用户给出的拓扑：
> ESP32 →(WebSocket)→ xiaozhi-server-rust →(Wyoming)→ Whisper Server / Piper，HTTP → FunASR / CosyVoice

| 用户设想 | PM 评估 |
|---|---|
| **ESP32 →(WebSocket)→ Rust Server** | ✅ **保持现状不动**。固件约束（xiaozhi 协议 + Opus），与本次需求无关，Wyoming 在此**不适用也不需要**。 |
| **Rust Server →(Wyoming)→ Whisper Server / Piper** | ✅ **这是 Wyoming 的正确落点，建议做（即P0）**。协议语义匹配、已有 Rust 社区 sherpa-onnx 服务可对接、MIT 无风险。**唯一代价是 PCM 转码（R2，架构师已修正为≈0）。** |
| **Rust Server →(HTTP)→ FunASR / CosyVoice** | ✅ **保持现状（HTTP）不动**。FunASR 本身有原生流式 WebSocket/gRPC 接口，用 Wyoming 反而是倒退。**不需要纳入 Wyoming 需求。** |
| 「甚至考虑给xiaozhi-server-rust 做一个 `wyoming` crate」 | ⚠️ **建议 P2，先做内部模块**（R3）。 |
| 「这是收益最大的一个」 | ⚠️ **PM 保留意见**：收益集中在「Server ↔ 推理服务」这一段，而这一段**目前只有 2 个引擎（SenseVoice/Kokoro）**，抽象收益 = 2 → N 的期权。若 N 短期内不增长，收益有限；而**代价是 PCM 转码 + 运维复杂度上升（R1/R2）**。**若确定要接 Whisper/Piper 到多台机器（GPU 主机），则收益确实显著，认同「收益最大」的判断。**请架构师在方案里明确回答：**预期接入几个远端服务、部署在什么机器上？** |

---

## 6. 给架构师的核心问题清单

1. **【阻塞】** `Wyoming*Provider` 能否要求 `--features sherpa`？还是必须让 `audiopus` 进入 `default`？（R2）
2. **【阻塞】** 同步 trait × 网络 IO：选「握手在`build_*()` 完成 + IO 在 `spawn_blocking` 内阻塞」还是改 trait 为 async？（§3.0、R6）
3. **【重要】** TTS 连接并发模型：连接池（`Mutex` 串行化）还是每会话一条连接？（P0-5）
4. **【重要】** `speed` / `speaker` 到 Wyoming `voice.name` 的映射策略；`speed` 无对应字段怎么办？（P0-2⑥）
5. **【重要】** 转码点位置：全链路保持 PCM 到最后一跳，还是尽早编码 Opus？（P0-3）
6. **【需量化】** 首句 TTS 延迟实测验收门槛（建议：p95 劣化 ≤ 200ms）。（P0-6）
7. **【需产品决策】** Wyoming 服务不可用时的降级策略：回落到本地 sherpa，还是直接报错？（R1）
8. **【需产品决策】** 是否接受「wyoming crate 不在本轮抽」？（R3 / P2-2）

---

## 附录 A：完整来源清单

**协议权威来源**
-协议主仓库：https://github.com/OHF-Voice/wyoming （MIT，v1.11.0，最后 push 2026-09-29）
- 事件编解码：https://raw.githubusercontent.com/OHF-Voice/wyoming/main/wyoming/event.py
- ASR 消息：https://raw.githubusercontent.com/OHF-Voice/wyoming/main/wyoming/asr.py
- TTS 消息：https://raw.githubusercontent.com/OHF-Voice/wyoming/main/wyoming/tts.py
- WakeWord 消息：https://raw.githubusercontent.com/OHF-Voice/wyoming/main/wyoming/wake.py
- 音频格式：https://raw.githubusercontent.com/OHF-Voice/wyoming/main/wyoming/audio.py
- 能力描述：https://raw.githubusercontent.com/OHF-Voice/wyoming/main/wyoming/info.py
- 错误：https://raw.githubusercontent.com/OHF-Voice/wyoming/main/wyoming/error.py
- 服务端基类：https://raw.githubusercontent.com/OHF-Voice/wyoming/main/wyoming/server.py
- 变更记录：https://raw.githubusercontent.com/OHF-Voice/wyoming/main/CHANGELOG.md
- 安全模型：https://raw.githubusercontent.com/OHF-Voice/wyoming/main/SECURITY.md

**服务端实现**
- Piper（MIT，活跃）：https://github.com/OHF-Voice/wyoming-piper ／ 流式实现 https://raw.githubusercontent.com/OHF-Voice/wyoming-piper/main/wyoming_piper/handler.py
- Piper CLI（`--samples-per-chunk 1024` / `--no-streaming`）：https://raw.githubusercontent.com/OHF-Voice/wyoming-piper/main/wyoming_piper/__main__.py
- Faster Whisper（MIT，活跃）：https://github.com/OHF-Voice/wyoming-faster-whisper
- Vosk（MIT，活跃）：https://github.com/rhasspy/wyoming-vosk
- openWakeWord（Apache-2.0）：https://github.com/rhasspy/wyoming-openwakeword
- microWakeWord（Apache-2.0）：https://github.com/rhasspy/wyoming-microwakeword
- whisper.cpp（MIT，**已归档**）：https://github.com/rhasspy/wyoming-whisper-cpp
- Add-ons（MIT，活跃）：https://github.com/rhasspy/wyoming-addons
- **已归档**：https://github.com/rhasspy/wyoming-satellite （2026-01，含弃用声明）

**Rust 生态**
- crates.io `wyoming`：https://crates.io/crates/wyoming （0.2.1，148 SLoC，Apache-2.0，无网络层）
- 源码仓库：https://github.com/zzjakub/wyoming （单人，2 stars，最后 commit 2025-01-07）
- Rust 社区服务：https://github.com/zaolin/wyoming-serpha-onnx ／ https://github.com/xiasi0/wyoming-sherpa-onnx

**生态与方向**
- Open Home Foundation Voice 组织：https://github.com/OHF-Voice
- 维护者对 Wyoming vs ESPHome 的表态：https://github.com/OHF-Voice/linux-voice-assistant/discussions/219
- 「Wyoming 卫星端已被替代」社区讨论：https://community.rhasspy.org/t/2026-the-real-future-of-rhasspy-the-end/5872/1
- HA 本地语音助手文档：https://www.home-assistant.io/voice_control/voice_remote_local_assistant/
- Debian `python-wyoming` 打包（HA Team 维护）：https://bugs.debian.org/cgi-bin/bugreport.cgi?bug=1077152
- HA 社区 wyoming-satellite 弃用确认：https://tarahome.ai/blog/home-assistant-local-voice-assistant-no-cloud/

**竞品**
- ESPHome voice 组件（Wyoming 缺失能力的对照）：https://esphome.io/components/voice_assistant
- Pipecat / LiveKit / OpenAI Realtime 对比：https://brendanjameslynskey.github.io/Voice_02_Realtime_Frameworks ／ http://aiworkflowlab.dev/article/build-production-voice-ai-agents-pipecat-python-latency-barge-in

**本仓库（实测）**
- `server/Cargo.toml`（feature gate：`default = []`，`sherpa = ["dep:sherpa-onnx", "dep:audiopus"]`）
- `server/src/plugins/asr/mod.rs`（`AsrEngine` 同步 trait）
- `server/src/plugins/tts/mod.rs`（`TtsEngine` 同步 trait + `TtsChunkCallback`）
- `server/src/plugins/wakeup/mod.rs`（**唤醒问候语**插件，非唤醒词检测）
- `server/src/media/opus.rs`（Opus 编解码，全部 `#[cfg(feature = "sherpa")]`）
- `server/src/workflow/downlink/`（`DownlinkPacer`，`lead_ms` 默认 240）
- `server/src/workflow/session/mod.rs:63-71, 307-318`（`spawn_blocking` 包装同步推理）

---

# 第二部分 · 可行性与接入点评估（架构师 高见远，2026-10-14）

> 评估对象：为 `xiaozhi-server-rust` 增加 Wyoming Protocol 化的 ASR / TTS / WakeWord Provider。
> 定位：**可行性评估 + 接入点设计**，不是 PRD，也不是最终实现方案。
> 所有结论均带 `文件:行号` 证据；读不到的地方明确写「未验证」。
> 评估日期：2026-10-14；代码基线：`agents.md` 2026-10-13 版 / `docs/designs/gap-implementation-design.md`

---

## ① 结论（8 行内）

**部分可行，且 TTS 是收益/成本比最高的一档；ASR 可行但缺热切换；WakeWord 在本仓库无处可插。**

1. TTS 侧**已有远程网络 provider 的成熟先例**（`plugins/tts/aiui.rs`，同步阻塞 WebSocket + PCM 分片回调），Wyoming TTS 是它的近亲，改造成本可控。
2. ASR 侧 trait 极简（`asr/mod.rs:29-31` 单方法），但 **`Engines.asr` 是不可变字段（`runtime/mod.rs:47`），没有热切换路径**——Wyoming 换实例只能重启（与 SenseVoice 同档，可接受）。
3. **PCM↔Opus 转码成本几乎为零**：引擎边界两侧本来就是 PCM f32，Opus 只存在于 ESP 链路两端的 `media/opus.rs`，且**已有 `StreamingResampler` 与 `f32_to_i16_16k`**（`media/pcm.rs:8`）——这是本次评估最反直觉的结论。
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
| AIUI 全链路 | `plugins/aiui/mod.rs:136` 附近（`FullChainEngine`） | 有状态长连接，`Mutex` 供 `spawn_blocking` 互斥（`workflow/session/mod.rs:131-132`） | ❌ 同步 | `Send` | — |
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
| 唤醒信号来自 **ESP32 固件**，不是服务端音频 | `workflow/session/wakeup.rs:9-11`：固件发 `{"type":"listen","state":"detect","text":"<唤醒词>"}`（`Protocol::SendWakeWordDetected`，由固件 `CONFIG_SEND_WAKE_WORD_DATA=y` 启用） |
| 服务端**看不到唤醒前的音频** | `workflow/session/mod.rs:491-511`（`handle_binary`）：上行二进制帧 → Opus 解码 → VAD → ASR。设备只在唤醒后才上行音频 |
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
| 3. 热刷新决策 | `decide_refresh(cap, old_sig, fresh) -> Result<Option<String>, String>` | **`runtime/mod.rs:535-560`** | **纯函数**（可在无引擎/无 feature 的 `cargo test` 直接断言）：<br>· 未知 engine → `Err`（**不** `Ok(None)`，否则静默保留旧引擎）<br>· `validate` 失败 → `Err`<br>· `warn_if` 有值 → `Err`（能力受限，**不重建**）<br>· 签名相同 → `Ok(None)`<br>· 签名不同 → `Ok(Some(sig))` |
| 4. 重建 | `refresh_tts` / `refresh_llm` / `refresh_memory` | `runtime/mod.rs:375-434` / `439-458` / `328-372` | 签名变 → **经 `descriptor.build` 重建**（`runtime/mod.rs:390-404`，注释明确「新增供应商只要加 1 条描述符，本函数不再分发」）→ 成功后热替换 + 清缓存 + 闸门对账；失败**保留旧实例**并写 `Phase::Degraded` |
| 5. 触发点 | `Engines::refresh_from_disk()` | `runtime/mod.rs:293-321`，调用点 `workflow/session/start.rs:46` | **会话开始**时读盘 → `host.set_config` → 逐能力 `refresh_*`。⚠️ **只覆盖 Tts/Llm/Memory 三个能力**（`runtime/mod.rs:311-319`） |

**⚠️ ASR 没有热切换路径**：`Engines.asr` 是**不可变 pub 字段**（`runtime/mod.rs:47`，对比 `tts: RwLock<Arc<dyn TtsEngine>>` 在 `runtime/mod.rs:49`），`refresh_from_disk` 的三个 `refresh_*` 里没有 ASR。这与 `ASR_SENSEVOICE.hot = RestartOnly`（`impls.rs:314`）一致。**所以 `[asr].engine` 切换本来就要重启，Wyoming ASR 不会比 SenseVoice 更差。**

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
| `workflow/session/mod.rs` | **无需改** | `asr_recognize`（307-321）已是 `spawn_blocking` + 闸门，trait 形状不变 | |
| `runtime/mod.rs` | **无需改** | ASR 无热切换路径（§2.4），新 provider 同样只需重启 | |
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
| `runtime/mod.rs` | **修改（关键）** | ① `tts_for(lang)`（**224-262 行**）：现逻辑对非 kokoro 直接返回默认引擎（`runtime/mod.rs:229-233` 含 `cfg.tts.engine_id() == "xfyun"` 硬编码分支）——**wyoming 天然落入该分支，零改动即正确** ✅<br>② `refresh_tts`（375-434）**经 `descriptor.build` 分发，无需改动** ✅（`runtime/mod.rs:390-392` 注释明说）<br>③ `sig_tts_wyoming` 变化会触发重建 + `tts_cache.clear()`（`runtime/mod.rs:426`）+ 清语言池（`runtime/mod.rs:423`）——**语义正确，无需改** | 这是注册表统一动作模型的**直接收益**：`agents.md:136-138` 的「不再需要中心化 match 分发」在此得到验证 |
| `workflow/session/bench.rs` | **修改** | 测试台 `tts_test` 有硬编码分支 **`workflow/session/bench.rs:140`** `if disk_tts.engine_id() != "xfyun"`（后续 145-155 附近走 xfyun 专属的 vcn 处理）。wyoming 会落入 else 分支，需确认该分支是否正确处理非 xfyun 引擎 | ✅ 行号已核实：`bench.rs:140` |
| `device/voices.rs` | **修改（可选）** | 音色目录探测**强耦合 xfyun**：`voices.rs` 读 `engines.config.tts.xfyun.clone()`（3 处），`engine_id() != "xfyun"` 时返回未配置错误。wyoming 音色需**另建探测路径或直接不做目录** | grep 证据：`device/voices.rs` 中 `tts.xfyun` 出现于 clone/错误文案分支 |
| `plugins/health.rs` | **修改（可选）** | `probe_tts`（127-153）对非 kokoro 直接 `ProbeReport::ok()`（128-130）⇒ **wyoming 零改动即正确** ✅ | |
| `gateway/ws/config.rs` | **无需改（若无密钥）** | `SECRET_PATHS`（67-75）只在有密钥字段时才需加。Wyoming TTS 通常无密钥 ⇒ **零改动** ✅ | 对比：xfyun 加了 3 条（`api_key`/`api_secret`/`app_id`） |
| `gateway/ws/config.rs::rewrite_legacy_paths` | **无需改** | 只处理 `tts.backend`→`engine`、`LEGACY_BODY_FIELDS`→`[tts.kokoro]`、`memory.backend`（123-164）。wyoming 无旧写法 ⇒ 不涉及 | |
| `config/overlay.rs` | **修改（可选）** | 设备 overlay 白名单（`overlay.rs:27-31`）目前允许 `["tts","xfyun","voice"]`；若要 per-device 选 wyoming 音色，需加 `["tts","wyoming","voice"]` | |
| `workflow/pipeline/prefetch.rs` / `workflow/downlink*` / `workflow/session/mod.rs` | **无需改** | TTS 流式回调契约（`prefetch.rs:372-387`）不变 | |
| `client/.../ConfigFormState.kt` | **修改** | ① `ttsRemoteEngines.addAll`（**369 行**）加 `"wyoming" to "Wyoming (本地服务)"`<br>② `buildConfigJson` 的 `put("tts", …)`（**716-740 行**）加 `put("wyoming", …)` 子段<br>③ `ConfigCards.kt` TTS 卡字段区（xfyun 字段区旁） | |
| `config.example.toml` | **修改** | 加 `[tts.wyoming]` 注释样例 | |

**TTS 改动点合计：新增 1 文件 + 修改 6~8 文件**（比 ASR 少，因为 TTS 私有段/注册表模式已跑通两轮）。

### 3.3 WakeWord：**无接入点，不建议纳入本期**

| 文件 | 性质 | 说明 |
|---|---|---|
| — | **不适用** | 见 §2.2：`[wakeup]` 是话术插件非引擎；唤醒词识别在 ESP32 固件；服务端看不到唤醒前音频 |
| `plugins/wakeup/mod.rs` | **无需改** | 它不持有引擎、不参与音频识别 |
| 若强行要做，需新增 | — | ① 新 `Capability::WakeWord` 变体（触发 `registry/tests.rs:44-52` 的**变体数硬断言 11 → 12**，`ALL_CAPABILITIES` 也要加）<br>② 新 trait（每会话有状态，`&mut self`）<br>③ 新描述符 + `impls_of` 分支<br>④ **前置条件：ESP32 固件改为常开上行音频流**<br>⑤ 新增音频路由分支（唤醒前音频 → wakeword，回调命中后再走 VAD→ASR），会改动 `workflow/session/mod.rs:491-511` `handle_binary` 主路径 |

---

## ④ `agents.md` §5.2b「新增供应商只需改两处」在网络型 Provider 上是否成立

**结论：不成立，但差距是「2 处 → 8~10 处」，且差距全部落在「客户端 + 配置结构」而非「服务端分发逻辑」。**

`agents.md:149-150` 的原话：

> 新增/改名实现时必改的**只有两处**：`plugins/*/mod.rs` 的 `<CAP>_ENGINES` 表（含别名）与注册表描述符；`config/tests.rs::declared_engine_ids_match_registry_implementations` 会守住二者一致。

### 4.1 该论断在**哪些维度上成立**

| 维度 | 是否成立 | 证据 |
|---|---|---|
| 服务端引擎分发逻辑 | ✅ **完全成立** | `runtime/mod.rs:390-392`：`refresh_tts` 注释明说「构建入口**由注册表描述符提供**（新增供应商只要加 1 条描述符，本函数不再分发）」 |
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
| **⑥ 音色目录（仅 TTS）** | `device/voices.rs` 强耦合 xfyun 真实 API 探测 | ✅ 特有 | +1 套探测逻辑或明确不做 |
| **⑦ 测试台硬编码分支** | `workflow/session/bench.rs` 有 `engine_id() != "xfyun"` 分支 | ✅ 特有 | 需核对 |
| **⑧ 签名语义不同** | 现有 `signature` 是**纯字段字符串**（`impls.rs:67-134`），进程内比对。对短连接 provider 够用；对**长连接 + 连接池** provider，「签名变了→重建引擎」意味着**丢弃连接池**，语义不匹配（详见 §6） | ✅ **网络型特有，且是设计层面的** | 需重新定义「重建」语义 |

**判定**：§5.2b 的「两处」是在**「本地模型 provider + 已跑通两轮的 TTS 私有段模式 + 字段元数据驱动的客户端(P5) 未落地」**这三个隐含前提下的陈述。Wyoming ASR 会同时踩中 ①②④；Wyoming TTS 只需 ②⑥⑦，服务端侧确实接近「两处」。**网络型 Provider 的真正额外成本不在「加一个 provider」，而在「长连接生命周期」与现有「会话开始按签名重建」模型的语义错位（§6）。**

---

## ⑤ PCM ↔ Opus 转码链路分析

### 5.1 结论：**几乎零成本**（本评估最重要的反直觉发现）

引擎 trait 的两侧**本来就是 PCM**：

```
                    ┌───────────── 服务端内部全部是 f32 PCM ─────────────┐
ESP32 --Opus--> [OpusFrameDecoder] --> VAD --> AsrEngine --> Llm --> TtsEngine --> [StreamingResampler] --> [OpusFrameEncoder] --Opus--> ESP32
                  media/opus.rs    vad/     &[f32]              synthesize_stream    media/resample.rs    media/opus.rs
                  :112-118 (f32 out)          +sample_rate:u32      (u32, &[f32])        (sr→downlink_sr)       :53-66 (f32 in)
```

证据：
- `AsrEngine::recognize(&self, samples: &[f32], sample_rate: u32)`（`asr/mod.rs:30`）——**已经是 f32 + 显式采样率**。
- `TtsChunkCallback = Box<dyn FnMut(u32, &[f32]) -> bool + Send>`（`tts/mod.rs:311`）——**回调第一参数就是采样率**。
- `OpusFrameDecoder::decode_frame -> Vec<f32>`（`media/opus.rs:112-118`）——上行解码**直接产出 f32**。
- `OpusFrameEncoder::encode_frame(&mut self, samples: &[f32])`（`media/opus.rs:53-56`）——下行编码**直接吃 f32**（内部才转 i16，见 `opus.rs:55-60`）。
- `StreamingResampler::new(from_sr, to_sr)` + `push(&[f32]) -> Vec<f32>`（`media/resample.rs:27-38`）——**跨分片保持相位连续**，且**无 feature gate**（`grep cfg(feature` 在该文件零命中）。

**所以 Wyoming 的 PCM（16k/16bit/mono）需求**：

| 方向 | 需要的转换 | 现有资产 | 是否新增代码 |
|---|---|---|---|
| **ASR：ESP 上行 → Wyoming** | Opus → f32 → **i16**（若上游要 s16le） | `OpusFrameDecoder`（已）+ `f32_to_i16_16k(pcm, uplink_sr)`（`media/pcm.rs:8`，**已存在**，含非 16k 的线性插值重采样） | **零**（`f32_to_i16_16k` 现在的唯一调用方是 AIUI，见 `pcm.rs:5-6`；直接复用即可） |
| **TTS：Wyoming → ESP 下行** | i16 → f32 → 重采样到 downlink_sr(24k) → Opus | `i16→f32` 是 `opus.rs:117` 的 `s as f32 / 32768.0` 一行；`StreamingResampler`（已） | **零到少量**（在 Wyoming 客户端内部做 i16→f32，反向转换） |

⚠️ **唯一的真实约束**：`audio-chunk` 的 `width`/`rate`/`channels` 是 Wyoming header 里的**必填字段**（协议规定），所以 `WyomingAsr` 必须知道 `self.params.uplink_sr` 并在 header 里声明。上行采样率来自 ESP hello 协商（`SessionParams.uplink_sr`，`workflow/session/mod.rs:83`），**不是配置项** ⇒ 描述符的 `signature` **不应**包含采样率，而应在每次调用时从 `recognize` 的 `sample_rate` 参数取（`asr/mod.rs:30` 已经把它传进来了 ✅ —— **这个参数的存在是幸运的**）。

### 5.2 `audiopus` 的 feature 门控问题

| 事实 | 证据 |
|---|---|
| `audiopus` 是 `optional`，只在 `sherpa` feature 下引入 | `server/Cargo.toml`：`sherpa = ["dep:sherpa-onnx", "dep:audiopus"]` |
| `default = []` | 同上 |
| **`media/opus.rs` 整文件被 `#[cfg(feature = "sherpa")]` 门控**，非 sherpa 构建提供**返回空帧的占位实现** | `opus.rs:13`、`22`、`35`、`69-82`（encoder 占位）、`89`、`95`、`121-133`（decoder 占位） |
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
| `AsrEngine::recognize` | **1** | `workflow/session/mod.rs:318`（`asr_recognize` 内的 `spawn_blocking` 闭包） |
| `TtsEngine::synthesize_stream` | **5** | `workflow/session/bench.rs:217`、`workflow/pipeline/prefetch.rs:372`、`benchmark/mod.rs:224`、`plugins/tts/mod.rs:334`（trait 默认实现 `synthesize` 内部）、`plugins/tts/aiui.rs:154`（自检）+ 2 处测试 |
| `LlmProvider::chat_stream` | **4** | `workflow/session/producer.rs`、`workflow/session/bench.rs`、`plugins/llm/retry.rs:159`（装饰器）、`plugins/memory/graph.rs` + 2 处测试 |

**这是本评估最重要的有利条件**：ASR 只有 **1 个** 生产调用点，TTS 只有 **3 个**（不含 trait 默认实现与测试）。

### 6.2 三方案对比

| 维度 | (a) trait 内桥接同步<br>（`Handle::block_on` / crossbeam） | (b) trait 改 async | (c) 新增 async 专用 trait，与旧 trait 并存 |
|---|---|---|---|
| **做法** | trait 签名不变（同步）；Wyoming client 内部用 `tokio::runtime::Handle::current().block_on(...)` 或 `crossbeam::channel` 把 async 网络 IO 同步化 | `AsrEngine::recognize` / `TtsEngine::synthesize_stream` 改成 `Pin<Box<dyn Future>>`（抄 `MemoryProvider` 的 `BoxFut`，`memory/mod.rs:61`） | 新增 `AsyncAsrEngine` / `AsyncTtsEngine`，注册表 `Built` 枚举加变体，`PluginHost`/`Engines` 加并行分支 |
| **改动面（服务端）** | **0**（调用点全不动） | ASR：1 个 trait 定义 + 1 个调用点（`session.rs:307-321`）+ 2 个实现（sensevoice/wyoming）<br>TTS：trait + 3 个调用点 + 3 个实现 + `TtsEngine::synthesize` 默认实现（`tts/mod.rs:329-347`）<br>连带：`workflow/pipeline/prefetch.rs` 的 `spawn_blocking` 包装（369-390）要拆、`workflow/pipeline/inference.rs` 闸门的 `spawn_blocking` 假设（`inference.rs:1-16`）要重读、**`spawn_blocking` 纪律整体作废**（`asr/mod.rs:7-9`、`tts/mod.rs:37-39`、`runtime/mod.rs:7-8` 三处模块注释） | **最大**：注册表 `Built` 闭枚举（`registry/mod.rs:383-393`）加 2 变体 → `host.rs:220-246` 的 match 加臂 → `runtime/mod.rs` 的 `refresh_*` 加分支 → `session.rs`/`prefetch.rs`/测试台各加「同步 or 异步」分派（约 +8~10 处） |
| **改动面（客户端/配置）** | 0 | 0（配置层不变） | 0 |
| **`Handle::block_on` 死锁风险** | ⚠️ **高**。`spawn_blocking` 里调 `Handle::block_on` 在 tokio 1.x 是**已知危险模式**（blocking 线程允许 block_on，但若 future 内部又用到同一 runtime 的资源则可能耗尽 blocking 池——tokio blocking 池默认上限 512，`runtime/mod.rs:15` 已提到这个数字） | 无 | 无 |
| **能否支撑长连接** | ⚠️ 勉强。`block_on` 每次调用都把 future 跑完，连接可跨调用复用（client 内部持 `Mutex<TcpStream>`），但**取消语义弱**：`synthesize_stream` 的 `chunk_cb` 返回 `false` 时如何从 `block_on` 内部退出需要小心设计 | ✅ **最好**：`chunk_cb` 可直接 `async`、取消可在 `.await` 点自然生效（配合 `tokio::select!` 查 `abort` 标志） | ✅ 最好（同 b） |
| **与既有纪律的冲突** | 违反 `agents.md` 的「重推理必须 `spawn_blocking`」硬约定（`asr/mod.rs:7-9`、`tts/mod.rs:37-39`、`runtime/mod.rs:7-8`、`workflow/pipeline/inference.rs:5-6`）——但 **xfyun 已经是网络 provider 且同样走 `spawn_blocking`**（`aiui.rs:13`：「同步阻塞实现（网络 IO + 分帧读取），调用方必须 `spawn_blocking` 隔离」），**所以并不新增冲突** | **废除**该约定。`workflow/pipeline/inference.rs` 整章的推理闸门设计前提（`spawn_blocking` 无并发上限 ⇒ 需要 Semaphore）依然成立（闸门本身与 async 无关），但注释需大面积重写 | 新增分支，但旧路径保留 ⇒ **两套心智模型共存** |
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
| 抽独立 crate 且引入 opus | ❌ 会把 `audiopus`（需系统 libopus）传染到 crate 边界 | ⚠️ **建议 crate 内不做 opus**，opus 编解码留在 `server/src/media/`（§5.1 已证明不需要跨界） |
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
| **R1：Wyoming TTS 先行** | 只做 `[tts].engine="wyoming"`（Piper） | ✅ **最高**：直接替换掉 N5105 上最重的 4 线程本地合成（`tts/mod.rs:287-289`），把 CPU 让给 ASR；Piper 音质/多音色优于 Kokoro；现有 `aiui.rs` 模式可 1:1 抄 | 新增 1 文件 + 修改 6~8（§3.2） | 低。`runtime/mod.rs:224-233` 的 `tts_for` 对非 kokoro 自动走默认引擎分支，**零改动即正确** |
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
2. **`runtime/mod.rs` 的 TTS 热切换路径完整可用**（`refresh_tts`，`runtime/mod.rs:375-434`），改完立刻能热切换；ASR 无此路径（§2.4）⇒ ASR 的每次调整都要重启验证，迭代慢。
3. **收益更直接**：省掉 N5105 上 4 线程本地合成（`tts/mod.rs:287-289` + `docker-compose.yml` `cpus:"3.5"` 限核背景），直接缓解 §5.2c 的断续问题根因（agents.md:172-175 第 6 条「上游持续略慢于实时」）。
4. **风险更低**：TTS 的下游有三段缓冲兜底（`prefetch.rs` 下载缓冲 + `downlink/reservoir.rs` 水库 + `DownlinkPacer`，见 `agents.md:205-215`），单句失败不会立刻毁掉整轮；ASR 失败直接丢整段话（`session.rs:345-348` 只 `continue`）。

---

## ⑨ 未决问题（需要用户拍板）

| # | 问题 | 选项 | 我的建议与依据 |
|---|---|---|---|
| **1** | **本期先做 TTS 还是 ASR？** | (a) 先 TTS（推荐）<br>(b) 先 ASR<br>(c) 两个都做 | **(a) 先 TTS**。理由见 §8.2 四条：TTS 私有段/注册表/热切换三条路径均已跑通两轮，ASR 侧是首次；收益更直接（省 4 线程）；风险更低（三段缓冲兜底）。 |
| **2** | **Wyoming 上游进程由谁管？** | (a) 用户自己在别的主机上跑 whisper.cpp / Piper（**推荐**：本仓库只做 client）<br>(b) 本仓库负责拉起/守护子进程<br>(c) docker-compose 增加 whisper/piper 服务容器 | **(a)**。用户诉求原文「Rust Server 完全不需要知道 Whisper/Piper 怎么运行」正好对应 (a)；(b) 会引入进程守护/日志/健康检查的全新职责，与本仓库「能力抽象 + 状态上报」的边界不符。⚠️ 但 (a) 需要用户确认**目标部署形态**（是否打算把 whisper/piper 与本 server 放同一台 N5105？若是，CPU 竞争会让 §8.1 的 TTS 收益打折）。 |
| **3** | **WakeWord 是否接受「本期不做」？** | (a) 接受，本期只做 ASR+TTS<br>(b) 要求一并做服务端唤醒词 | 需要向用户说明**硬阻塞**：`[wakeup]` 是话术插件（`wakeup/mod.rs:1`），唤醒词识别在 ESP32 固件里（`workflow/session/wakeup.rs:9-11`），服务端看不到唤醒前音频（`session.rs:491-511`）。做服务端唤醒词 = **固件先改造成常开音频上行**。建议 (a)，并把「服务端唤醒词」列为独立需求（关联固件仓库）。 |
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
| `Engines.asr`（**不可变 pub 字段**） | `server/src/runtime/mod.rs:47` |
| `Engines.tts`（`RwLock`，可热替换） | `server/src/runtime/mod.rs:49` |
| `Engines::tts_for`（**xfyun 硬编码分支**） | `server/src/runtime/mod.rs:224-262`（分支 229-233） |
| `Engines::refresh_from_disk`（**只覆盖 Tts/Llm/Memory**） | `server/src/runtime/mod.rs:293-321` |
| `Engines::refresh_tts`（**经 `descriptor.build` 分发**） | `server/src/runtime/mod.rs:375-434` |
| `decide_refresh`（纯函数热切换决策） | `server/src/runtime/mod.rs:535-560` |
| `refresh_from_disk` 调用点（会话开始） | `server/src/workflow/session/start.rs:41-46` |

### A.4 调用点（async 化波及面）
| 内容 | 位置 |
|---|---|
| `AsrEngine::recognize` **唯一**调用点 | `server/src/workflow/session/mod.rs:318`（在 `asr_recognize` 307-321 的 `spawn_blocking` 内） |
| `blocking_result`（`spawn_blocking` 包装） | `server/src/workflow/session/mod.rs:66-76` |
| `recognize_segments`（ASR 失败只 `continue`） | `server/src/workflow/session/mod.rs:324-369`（失败分支 345-348） |
| `TtsEngine::synthesize_stream` 调用点 ×3 | `workflow/session/bench.rs:217`、`workflow/pipeline/prefetch.rs:372`、`benchmark/mod.rs:224` |
| `prefetch` 的闸门 + `spawn_blocking` + 回调 | `server/src/workflow/pipeline/prefetch.rs:352-390` |
| `InferenceGates::asr/tts/reconfigure_tts` | `server/src/workflow/pipeline/inference.rs`（`pub fn asr` / `tts` / `reconfigure_tts`） |
| `LlmProvider::chat_stream` 调用点 | `workflow/session/producer.rs`、`workflow/session/bench.rs`、`plugins/llm/retry.rs:159`、`plugins/memory/graph.rs` |

### A.5 音频链路
| 内容 | 位置 |
|---|---|
| `OpusFrameDecoder::decode_frame -> Vec<f32>` | `server/src/media/opus.rs:112-118` |
| `OpusFrameEncoder::encode_frame(&[f32])` | `server/src/media/opus.rs:53-66` |
| `OpusFrame*` 的 `#[cfg(feature="sherpa")]` 门控 + 占位实现 | `opus.rs:13/22/35/69-82/89/95/121-133` |
| `f32_to_i16_16k`（**已存在，AIUI 专用**） | `server/src/media/pcm.rs:8-27` |
| `StreamingResampler`（**无 feature gate**） | `server/src/media/resample.rs:27-38` |
| `handle_binary`（上行 Opus→VAD→ASR） | `server/src/workflow/session/mod.rs:491-511` |
| `SessionParams.uplink_sr`（来自 hello 协商） | `server/src/workflow/session/mod.rs:83` |
| `media/mod.rs` 三子模块 | `server/src/media/mod.rs` |

### A.6 配置层
| 内容 | 位置 |
|---|---|
| `Config` 12 个段 | `server/src/config.rs:115-150` |
| `Config::normalize`（唯一入口，幂等） | `server/src/config.rs:425-435` |
| `EngineSpec` / `canonical_engine`（唯一权威） | `server/src/config.rs:83-113` |
| 私有段约定（文档） | `server/src/config.rs:20-30` |
| `SECRET_PATHS`（6 条密钥路径） | `server/src/gateway/ws/config.rs:67-75` |
| `rewrite_legacy_paths`（tts/memory 旧路径重写） | `server/src/gateway/ws/config.rs:123-164` |
| `merge_client_patch` | `server/src/gateway/ws/config.rs:191-204` |
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
| `aiui::test_credentials`（凭据探测，`spawn_blocking`） | `device/voices.rs:309-310` |
| `device/voices.rs` 强耦合 xfyun | `grep 'tts.xfyun'` 命中 3 处 `clone()` + 错误文案分支 |
| `plugins/mod.rs`：「远程/云厂商实现无 feature 门槛」 | `plugins/mod.rs:21-22` |

### A.9 构建与部署
| 内容 | 位置 |
|---|---|
| `sherpa = ["dep:sherpa-onnx", "dep:audiopus"]`，`default = []` | `server/Cargo.toml` |
| `tokio features = ["full"]` | `server/Cargo.toml` |
| Dockerfile 恒带 `--features sherpa` | `Dockerfile`（`cargo build --release --features sherpa --locked`） |
| `cpus: "3.5"` 限核 | `docker-compose.yml` |
| 推理闸门/线程预算 | `server/src/runtime/mod.rs:4-17`（模块注释） |
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
