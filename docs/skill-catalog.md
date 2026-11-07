# NLU 技能目录（NLU Skill Catalog）说明

- 日期：2026-02
- 对应 PRD / 架构：`docs/plans/nlu-skill-domain-prd.md`、`docs/plans/nlu-skill-domain-architecture.md`
- 口径裁决：技能按**执行者**分域——设备固件执行的归设备域（device），服务端执行的归服务器域（server），第三方设备（tuya）归 IoT 域（iot，服务端执行）。

## 1. 概念统一：一切都是 NLU 技能（NLU Skill）

NLU 层原来散落三类概念，现在统一为 **NLU 技能（NLU Skill）**。命名口径：私有概念一律带 NLU 前缀，与 LLM 生态的标准 skill / MCP 工具区分；管理台中文界面简称"技能"（语境限定在 NLU 卡片，无歧义），书面与代码英文术语用 NLU Skill：

| 统一术语 | 定义 | 覆盖的旧概念 |
|---|---|---|
| NLU 技能（NLU Skill） | 名称（= 训练标签）+ 控制域 + 训练例句模板 + 槽位 + 动作 | nlu intents、command 词表条目、写死 `mcp:<工具>` 的动作 |
| 控制域（domain） | 技能的一级分组 = 控制目标/执行者类别（device / server / iot） | 新增字段 |
| 动作（action.kind） | reply / end_session / `mcp:<固定工具名>` / plan，不新增种类 | catalog 的 kind |

- `mcp 工具`降级为设备域技能"动作"里引用的**执行资源**：工具名必须是静态字面量；会话执行时按设备 discovery 实际声明校验，未声明走既有降级话术（不崩会话、无新拦截机制）。
- `domain` 只描述分组与执行者，**不进训练输入**（`input.json` 只含 name/templates），trainer 脚本零改动。

### 1.1 概念定位：私有技能 ≠ LLM 生态的标准概念

本目录的"NLU 技能（NLU Skill）"是**项目针对 NLU 语音链路设计的私有概念**，与 LLM 生态的两个标准概念不是一回事，也不互相改造：

| 概念 | 归属 | 本项目对应 |
|---|---|---|
| NLU 技能（NLU Skill） | **私有**：为 minirbt 分类链路设计（意图 = 训练标签 + 例句 + 槽位 + 声明式动作），目标是管理台可编辑、可训练、按域维护扩展 | 本目录全部内容 |
| MCP 工具 | **标准**（Model Context Protocol）：设备固件作为 MCP server 上报工具清单，服务端作为 MCP client 消费（`workflow/mcp/discovery.rs`） | 设备域技能动作引用的执行资源 |
| LLM 工具调用 / Agent Skills | **标准**（LLM function calling、SKILL.md 规范）：本服务端语音识别走小模型分类，**无 LLM 参与**，暂不涉及 | 无 |

两层通过设备 MCP 工具衔接：NLU 技能动作（`mcp:<tool>`、plan → `devices.call`）最终都落到**标准 MCP 工具**执行——私有技能只是"语音意图 → 标准工具"的声明式映射层。未来若引入 LLM 通道，LLM 直接消费设备 MCP 工具清单（function calling），不经过私有技能概念；届时本目录的技能体系保持 NLU 通道职责不变，两者共享工具资源、互不侵入。

## 2. effective_catalog 机制（同源目录）

唯一目录入口：`crate::plugins::skill::effective_catalog(&command, &nlu)`（`server/plugins/skill/catalog.rs`）。
生产识别（`SkillRegistry`）、训练输入（`nlu_build::training_catalog`）、manifest 快照与候选映射**共用同一函数**，保证"训练标签 = 生产技能"永不漂移。

