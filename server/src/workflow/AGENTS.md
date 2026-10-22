# 对话工作流指引

继承[服务端指引](<../../AGENTS.md>)。对话核心闭环：会话状态机 + 流水线环节（推理/分句/注入/TTS 缓存/预取）+ 会话协议与传输 + 下行音频 + MCP 客户端。本模块只编排时序与边界，不嵌入具体引擎实现（引擎在 [plugins](<../plugins/AGENTS.md>)，共享实例装配在 [runtime](<../runtime/mod.rs>)）。

- 会话见[会话维护](<session/AGENTS.md>)：推理许可、abort、逐句预取与回复节拍；不得在句间重置节拍，也不得删除 pacing / 预取 / 背压。
- 推理并发闸门见[推理闸门](<pipeline/inference/AGENTS.md>)：ASR/TTS 信号量包在 `spawn_blocking` 外；线程预算以 [runtime/mod.rs](<../runtime/mod.rs>) 为准。
- 协议见[协议说明](<protocol/protocol/AGENTS.md>)：serde 命名、网络序、hello 协商；改动必须跑 `mock_client` 联调——`cargo check` 发现不了 serde 错误。
- 下行见[下行维护](<downlink/AGENTS.md>)：设备队列与服务端缓冲的区别；缓冲能吸收间歇停顿，不能弥补持续慢于实时的上游。
- MCP 见[MCP 契约](<mcp/AGENTS.md>)：抽帧 / 清单 / stop 契约；设备工具白名单由技能目录校验，LLM 只能提名不能指定工具或动作。

## 上级入口

[服务端指引](<../../AGENTS.md>)

## 直属下级

- [会话](<session/AGENTS.md>) `session/`
- [流水线](<pipeline/inference/AGENTS.md>) `pipeline/`（inference/splitter/inject/prefetch/tts_cache）
- [协议](<protocol/protocol/AGENTS.md>) `protocol/`（protocol/transport）
- [下行](<downlink/AGENTS.md>) `downlink/`
- [MCP](<mcp/AGENTS.md>) `mcp/`
