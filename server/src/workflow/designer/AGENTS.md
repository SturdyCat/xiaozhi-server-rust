# 可执行工作流编译

[编译器](<../designer.rs>)只验证和排序会话固定快照，不直接识别意图或执行技能。核心 VAD/ASR/LLM/TTS/AIUI 引擎仍不可作为任意 hook 绑定。

- 普通构建的默认计划只包含一个 `nlu.model` 模型门控。**纯底座构建默认计划为空**，静态 `wakeup.greeting`、`soul.profile`、`memory.*`、`nlu.model` 及兼容别名均不进入目录，也不能作为 binding；仅当前已装载的 WASM 观察者可绑定三个 hook 位。普通构建中旧 `command.gate`、`nlu.rules`、`skill.gate` ID 兼容读取并显示为 alias；即使共存，生产会话也只分类一次、最多执行一个技能。旧 command hook 只授权 `end_session`，旧 NLU hook 只授权保存 NLU 目录的技能；别名不会恢复规则/启发式识别。
- 安全门控 `on_error=stop` 不允许 continue。`text_contains` 是执行条件，不是意图识别器；必须先有真实模型接受的同一意图。`confidence` 在唯一分类后用真实有限分数评估；普通上下文没有该分数时 fail closed。
- 明确空 bindings 关闭可选增强，不关闭核心语音/纯聊天。禁用项同样验证结构、条件和错误策略，防止保存潜伏的无效配置。
- 编排同位置按 order、同序按稳定 binding id 排列；会话固定快照 `next_session`，不在句间热切换。多模型别名不构成多个识别阶段。
- 声明式条件/动作技能计划是 [SkillSystem](<../../skills/AGENTS.md>) 的职责，由 [会话](<../session/AGENTS.md>) 绑定可信设备并执行；此编译器不开放任意脚本或 LLM 动作回提。

回归见[编译器测试](<tests.rs>)与[真实会话行为测试](<../session/plan/tests/model_behaviors.rs>)。本地无模型回归不能代替真模型拒识验收或 ESP/EchoMuse 下行联调。
