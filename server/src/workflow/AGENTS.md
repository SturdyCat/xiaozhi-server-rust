# 对话工作流指引

继承[服务端指引](<../../AGENTS.md>)。对话核心闭环：会话状态机 + 流水线环节（推理/分句/注入/TTS 缓存/预取）+ 会话协议与传输 + 下行音频 + MCP 客户端。本模块只编排时序与边界，不嵌入具体引擎实现（引擎在 [plugins](<../plugins/AGENTS.md>)，共享实例装配在 [runtime](<../runtime/mod.rs>)）。

- 会话见[会话维护](<session/AGENTS.md>)：推理许可、abort、逐句预取与回复节拍；不得在句间重置节拍，也不得删除 pacing / 预取 / 背压。
- 推理并发闸门见[推理闸门](<pipeline/inference/AGENTS.md>)：ASR/TTS 信号量包在 `spawn_blocking` 外；线程预算以 [runtime/mod.rs](<../runtime/mod.rs>) 为准。
- 协议见[协议说明](<protocol/protocol/AGENTS.md>)：serde 命名、网络序、hello 协商；改动必须跑 `mock_client` 联调——`cargo check` 发现不了 serde 错误。
- 下行见[下行维护](<downlink/AGENTS.md>)：设备队列与服务端缓冲的区别；缓冲能吸收间歇停顿，不能弥补持续慢于实时的上游。
- MCP 见[MCP 契约](<mcp/AGENTS.md>)：抽帧 / 清单 / stop 契约；设备工具名只能来自模型已接受技能的保存配置，运行期再核对设备工具清单。聊天 LLM 不提供技能工具，也不接受动作回提。
- 会话外设备命令见 [device_cmds.rs](<device_cmds.rs>) 模块注释：活跃会话按 `device_id` 注册 `mpsc` 通道（与 inject 同构 + oneshot 回执），scheduler 到期动作任务经它直推一次 MCP 工具调用；transport 独占权在会话主循环，命令只在循环内展开。

## 上级入口

[服务端指引](<../../AGENTS.md>)

## 直属下级

- [会话](<session/AGENTS.md>) `session/`
- [流水线](<pipeline/inference/AGENTS.md>) `pipeline/`（inference/splitter/inject/prefetch/tts_cache）
- [协议](<protocol/protocol/AGENTS.md>) `protocol/`（protocol/transport）
- [下行](<downlink/AGENTS.md>) `downlink/`
- [MCP](<mcp/AGENTS.md>) `mcp/`

## 可配置插件工作流（当前实现）

`designer` 提供版本 1 的 workflow 编译与投影。普通构建的 SQLite 配置缺少 `workflow` 时使用兼容默认计划；**纯底座构建**缺少 `workflow` 时使用空计划，不投影或接受静态业务绑定，仅允许运行期已装载的 WASM 观察者挂到已接线 hook。保存前必须通过同一编译器，非法配置不会写入。核心 ASR/VAD/LLM/TTS/AIUI 引擎描述符仅用于只读元数据，不可作为任意 binding；普通构建当前可编排项为已接线的唤醒、人格、记忆、指令/NLU hooks。

条件只声明真实可执行子集：`always`、`text_contains{text,case_sensitive?}`、`device_kind{value}`；模型门控 `nlu.model` 的 `confidence{min,max?}` 只用本轮唯一真实分类后取得的有限分数；普通上下文无可信置信度时 fail closed，不伪造默认分数。`command.gate`、`nlu.rules`、`skill.gate` 仅为兼容别名，不提供规则识别，多别名共存也只分类/执行一次且不扩大旧目录作用域。P2d 起 wasm 插件可绑定为**观察者挂钩**（仅 `start`/`link.nlu.llm`/`end` 三个已接线 hook 位，`on_error` 恒 `continue`）：会话以 `hook.<position>` 事件旁路派发（payload 含 session/device/text），效果 fire-and-forget 异步执行——不阻塞对话轮、不回灌、不可改写对话流；保存时按当前装载目录严格校验（`compile_with_observers`），会话侧对未装载绑定宽松剔除+告警（缺插件不炸会话）。模型门控的 `on_error` 固定为 `stop`；上下文 hook 可使用 `continue`。工作流修改为 `next_session`，会话固定编译快照，不在句间重排。时钟任务走稳定[技能系统](<../skills/AGENTS.md>)，提醒投递不重新注入 ASR/NLU。