1. 以保存态 `[nlu]` 配置为基（默认 JSON 只在首次缺省配置播种，不隐藏注入）；
2. `[nlu].enabled=false` 时清空技能与槽位；
3. `[command].enabled` 且词表非空时，把词表原句（trim/去空/去重）注入为技能 `end_session`：
   - `action.kind = "end_session"`、`reply = [command].reply`；
   - **`domain = device`**（作用于当前会话/设备，按执行者归设备域）；
   - `end_session` 是保留训练标签，**只能**经此注入，禁止出现在 `[nlu].intents`（`validate` 硬约束）；
   - `[command]` 词表文件因此退化为该技能的**训练样本来源**，保存链路独立（管理台"断开对话技能"卡片）。
4. `catalog.enabled = !intents.is_empty()`。

### 目录来源与版本

`/api/skills/catalog` 每条技能带 `source`（`default`=定义等于出厂种子 / `custom`=保存态覆盖或新增 / `command`=断开对话技能训练词表）与 `domain`（全量透出）。训练产物 manifest 记录 `catalog_source`：

- `effective_command_nlu_v2`（当前）：有效目录的技能结构带 `domain` 字段，`end_session` 注入为设备域；
- `effective_command_nlu_v1`（旧产物）与缺省字段：照常读取，始终使用原快照与原 labels，不伪造新增技能。

### 兼容规则

- 旧保存态 JSON / 旧 manifest 缺 `domain` 字段：缺省 `server`，**不迁移、不改写**已保存配置；
- 枚举即白名单：非法域值（如 `"cloud"`）反序列化失败，进入既有"保存态无效"链路；
- 本次不改任何默认技能名（`end_session` 是注入名非 intent 名），旧模型标签不受影响，无需强制重训；
- **改名 = 训练标签变更**：管理台保存改名技能时强提示"旧模型标签失效，需重新训练"（仅提示不拦截，沿用保存 → 提交训练链路）。

## 3. 默认技能目录（出厂种子 `server/plugins/nlu/defaults.json`）

### 3.1 服务器域（server，服务端执行，动作 = plan → 系统能力组合）

| 技能名 | 例子 | 动作 | 系统能力依赖 | 槽位 |
|---|---|---|---|---|
| reminder_countdown | {delay}后提醒我{task} | plan | tasks.create（after）+ replies.say | delay, task |
| reminder_schedule | 在{time}提醒我{task} | plan | tasks.create（at）+ replies.say | time, task |
| reminder_repeat | 每隔{interval}提醒我{task} | plan | tasks.create（interval）+ replies.say | interval, task |
| reminder_daily | 每天{daily_time}提醒我{task} | plan | tasks.create（daily）+ replies.say | daily_time, task |
| reminder_workdays | 每个工作日{daily_time}提醒我{task} | plan | tasks.create（workdays）+ replies.say | daily_time, task |
| reminder_weekly | 每周{weekday}的{daily_time}提醒我{task} | plan | tasks.create（weekly）+ replies.say | weekday, daily_time, task |
| reminder_monthly | 每月{day}日{daily_time}提醒我{task} | plan | tasks.create（monthly）+ replies.say | day, daily_time, task |
| reminder_yearly | 每年{month}月{day}日{daily_time}提醒我{task} | plan | tasks.create（yearly）+ replies.say | month, day, daily_time, task |
| reminder_hourly | 每小时{minute}分提醒我{task} | plan | tasks.create（hourly）+ replies.say | minute, task |
| reminder_list | 查询我的提醒 | plan | tasks.list + replies.say | — |
| reminder_cancel | 取消提醒{task} | plan | tasks.cancel + replies.say | task |

提醒创建类计划默认 `timezone: Asia/Shanghai`；到期提醒先短铃声再读文案，投递与实际播报是不同环节。

### 3.2 设备域（device，固件 MCP 工具直调，动作 = `mcp:<工具名>`）

