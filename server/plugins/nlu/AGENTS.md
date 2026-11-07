# NLU配置与模型

继承[插件指引](<../AGENTS.md>)。意图选择统一通过[技能目录](<../skill/AGENTS.md>)，本目录不提供规则意图识别器。

- [配置](<mod.rs>)的首次缺省intents/slots来自[JSON种子](<defaults.json>)；Rust Default与serde省略字段同源。显式intents=[]保留空目录，保存的旧同名reply/自定义项不改写。with_builtins仅旧调用兼容clone，不注入任何隐藏技能。
- 初始时钟包11项：原countdown/schedule/repeat/daily四ID，加workdays/weekly/monthly/yearly/hourly/list/cancel（均reminder_前缀）。前9项调用tasks.create，查询/取消分别调用tasks.list/tasks.cancel；每项再用replies.say.text={{steps.0.speech}}播真实服务结果，action.reply为空避免双回执。所有create显式Asia/Shanghai，包括after/interval；不把旧reply配置静默迁成副作用。
- 日历args结构以[稳定API](<../../src/skills/AGENTS.md>)为准：workdays仅周一至周五、不含法定调休；weekly传weekday/time，monthly传day/time，yearly传month/day/time，hourly传minute。不存在的月31日/闰日跳过，不夹紧日期。裸钟点与今天/明天/后天/大后天及上午/下午/晚上保留time原文，由API解释下一次到达或明确澄清，不在NLU补日期。
- 9槽delay/time/daily_time/interval/task/weekday/day/month/minute；weekday为中文一至七/日/天及1..7封闭表，其余开放原文。每类至少两种不同训练模板。weekly支持有/无“的”；相邻槽仅允许前槽必填封闭枚举且边界唯一，不把连续阿拉伯数字拆成两个槽。daily_time保留“下午十点”等完整时段。
- 模型开关默认关闭、阈值0.75。model_threshold/model_margin只兼容旧配置，不参与识别，不向schema暴露。构建模型或候选评测不自动打开生产开关。
- [分类器](<bert.rs>)只输出标签与置信度；真实分类失败严格报错。other永不映射为动作；标签/manifest输出下标必须一致。
- [模板抽槽](<matcher.rs>)只能用于模型已接受的同一技能，完整边界消费、不做模糊包含或跨技能选择；开放文本保留原文冒号等内容，封闭值表精确等价。同技能多模板完整匹配时，优先字面量更多的显式边界（不把“的/在”吞进开放槽）；同具体度槽冲突仍NeedsSlots。模型只调用一次、置信度不改写，不能以模板结构选择意图或覆盖拒识；缺槽不交LLM再选技能。
- ActionDef.plan类型由稳定技能系统维护，kind=plan必须携带有效计划，其他kind不得夹带plan。保存验证在skill::validate，外置[测试](<tests.rs>)。
