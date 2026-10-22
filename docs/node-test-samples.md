# 节点测试：快照、输入与样本

## 实现边界

服务端统一入口为[节点测试服务](<../server/src/benchmark/mod.rs>)，HTTP 接线为[测试路由](<../server/src/gateway/ws/bench_api.rs>)。
该服务只诊断 **SQLite 已保存配置**；不更新配置、不刷新/发布生产引擎、不共享设备会话历史、不执行设备动作、不写记忆、不读写成品音频缓存。

- `GET /api/test/capabilities`：只读配置、纯校验与音频模型文件存在性检查；不构造引擎、不联网、不调用会写临时文件的记忆健康探针。
- `POST /api/test/run`：统一 `action_id` 调用。
- 原 `POST /api/test/llm`、`/api/test/tts`、`/api/test/vad`、`/api/test/llm/models` 保留；新增 `/api/test/asr`、`/api/test/nlu`、`/api/test/pipeline`。
- 所有测试端点复用服务端管理 token 验证：`Authorization: Bearer <token>`；保留既有浏览器 `?token=` 兼容分支。JSON 中的 token 或草稿配置不能授权请求；未配置管理 token 时沿用既有免验证策略。
- 每个执行请求只调用 `ConfigStore::load` 一次；不读启动配置或生产引擎作为失败兜底。所有阶段消费这份固定快照，即使测试期间另有管理请求保存新配置也不混用。
- `revision` 复用配置 API 的 secret-safe 修订值；凭据仅参与“是否设置”，不输出原文。`expected_revision` 不匹配返回 `409 revision_conflict`。
- 当前不支持 `draft_patch`、`device_id`、设备 overlay；未知字段显式拒绝，不静默忽略。
- 旧 LLM `api_base/api_key` 与 TTS `lang/vcn/speed` 只影响本次隔离实例；响应标记 `source=saved+request`，修订值仍标识底层保存快照，绝不持久化。

## 动作与诚实的结果语义

[能力投影](<../server/src/benchmark/capabilities.rs>)覆盖客户端的所有标准及功能节点动作。

| 动作 | 输入及含义 |
|---|---|
| `vad.sample` | 内置真实语音样本或指定 PCM；构造独立 Silero；统计语音段。默认编译桩不可用于成功证据。 |
| `asr.sample` | 内置真实语音样本或指定 PCM；独立真实 ASR；空转写不是识别成功。 |
| `asr.recording` | 必须提供 typed PCM；不隐式回退到内置样本。 |
| `nlu.classify` | 真实 `SkillRegistry::parse_staged` 级联提名；仅 dry-run；规则/本地模型来源可见，绝不执行动作。TinyBERT 配置了却不可构造时明确不可用，不把降级当作模型成功。 |
| `llm.chat` | 独立 LLM、零历史、无记忆及设备工具；使用保存的 `llm.system_prompt`；不是完整上下文流水线。 |
| `tts.synthesis` | 独立真实 TTS；统计 provider PCM，不回填生产实例/缓存。 |
| `pipeline.text` | 显式 `skipped` VAD/ASR，再 NLU→LLM/技能回复→TTS；NLU 命中只预览提名。 |
| `pipeline.audio` | VAD→ASR→NLU→LLM/技能回复→TTS；无语音时明确停止，不把静音送入 ASR 当识别成功。 |
| `input.profile`、`output.profile` | 配置/格式预览；不是实际传输/播放验证。 |
| `wakeup.preview`、`command.match`、`soul.preview` | 复用既有纯预览/匹配服务；不播放问候、不执行命令。 |
| `memory.recall`、`memory.writeback.dry_run` | 当前没有快照隔离、只读且不迁移/后台索引的适配器，明确 `unavailable`。现有独立 scoped 记忆管理 API 不受影响。 |
| `tts.playback` | 必须原生客户端诊断；HTTP provider 测量不冒充播放成功，明确 `unavailable`。 |

`pipeline` 的 `context_mode=pipeline_preview`：保存的人格在 LLM 分支参与组装，零会话历史；当前记忆召回不执行、写回抑制、设备动作抑制；`production_fullflow=false`。AIUI 托管全链路没有隔离适配器，明确不可用，不以本地链路替代声称成功。

所有节点共用原有 `state/elapsed_ms/first_packet_ms/engine/text/error` 字段；追加 `request_id/action_id/source/revision/error_code/readiness/context_mode/stages/details`。

- `readiness=unavailable`：当前不能构造/调用；不是“失败后回退到桩仍成功”。
- `readiness=configured`：配置/文件前置条件允许尝试；**不证明**原生模型或远端推理成功。只读能力查询永远不返回 `provider_verified`。
- `readiness=provider_verified`：本次实际调用了真实服务；不推导其他 stage、下次请求、设备传输或端到端播放已验证。
- 测试用 fake factory 只用于回归测试，结果仍是 `configured` 并标记 `test_double`，不能作为真实推理证据。
- `first_packet_ms` 保持兼容，但含义由 stage 的 `measurement` 标识。LLM 用 `first_token_ms/provider_token`；TTS 用 `first_pcm_ms/provider_pcm`；VAD 的 `first_segment_ms/provider_segment` 是语音段产出，不是播放首包。
- `socket_written=false`、`played=false` 明确测量边界；构造耗时仅在 `build_ms` 单列，不伪造首包时间。

