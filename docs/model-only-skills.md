# 模型唯一意图识别与配置驱动技能

本文说明本轮源码的职责和迁移边界，不代表已发布到 NAS，也不代表现有模型已达到生产验收标准。真实模型历史结果见[验证记录](<nlu-model-validation-20261101.md>)，底层契约以[技能系统](<../server/src/skills/AGENTS.md>)及测试为准。

## 1. 三层职责

1. **NLU 只选意图**：真实训练分类器每个用户轮次最多调用一次。`other`、低分、未知/未授权技能不能产生动作；关闭模型只走普通聊天，加载/推理错误明确报错。关键词、模板规则、启发式打分和聊天 LLM 工具均不再作为意图识别后门。
2. **技能是保存的配置**：训练例句、槽位定义、有限条件、动作步骤、成功应答。模板只为模型已接受的**同一个**意图抽槽；抽不到明确澄清，不跨技能再匹配、不编造参数。
3. **执行走稳定能力**：白名单能力调用统一 `SkillSystem`，可信设备/会话身份取自连接上下文。管理 HTTP API 与语音内部调用复用同一服务，内部不绕一圈 HTTP。没有任意脚本、shell、SQL 或通用 HTTP 执行能力。

使用已有 API 改技能无需重新编译 Rust；**新增模型标签仍须重新训练并独立验收**。这两件事不能混为一谈。

## 2. 当前初始命令与技能清单

提醒初始种子是[可读 JSON](<../server/src/plugins/nlu/defaults.json>)；`end_session` 从保存的 `[command]` 词表导出到相同目录、训练输入和标签清单。命令词表不是执行匹配器。

| 意图 | 初始例句/模板 | 槽位 | 执行及边界 |
|---|---|---|---|
| `end_session` | 退下、闭嘴、关闭 | 无 | 先告别，再 MCP stop-listening；老固件/工具不支持/超时按原契约结束当前会话，不是设备关机 |
| `reminder_countdown` | `{delay}后提醒我{task}`、`过{delay}提醒我{task}`、`{delay}之后提醒我{task}` | delay、task | `tasks.create` 的 after；例如“五分钟后提醒我喝水” |
| `reminder_schedule` | `{time}提醒我{task}` | time、task | `tasks.create` 的 at，Asia/Shanghai；例如“后天下午三点提醒我开会” |
| `reminder_repeat` | `每隔{interval}提醒我{task}` | interval、task | `tasks.create` 的 interval；例如“每隔半小时提醒我喝水” |
| `reminder_daily` | `每天{daily_time}提醒我{task}`、`每日{daily_time}提醒我{task}` | daily_time、task | daily；例如“每天08:00提醒我吃药” |
| `reminder_workdays` | `每个工作日{daily_time}提醒我{task}` | daily_time、task | workdays；例如“每工作日上午九点提醒我打卡”，工作日指周一至周五 |
| `reminder_weekly` | `每周{weekday}的{daily_time}提醒我{task}`，也支持无“的” | weekday、daily_time、task | weekly；例如“每周1的10点提醒我开会” |
| `reminder_monthly` | `每月{day}日{daily_time}提醒我{task}` | day、daily_time、task | monthly；例如“每月31日晚上八点提醒我对账”，没有31日的月份跳过 |
| `reminder_yearly` | `每年{month}月{day}日{daily_time}提醒我{task}` | month、day、daily_time、task | yearly；例如“每年2月29日八点提醒我纪念日”，非闰年跳过 |
| `reminder_hourly` | `每小时的{minute}分提醒我{task}`，也支持无“的” | minute、task | hourly；例如“每小时的20分提醒我休息”，不是从现在起隔一小时 |
| `reminder_list` | 查询我的提醒、我有哪些提醒 | 无 | tasks.list → 播报真实结果，只查询当前设备 |
| `reminder_cancel` | `取消提醒{task}`、`取消{task}的提醒` | task | tasks.cancel → 播报真实结果，按文案精确选择；同名多条要求进一步明确 |

- 阿拉伯周几和数字钟点相邻时须加“的”，如“每周1的10点”；不把“每周110点”猜拆成周一10点。中文“每周一10点”可按封闭周几边界解析。
- 原来的三个提醒只是 `reply` 动作，不会保存任务。本轮新安装默认种子才使用真实任务 plan；**存量同名自定义 reply 不自动改成有副作用的计划**。
- 默认定义在缺省配置时播种，保存后可编辑。显式 `nlu.intents=[]` 不会重新注入提醒；若 command 仍开启，目录仍可有独立的 `end_session` 命令类。
- 历史本地 NAS 快照中的 `help_guide_demo` 是用户自定义帮助技能，不是新安装默认项；本轮未重新检查线上配置，不能声称它当前在线存在。
- 音量、Wi-Fi、重启、音乐、BLE、文件与固件管理接口**不自动变成语音技能**。要做语音技能，必须明确配置训练例句和受控动作，并确认目标设备实际支持对应能力。
- 全量**保存态**清单以管理台读取的 `GET /api/skills/catalog` 为准；`source=default` 只表示定义与种子相等，不是编辑历史证明。

