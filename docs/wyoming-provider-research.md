# Wyoming 协议事实卡 + xiaozhi-server-rust 接入需求清单

> 调研人：许清楚（PM）｜日期：2026-10-14｜目标读者：软件架构师
> 任务：评估「借鉴 Wyoming，把 ASR/TTS/WakeWord 做成标准化 Service」这一诉求的可行性。
> **本文档只做调研与需求定义，不含实现方案与Rust 代码。**

---

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

**传输**：TCP（也支持 `unix://` 域���节与 `stdio://` 标准输入输出，见 `server.py::AsyncServer.from_uri`）。URI 形如 `tcp://0.0.0.0:10300`。
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

> 🔴 **这是本次调研最重要的结论**：xiaozhi-server-rust 与 ESP32 之间是 **Opus**（`app/audio/opus.rs`，`OpusFrameEncoder`，Voip 模式，有状态流式编码器），而 Wyoming 是 **PCM**。接入 Wyoming 意味着在这条链路上**必须新增一路 PCM↔Opus 转码**，详见 §4 风险 R2。

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

**约束②：现有调用点已用 `spawn_blocking` 兜住同步阻塞。** 来源：[app/session.rs:63-71](file:///Users/admin/Desktop/notip.com.cn/xiaozhi-server-rust/server/src/app/session.rs)

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
| | ② ⚠️ **`audiopus` 当前是 `sherpa` feature 可选依赖**（`Cargo.toml`: `sherpa = ["dep:sherpa-onnx", "dep:audiopus"]`），而 `app/audio/opus.rs` 全部类型都带 `#[cfg(feature = "sherpa")]`。**如果 Wyoming provider 在 `default` feature 下也要工作，就必须在 `default` 中启用 `audiopus`**，或接受「Wyoming provider 仅在 `--features sherpa` 下可用」并明确报错。**这是需要架构师决策的第一个岔路口。** |
| | ③ Wyoming TTS 输出可能是 **22050/24000/16000 Hz**，`audiopus` 仅支持 8k/12k/16k/24k/48k（见 `to_sample_rate`）。**22050 必须先重采样到 24k**，否则编码失败 |

#### P0-4 配置面与热切换语义

| 项 | 内容 |
|---|---|
| **验收标准** | 修改 `[asr.wyoming].host` 并 hot-reload 后，**下一个会话**使用新地址；当前会话不中断 |
| **细节** | ① 热切换语义对齐现有约定：ASR/TTS 段建议 **`HotReload::NextSession`**（新建连接有握手开销，不适合 RestartOnly 也不适合 hot） |
| | ② `engine.rs` 的**引擎缓存键必须包含 host:port**，否则改地址后会命中旧引擎缓存（这是极易漏的坑） |
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

> **真实风险（替换为）**：因为按**句子边界**触发合成，**首包延迟 = 首句文本的合成耗时**。LLM 流式输出时，若第一句是长句（如「让我查一下今天北京天气怎么样」），首包延迟将随首句长度线性增长。而 `DownlinkPacer`（`app/downlink`，已有 `lead_ms` 预充设计，默认 `lead_ms=240`，见 `tests.rs`）**假设音频是连续稳定到达的**——句子间可能出现**不规则空隙**，导致节拍器时间表频繁重排。

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
| **P1-7** | 可观测性 | 为 Wyoming provider 增加结构化日志/指标：连接建立耗时、首包延迟、错误计数，与现有 `app/metrics` 对齐 | 没有指标无法运维 |

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

- Wyoming **只支持 PCM**（已核实：`audio-chunk` 无 `format` 字段，payload 是裸 PCM）。
- 本仓库 ESP32 链路**全链路 Opus**（`app/audio/opus.rs` 有状态 `OpusFrameEncoder`，Voip 模式）。
- **代价**：
  1. 上行：ESP32 的 Opus → 本地解码为 f32（**已在做**，`OpusFrameEncoder` 的注释明确写了「上行：设备发来的 Opus 帧 → 解码为单声道 f32 PCM → VAD/ASR」）→ 现在需要**再编码回 i16 PCM** 发给 Wyoming。这一步是**纯浪费**：ESP32 发来的 Opus 本可以直接喂给 Wyoming 的 Whisper，只需在 Wyoming 服务端解一次。
  2. 下行：Wyoming 回 PCM → 需转 f32 → 编码 Opus。**这一步是必要的**，且现有链路已在做。
- **CPU 开销**（N5105，4×Jasper Lake @1.1GHz，无AVX2 单元）：
  - 上行**多出一次 Opus 编码**（16000Hz mono Voip，单帧 20ms）。Opus 编码是整数运算为主，**不依赖 AVX**，N5105 可承受；但这是**纯新增的、零收益的**开销。
  - **缓解方案（需架构师评估）**：若上行 Wyoming ASR 与本地 VAD 都存在，考虑**直接把 PCM 送VAD、Opus 送 Wyoming**（但 Wyoming 不吃 Opus，此路不通）；或**让 Whisper 服务端接受 Opus**（不可控）。
  - 实际影响：**每路会话新增约 1–3% 单核占用**（估算，需实测）。相比 sherpa 推理本身的消耗占比不高，但**在 `max_concurrent` 拉满时是雪崩因素**。
- **`audiopus` feature gate 问题**：`default = []` 下 `audiopus` 不编译，`app/audio/opus.rs` 的全部类型都被 `#[cfg(feature = "sherpa")]` 屏蔽。**「 Wyoming provider 能否在 `default` feature 下工作」是一个必须先回答的问题。**

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
| **Rust Server →(Wyoming)→ Whisper Server / Piper** | ✅ **这是 Wyoming 的正确落点，建议做（即P0）**。协议语义匹配、已有 Rust 社区 sherpa-onnx 服务可对接、MIT 无风险。**唯一代价是 PCM 转码（R2）。** |
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
- `server/src/app/audio/opus.rs`（Opus 编解码，全部 `#[cfg(feature = "sherpa")]`）
- `server/src/app/downlink/`（`DownlinkPacer`，`lead_ms` 默认 240）
- `server/src/app/session.rs:63-71, 307-318`（`spawn_blocking` 包装同步推理）
