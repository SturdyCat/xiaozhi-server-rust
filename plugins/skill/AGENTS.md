# 模型技能目录

继承[插件指引](<../AGENTS.md>)。本目录只负责保存态目录、模型决策与同技能槽位校验；副作用由会话层和[稳定能力系统](<../../server/src/skills/AGENTS.md>)执行。

- [recognize](<mod.rs>)是每轮唯一意图入口：`Accepted`携完整调用，`NeedsSlots`只澄清，`Disabled/Rejected`可纯聊天，`Unavailable`必须严格报错。真实模型每次至多分类一次；无规则、启发式或LLM重新识别。
- 模型关闭即不识别；启用但引擎缺失/分类异常/真正未知标签返回Unavailable。已知但禁用技能、other、低分不授权；无LLM工具重提通道。
- [统一有效目录](<catalog.rs>)由保存的nlu配置与command词表组成，生产、训练、manifest新产物、候选共用。other/end_session是保留名，end_session只能来自command；旧manifest不得事后注入新标签。
- 默认技能只在初始配置中由[nlu JSON种子](<../nlu/defaults.json>)播种；显式空目录不补回，已有同名reply绝不偷偷变为真实任务。
- 模板只对模型已接受的同一技能抽槽，不跨意图搜索。完整消费模板、封闭值精确等价、保留开放槽位原文；相邻槽前项必须必填封闭值表且边界唯一，连续十进制数不拆槽。同技能按声明模板的字面量长度优先显式边界，等具体度不同槽仍澄清，不能用于意图选择、修改模型置信度或再次分类。时间语义由稳定任务API验证，不在NLU补日期/猜时间。
- 动作支持reply/end_session/固定mcp工具及plan；保存时校验注册API、步骤、参数引用、同技能槽位条件。plan只允许受控能力，模型/槽位不能改变API或设备工具名称。步骤结果仅允许先前tasks.create/list/cancel的speech用于replies.say.text，不得流向API名、工具、设备或任务参数；校验/执行由稳定能力与会话泛型完成。
- `tools()`恒空、`from_tool_call`恒拒绝；旧parse包装仅供兼容诊断且同样模型唯一，生产只能消费完整Recognition，不把错误吞成聊天放行。
- [模型接线测试](<tests.rs>)和[目录/计划测试](<catalog_tests.rs>)均外置。注入分类器只验证接线/安全契约，不代表真实模型效果；当前NAS旧候选负例/歧义误接受10/24未通过，不能自动开启远端。