## 3. 稳定能力目录

| 能力 | 作用 | 成功含义 |
|---|---|---|
| `tasks.create` | 创建 after/at/interval/daily/workdays/weekly/monthly/yearly/hourly 持久任务 | SQLite 已提交，返回真实日期/频次 speech，不是提醒已播报 |
| `tasks.list` / `tasks.get` | 查绑定设备的任务 | 真实存储查询 |
| `tasks.cancel` | 取消任务并撤销未提交 outbox/租约 | 不再安排新投递；已交给承载的数据不能撤回 |
| `messages.send` | 写绑定设备的持久消息 outbox | pending，不是已经播音 |
| `devices.call` | 调用本会话 MCP 设备工具 | 静态工具名、已发现的工具范围，检查设备返回结果 |
| `sessions.end` | 结束当前会话聆听 | 沿现有 MCP/兼容降级路径执行；只能作为末步 |
| `replies.say` | 所有步骤成功后的延迟应答 | 不是任意位置即时播报，也不是设备播放确认 |

能力参数与边界由 `GET /api/skills/capabilities` 返回。它是配置执行白名单，不是向模型开放任意工具。

### 一个不需要新增 Rust 分支的技能定义

以下是 `nlu.intents` 中一项，另外需声明开放的 `delay`、`task` 槽位：

```json
{
  "name": "reminder_countdown",
  "templates": ["{delay}后提醒我{task}", "过{delay}提醒我{task}"],
  "action": {
    "kind": "plan",
    "reply": "",
    "plan": {
      "conditions": [{"kind": "slot_present", "slot": "task"}],
      "steps": [{
        "api": "tasks.create",
        "args": {"text": "{{slots.task}}", "after": "{{slots.delay}}", "timezone": "Asia/Shanghai"}
      }, {
        "api": "replies.say",
        "args": {"text": "{{steps.0.speech}}"}
      }]
    }
  }
}
```

- 条件仅支持 `slot_present`、`slot_equals`、`device_kind`；不支持任意表达式。
- 插值逐 JSON 值替换，不构造 JSON 源码；缺槽/未知变量/二次模板注入拒绝。设备 ID 不能被槽位覆盖，API 名和设备工具名必须是配置中的字面量。
- `{{steps.N.speech}}` 只允许引用本计划前面已成功的 tasks.create/list/cancel 结果，且只用于 `replies.say.text`；N 是原始零基步骤索引。禁止前向/自引用、任意结果字段以及将结果带入后续动作参数。创建后回读持久服务计算的完整日期/时区/频次，查询与取消不使用固定的假成功文案。
- 先检查全部条件、步骤和参数引用，再执行第一步；成功应答在动作真正成功后才播。
- 多步不是跨 SQLite/设备的全局事务：后一步运行失败不会回滚已提交的前一步，不能播整体成功或谎称全部撤销。turn abort 只阻止尚未开始/提交的新动作。

## 4. 管理 API

全部沿现有管理鉴权；不开放匿名执行入口。开发模式可以真实 SQLite CRUD，但没有生产调度器/设备播报，不伪造效果。

| 方法与路径 | 用途 |
|---|---|
| `GET /api/skills/catalog` | 统一保存态技能/槽位目录、来源语义 |
| `GET /api/skills/capabilities` | 稳定能力及参数元数据 |
| `GET /api/skill-tasks?device_id=...&limit=50&offset=0` | 任务分页，可由管理权限跨设备只读 |
| `POST /api/skill-tasks` | 显式指定设备创建任务，建议始终提供稳定 request_id |
| `GET /api/skill-tasks/{id}?device_id=...` | 查指定任务 |
| `POST /api/skill-tasks/{id}/cancel` | 取消该设备任务 |

创建示例（与语音创建共用任务服务）：

```json
{
  "device_id": "example-device",
  "request_id": "client-generated-stable-request-id",
  "args": {"text": "喝水", "after": "五分钟", "timezone": "Asia/Shanghai"}
}
```

同一设备的相同 request_id/参数重试返回原结果，不同参数复用键拒绝；幂等窗口为 7 天，过窗不保证去重。管理请求超时后后台事务可能仍完成，应使用同一键查询/重试，不能换键盲目再建。

## 5. 支持范围与投递语义