| 技能名 | 例子 | 动作（静态工具名） | 工具参数（槽位） | 说明 |
|---|---|---|---|---|
| device_volume_set | 音量调到{volume} | `mcp:self.audio_speaker.set_volume` | volume（开放槽位） | 槽位名 = 固件工具参数名；数值由固件侧解析 |
| device_mute | 静音 / 把设备静音 / 关声音 | `mcp:self.audio_speaker.set_mute` | — | |
| device_pause_playback | 暂停播放 / 暂停音乐 / 先停一下 | `mcp:self.audio_player.pause` | — | 播放器工具名以各板型 discovery 上报为准 |

工具依赖说明：以上工具名是出厂示例的**静态字面量**；非目标板型未声明对应工具时，会话执行走既有四降级路径（Unsupported / missing / unknown-tool / transport）给固定话术拒识，不报错崩会话。管理员可在管理台按同型新增设备域技能（动作选"设备工具"，工具名取设备 MCP 清单）。

### 3.3 设备域注入技能（不出厂种子，来自 [command] 词表）

| 技能名 | 例子 | 动作 | 依赖 | 说明 |
|---|---|---|---|---|
| end_session | 退下 / 闭嘴 / 关闭（默认词表） | end_session | 设备结束聆听通道 | 保留训练标签；词表即训练样本来源，管理台"断开对话技能（end_session · 设备域）"卡片编辑 |

### 3.4 IoT 域（iot，服务端执行，语音开关 tuya 设备）

| 技能名 | 例子 | 动作 | 系统能力依赖 | 槽位 |
|---|---|---|---|---|
| iot_device_on | 打开{device_name} / 把{device_name}打开 / 开一下{device_name} / 我要开{device_name} | plan | tuya.control（action 静态 "on"）+ replies.say | device_name（开放槽位） |
| iot_device_off | 关闭{device_name} / 把{device_name}关了 / 关掉{device_name} / 把{device_name}关闭 | plan | tuya.control（action 静态 "off"）+ replies.say | device_name（开放槽位） |

能力依赖说明：`tuya.control` 由服务端 TuyaEngine 执行——设备名称匹配（本地别名 → 云名称，先精确后双向包含，歧义不自动选择）+ 局域网开关 DP 下发（bool 型 DP，`id=="1"` 优先；缓存缺失先 status 回读填充）。未找到/歧义/离线/超时/不支持五类失败全部以 speech 话术收尾，会话不中断。`action` 是静态字面量（校验层拒绝插值与非 on/off 值），开关语义写死在独立技能里，不共用值槽位。

**重训提示**：这两个技能是新增训练标签，旧模型不认识——需在管理台**手动提交训练**并验收新模型后才生效（本期只落代码与种子，不自动触发训练）。

出厂种子 **17** 个技能：服务器域 12（11 提醒 + 1 天气）+ 设备域 3 + IoT 域 2；再加 end_session 注入技能（§3.3），有效目录合计 **18** 个。

## 4. 管理台消费

- **技能目录**（NLU 卡片 → 读取技能清单）：按设备 / 服务器 / IoT 三域分组展示，支持控制域筛选与技能名搜索；来源与摘要逐条透出。
- **技能编辑**：名称 / 控制域 / 训练例句 / 槽位 / 动作；控制域可改（仅影响分组展示，不影响训练标签）。
- **改名强提示**：相对保存态改名时，保存前显示"旧模型标签失效，需重新训练"警示（不拦截）。
- **断开对话技能**：`[command]` 卡片改称"断开对话技能（end_session · 设备域）"，词表编辑能力不变。

## 5. 不变量（回归时逐条核对）

1. `end_session` / `other` 是保留训练标签；`end_session` 只能来自 command 注入，绝不出现在 `[nlu].intents`。
2. 生产、训练、manifest、候选映射共用 `effective_catalog`，技能名即训练标签。
3. 技能动作中的 MCP 工具名必须是静态字面量；执行时校验设备已声明，未声明走既有降级话术。
4. 改默认技能名 = 破坏旧模型标签；任何种子重组不得改动现有技能名。
5. `domain` 不进训练输入；ESP 固件协议零触碰。
