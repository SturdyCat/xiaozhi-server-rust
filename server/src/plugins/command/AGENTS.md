# 结束聆听技能配置

继承[插件指引](<../AGENTS.md>)。本模块只维护end_session训练样例、目录开关与告别语，不再有运行期关键词闸门。

- 首次词表与告别语来自[JSON种子](<defaults.json>)。保存词表仅trim、去空、去重，保留语气词与标点原句；[统一有效目录](<../skill/catalog.rs>)将其作为end_session模板导出，训练/manifest/候选与生产同源。
- match_mode/max_chars只兼容读取旧配置，不参与识别、保存词表长度限制或有效签名，也不向schema暴露。长结束语样例合法；训练划分不足必须补真实变体，不能把同句复制到训练与测试。
- command.enabled只决定end_session目录条目。模型关闭时不识别任何命令；模型启用却不可用时明确失败。即使说出精确词表也不能绕过真实分类器。
- CommandGate仅旧诊断调用兼容，hit恒None，生产必须调用SkillRegistry::recognize。没有任何隐藏分类器或LLM动作回提。
- 模型接受后的告别、MCP stop与旧固件兼容行为归会话层，不在本模块执行。
- [测试](<tests.rs>)外置，覆盖无规则旁路、保存态词表和旧键兼容，不代表真实模型效果验收。