- after/interval：明确正时长，1 秒至 366 天。
- at：带偏移 RFC3339，或今天/明天/后天/大后天（0..3天）加明确钟点；支持上午/下午/晚上等已定义时段，保留原文。显式日期已过去则拒绝；不带日期的钟点采用已声明的“下一次到达”策略：今日未来则今日，否则明日，创建后回读完整日期/时间/时区。
- daily/workdays/weekly/monthly/yearly 的 time：HH:MM 或受限中文钟点。每日钟点单独使用 daily_time，训练时不混入“明天/后天”。歧义的时段十二点要求明确 00:00/12:00。
- hourly 是每个自然小时的指定分钟（0..59），不同于从创建时刻开始每隔一小时；工作日固定周一至周五，不包含中国法定节假日/调休规则。
- monthly/yearly 遇没有该日的月份/年份跳过，不夹紧成月底；每年2月30日这种永远不存在的月日组合拒绝。所有循环计算下一次严格未来时刻，漏跑只合并、不补发洪泛。
- 时区只支持 UTC 和固定 UTC+08:00 的 Asia/Shanghai；默认时钟配置显式使用后者，不假装支持其他时区/DST。新单次和间隔任务保存创建时区以便查询一致；旧记录缺失时区则明确用 UTC，不编造历史设置。
- 任务独立存于配置库旁的 skills.db，进程重启保留。调度 worker 只生成持久 outbox；真实本地设备会话空闲时 claim→播放本地720ms短旋律→播报文案→提交状态。铃声和文案共享同一次 TTS start/stop、节拍器与既有设备下行，每个音源均须完成，不能用铃声已发送掩盖文案失败。普通 messages.send 不带闹铃。消息不作为 ASR 文本重新注入 NLU。
- 离线保留，循环漏跑按当前时间合并，容量满明确背压，不洪泛补播、不挤掉 pending。租约避免多会话同时领取，但崩溃后的重试仍可能重复，不能承诺 exactly-once 播放。
- `schedule_state=completed` 只表示单次调度已生成 outbox；`delivery_state` 才区分 pending/leased/submitted。`played=null` 表示没有设备播放确认，不将 submitted 叫作 played。
- 测试客户端与 AIUI 托管云闭环不领取本地 outbox；不能因此宣称 AIUI 已支持提醒播音。

## 6. 迁移与验收

- 新工作流使用 `nlu.model`。旧 `command.gate`、`nlu.rules`、`skill.gate` 仅作为配置兼容别名；同轮仍只有一次模型选择和一次执行，旧作用域/条件不得扩大权限。
- 旧 command.match_mode/max_chars 和 nlu.model_threshold/model_margin 只兼容读入，不再暴露为有效识别调参。管理台不再宣传三段级联或规则优先。
- 保存配置 → 从该 revision 构建候选 → 候选独立正负例真推理 → 用户明确采用。构建不自动改生产开关/路径。新 end_session 标签需要重训；旧候选清单保持不可变，不能向旧 ONNX 假加标签。
- 本轮源码回归使用可注入分类器、假承载、真实 SQLite 和 fake clock；这些验证授权/副作用/持久化契约，**不证明真实模型泛化、设备播放或线上部署**。
- 历史真实候选存在高置信误接受否定/含糊句的问题，详见[验证记录](<nlu-model-validation-20261101.md>)；本轮未降低阈值来制造通过，也未自动启用该候选。发布新服务后仍须重新训练并用隔离正负例验收，尤其是 end_session 误触发。

集成验证命令：

```bash
~/.cargo/bin/cargo test --manifest-path server/Cargo.toml --locked
DOCS_RS=1 ~/.cargo/bin/cargo check --manifest-path server/Cargo.toml --locked --features sherpa,nlu
python3 -B -m unittest discover -s tools/nlu-trainer -p 'test_*.py' -v
```

实际执行结果另以当前交付报告为准；`DOCS_RS=1` 只证明 feature 类型检查，不代表真实模型链接、推理和真机验收。

## 7. cn2t 参考项目核查

用户提供的 [cn2t](<https://github.com/WFLing-seaer/cn2t>) 已做只读源码核查，基准提交 `eea0a600140280cc233ea5c66a94e770de3abf62`。没有安装依赖或执行该项目代码，未做准确率/性能实测，也未复制其源码或词库。

- [词典](<https://github.com/WFLing-seaer/cn2t/blob/eea0a600140280cc233ea5c66a94e770de3abf62/lexicon.yml>)确有今天/明后大后天、时段、周/月/年、点/分规则；但未见每天/每周/工作日等重复任务生成流程，CYCL 元数据不等于闹钟 recurrence。
- [Python 入口](<https://github.com/WFLing-seaer/cn2t/blob/eea0a600140280cc233ea5c66a94e770de3abf62/__init__.py#L147-L309>)中 full_parse 无统一 now/tz 参数；下层可分别传基准，但最终构造 naive datetime，没有本项目所需的完整时区、未来优先和歧义澄清策略。结果也可能是时间范围，不能把范围起点未经校验当闹钟时刻。
- [依赖清单](<https://github.com/WFLing-seaer/cn2t/blob/eea0a600140280cc233ea5c66a94e770de3abf62/requirements.txt>)含多个未锁版本的 Python 包，导入还涉及 YAML 与临时词典 I/O；不能凭“无大模型”推导零成本或并发安全。
- 当前 [完整树](<https://api.github.com/repos/WFLing-seaer/cn2t/git/trees/eea0a600140280cc233ea5c66a94e770de3abf62?recursive=1>)未见许可证文件，GitHub 元数据为 license=null；许可不明确，不认定可自由复制。

结论：可参考分层思路，但本轮不直接依赖/搬入 cn2t，也不增加 Python 生产运行时。时间参数采用独立 Rust 实现并注入统一时钟与明确时区；NLU 仍只由模型授权意图，时间词不能绕过它直接创建任务。