## 统一输入与资源上限

统一请求示例：

```json
{"action_id":"asr.sample","sample_id":"harvard.en.first","expected_revision":"<GET capabilities revision>"}
```

显式录音输入示例（数据只为形状示例，不是有效语音识别证据）：

```json
{"action_id":"asr.recording","audio":{"format":"pcm_s16le","sample_rate":16000,"channels":1,"data_base64":"AQIDBA=="}}
```

- `sample_id` 只能选择能力响应中的内置 catalogue；未知 ID（包括路径/URL）拒绝。
- 不接受任意文件路径或远程音频 URL。音频从二进制内嵌字节或有界 JSON PCM 来，不在请求中访问文件系统/网络。
- typed PCM 固定 16 kHz、单声道、signed16 little-endian、标准 base64，非空偶数字节；最多 15 秒。客户端负责明确解码/重采样，不把 Opus/整 WAV 字节冒充 PCM。
- `sample_id` 和 `audio` 互斥；文字动作不静默忽略音频资源。`text` 及旧覆盖项最多 4096 字节；HTTP body 最多 768 KiB。
- 进程内最多两个诊断请求（节点、流水线、模型目录共用）；超额立即 `429 busy`，不无限排队。重型 ASR/TTS 还必须取得**生产同一组推理闸门**。
- 每次节点/流水线 deadline 45 秒；模型目录网络 deadline 15 秒、响应最多 256 KiB、最多 1000 项。
- LLM/ASR 文字输出最多 16 KiB；TTS 不累计/返回完整 PCM，仅统计片段，最多 60 秒输出，检查采样率一致和有限数值；VAD 输出样本量也有界。
- 原生同步构造/识别不能强制抢占；HTTP 超时/断开只取消可协作工作，工作线程在**实际退出前**继续持有测试容量及推理许可，避免超时请求释放容量导致无限后台工作。TTS 回调/VAD chunk 检查取消。

## 内置语音样本来源、转换与许可

样本文件：[Harvard opening PCM](<../server/test_assets/harvard_first_sentence_16k.pcm>)；catalogue ID `harvard.en.first`。

- 来源：Real Python 的 [Speech Recognition with Python 资源库（固定 commit）](https://github.com/realpython/python-speech-recognition/tree/0c07b810808c01144a9611faf84739f24513184e)。
- 原始资源：[harvard.wav（固定 commit）](https://raw.githubusercontent.com/realpython/python-speech-recognition/0c07b810808c01144a9611faf84739f24513184e/audio_files/harvard.wav)。
- 背景及参考转写：[The Ultimate Guide to Speech Recognition With Python](https://realpython.com/python-speech-recognition/)。开头参考句是 **“The stale smell of old beer lingers.”**；这是样本内容提示，不是对当前配置模型准确率的保证。
- 许可：[该资源库的 MIT LICENSE（固定 commit）](https://github.com/realpython/python-speech-recognition/blob/0c07b810808c01144a9611faf84739f24513184e/LICENSE)，Copyright (c) 2018 Real Python；下方保留完整通知。
- 原始 WAV：44.1 kHz、双声道、16-bit PCM、809508 frames。
- 派生：取开始 4 秒，两声道算术均值转单声道，线性插值到 16 kHz，四舍五入为 signed16 little-endian，不添加静音、不合成替代语音、不放大。
- 派生长度：64000 samples，128000 bytes，4 秒，无 WAV header。
- 原始 SHA-256：`971b4163670445c415c6b0fb6813c38093409ecac2f6b4d429ae3574d24ad470`。
- 派生 SHA-256：`b4dd7cc77adcd04c40768459d81240f0dbd7d95d23ef7cb0771d955d9d60f805`。
- 样本用于诊断，不用于训练；不会在每次请求联网下载。自带英文样本不证明中文模型覆盖效果，真实识别以返回转写为准。

### MIT 许可通知（来源原文）

```text
MIT License

Copyright (c) 2018 Real Python

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

## 验证边界

[外置服务回归](<../server/src/benchmark/tests.rs>)及[HTTP 回归](<../server/src/benchmark/tests/http.rs>)运行真实 SQLite snapshot/共享 orchestrator/实际 Axum 路由，provider 用显式注入 fake；NLU 规则和本地轻量级联复用真实 `SkillRegistry`。这些测试不证明原生模型可加载、真实 TTS 音色可用、设备收到数据或扬声器播放。

默认编译测试和 `DOCS_RS=1 cargo check --features sherpa,nlu` 仅是逻辑/类型证据；真实 native link、真实音频推理及 ESP/EchoMuse 设备体验需有模型/原生库的环境单独验收。
