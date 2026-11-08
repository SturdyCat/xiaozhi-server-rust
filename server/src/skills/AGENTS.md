# 稳定技能能力与持久调度

继承[服务端指引](<../../AGENTS.md>)。本目录不识别意图、不调用模型、不联网、不直接控制设备或生成音频；识别及会话执行分别由插件目录与会话模块负责。新增使用已有能力的技能只需保存 plan；新增底层能力才改 Rust。

weather.forecast 是本目录**唯一**允许联网的能力；其网络实际发生在 `plugins/weather`（持有 reqwest 客户端与简易缓存；`[weather].backend="wasm"` 时改经 `plugins/wasm/bridge.rs` 事件桥进 weather.official 插件，网络发生在插件 http-request 效果），本目录仅做调度与结果组装，不内联任何 HTTP 请求。其它能力仍严守「不联网」铁律。

## 类型与安全边界

- `SkillPlan { conditions, steps }`、`SkillStep { api, args }` 使用 serde JSON，均拒绝未知结构字段。最多 16 条条件、1..16 步，完整 plan <=32768 字节；args 深度 <=12、节点 <=1024。
- 条件是有限 `kind` 枚举：`slot_present {slot}`、`slot_equals {slot,value}`、`device_kind {value}`；全为真才执行。没有任意表达式、脚本或 HTTP 能力。
- `validate_plan` 静态校验能力白名单、参数形状、长度与引用语法；`resolve_step` 按 JSON 树替换字符串中的 `{{slots.name}}`、`{{ctx.device_id}}`、`{{ctx.session_id}}`，不拼接/重新解析 JSON。缺槽、未知变量、二次占位符注入报错。共享字符串解析按输出字节位置与原引用身份追踪允许保留的 tokens（ctx 未提供时只保留原 ctx+steps，提供时只保留原 steps），完成后重新扫描并严格比对，拒绝 slot-slot、literal-slot、ctx 边界拼接出的新引用；正常单花括号不禁。只有两个 ctx 变量可信；slot 的 `device_id` 永远不能改执行设备。
- `resolve_plan` 是仅槽位的解析入口，保留可信 ctx 与受限 step 引用等待执行。会话使用 `resolve_step` 并在执行前完整解析/校验所有步骤；ctx 标识自身也拒绝占位符注入。只有 `replies.say.text` 可引用 `{{steps.N.speech}}`，N 必须严格指向前面的 `tasks.create/list/cancel`；不能读取任意结果字段，不能给副作用参数/API/tool 注入结果。`resolve_step_results` 只使用按原索引记录的已成功结果，单次字符串替换且拒绝结果里的二次占位符，缺少/非字符串/过长 speech 失败关闭。API 名、MCP 工具名必须是保存配置中的静态字面量，不允许插值；MCP 还必须由会话检查本设备实际声明工具清单。
- API 执行 ctx 包含传输绑定或鉴权管理请求明确指定的 `device_id`、`session_id`、每步稳定 `request_id`、fake/真实 `now_ms`、取消 AtomicBool 和可选设备类型。API args 不接受覆写 `device_id`；所有 SQL 使用绑定参数。

## 能力契约

- `tasks.create {text, after|at|interval|hourly|daily|workdays|weekly|monthly|yearly, timezone?, action?}`：九种时间字段必须恰选一种；文本 1..2000 字符。返回真实持久化 `task` 和 `speech`，回读完整本地年月日时分秒、频次与倒计时实际目标；不能保存失败仍应答成功。
- 可选 `action`（到期动作）：必须是**单个** `devices.call` 步骤（`{api, args:{tool,arguments}}`，tool 静态字面量）。`validate_create_action` 白名单拒绝 `tasks.*`（防"到期再排到期"递归）、`replies.say`/`messages.send`（到期播报已由 text 覆盖）与 `sessions.end`（无会话上下文）。嵌套 args 同样深递归做 `{{slots.*}}` 替换与深度/节点计数。到期时**不进 outbox**：`tick_due` 把它放进 `TickReport.actions` 交还调用方（`runtime/skill_tasks`）经 `workflow::device_cmds` 向在线设备直推一次；离线/忙/失败降级为纯文字提醒（原文经 `enqueue_task_speech` 进 outbox）。背压与普通任务一致：outbox 满时动作任务连调度都不推进。
- `after`/`interval` 接受明确正时长，如 `五分钟`、`10分钟`、`半小时`、`每隔两小时`；范围 1 秒..366 天，不支持的分数/模糊表达报错。
- `at` 接受带偏移 RFC3339，或 `今天/明天/后天/大后天`（0..3天）加钟点；明确日期的过去时刻拒绝。无日期钟点按该时区的下一次到达解释：今天未来则今天，否则明天，真实日期必须回读确认。钟点支持 `HH:MM`、`八点`=08:00、`下午八点半`=20:30，以及有明确时段的 `上午10`/`下午3`/`晚上8`。非“中午”的十二点时段表达有歧义，拒绝并要求 `00:00`/`12:00`。
- `hourly:20` 或 `hourly:"20分"` 表示每个墙钟小时第20分钟（0..59），不是从创建时刻每隔一小时；`daily:"08:00"`；`workdays:"08:00"` 严格周一至周五，不排除法定节假日；`weekly:{weekday:"1",time:"10点"}` 星期1..7对应周一至周日（支持中文/周一/星期日）；`monthly:{day:"31",time:"08:00"}`；`yearly:{month:"2",day:"29",time:"08:00"}`。数值字段可用整数或中文/数字字符串，其他未知对象字段拒绝。
- 日历 next 严格晚于 now；月底没有31日、平年没有2月29日时跳过，不夹紧成其他日期；永远不合法的2月30日等输入直接澄清。每月搜索至多12个月，每年搜索至多8年（覆盖非闰世纪造成的闰年间隔）；循环离线合并，不补发历史每次。
- timezone 只支持 `UTC` 和固定 UTC+08:00 的 `Asia/Shanghai`，省略为 UTC；默认时钟技能显式用 Asia/Shanghai。其他区域拒绝，不假装支持夏令时/历史时区规则。新的单次/倒计时/间隔也持久化所选时区，查询按每项原时区回读并标明时区；旧无时区记录只能按明确标识的 UTC 展示，不猜历史时区。
- `tasks.list {limit?,offset?}`（1..100、0..10000）返回本设备活跃任务及已到期但未提交的提醒，附确定性有界 `speech`（至多8项、约1800字符，说明总数和省略项）；`tasks.get {task_id}` 只看 ctx 绑定设备。管理端显式鉴权的 `list_tasks(Option<device_id>,...)` / `get_task(id,Option<device_id>)` 保留全状态跨设备只读语义。
- `tasks.cancel {task_id}` 或 `{text}` 必须恰选一种；text 按当前设备活跃/待投递提醒的完整内容精确匹配。同名多条或未匹配抛出可播 `SkillActionError {speech}`，停止整个 plan、绝不全删；歧义引导管理任务列表按编号处理。成功时真实撤销任务及未提交 outbox/lease，并返回 `speech`；已取消的按ID重试安全。时间解析失败同样使用明确的 typed 澄清，不暴露 DB 内部错误。
- `messages.send {text}`：真实写入绑定设备 outbox，返回 `pending`，不是已经播报。`devices.call {tool,arguments}`、`sessions.end {}`、`replies.say {text}` 只返回 `dispatch` 给会话，不在本目录产生设备副作用。`sessions.end` 只能是最后一步；`replies.say` 是整个 plan 成功后的延迟 receipt，并非任意位置即时播报。
- 会话在第一个 effect 前对所有已解析步骤调用纯 `preflight_step_args`：检查 timezone 白名单、时长语法/范围、共享 at 日期/钟点语法及日历字段/永远非法月日；非法任务参数返回 typed 安全澄清。该预检不读 DB、不取当前时钟、不判过去，合法但已过期的绝对时间仍可进入 API 先读取原幂等回执；当前是否未来只在真正创建时判断。运行期容量/取消/存储错误仍可能造成部分提交。
- 多步 plan **不是跨步骤全局事务**：后一步运行期失败不会回滚已经提交的前一步，失败不得播整体成功 receipt。取消在开始、取得写锁后与每个事务提交前检查；已经提交的操作不能被后来的 turn abort 撤销，会话须在 await/派设备前后检查取消并停止后续步骤。

## SQLite、容量与投递

- `SkillSystem::new(PathBuf)` 只保存路径，懒打开独立文件；构造不 IO、不启动 worker。不支持 `:memory:`。全部 API 为同步阻塞，由会话、HTTP、后台 worker 用 `spawn_blocking` 调用。dev 可做真实 SQLite 管理，但 root worker 和设备自动投递必须关闭，不伪造模型/音频成功。
- SQLite WAL、5 秒 busy timeout、`BEGIN IMMEDIATE` 串行化写事务。一次操作中任务/outbox/幂等响应一起提交；进程重启保留未来、离线与已租赁消息。持久幂等键为 `(device_id, request_id)`，相同 key 不同参数拒绝；相同逻辑 step 重试必须用相同 request_id，不得每次重试生成新 UUID。
- 幂等保留 **7 天**（过窗可复用），全局最多 8192 条；活跃任务全局 1024 / 每设备 128，待投递全局 2048 / 每设备 256；满容量明确报错，永不挤掉离线 pending。终态 outbox 保留最近 2048，已无任何 outbox 引用的终态任务另保留最近 2048。读取不会主动触发投递。
- `tick_due(now_ms)` 每轮最多处理 256 个可推进到期任务，snapshot 先排除容量阻塞设备；满容量的单次任务保持 active 等待空间，不丢失。循环任务按当前时间推进至下一次，跳过历史频次；同任务最多一个 pending/leased，离线期间合并而非洪泛。返回 `TickReport { materialized, actions }`：普通任务照旧物化 outbox，`action` 非空的任务不物化、由调用方直推（见能力契约的 action 条目）。
- `claim_delivery(device,session,now)` 用原子租约取本设备消息；同设备同时最多一条有效 lease，5 分钟过期，重启后可重新 claim。`delivery_is_current` 供会话播报前/播报中复查取消和租期；旧 token/其他设备/其他会话不能 ack 新租约。
- `finish_delivery(claim,true,now)` 仅标 **submitted**（完整音频交给传输），从不证明 **played**；false 释放为 pending，1 秒后可重试。会话必须只在 idle 时 claim，并在租约失效/任务取消/abort 时停音频，不把消息重新当用户输入。
- tasks API `state`/`schedule_state` 仅表示调度状态；单次到期写入 outbox 后是 completed，但 `delivery_state` 仍可能 pending/leased。`played:null` 表示无实际播放确认。submitted 后取消不能撤销已经交给传输的数据。
- 租约提供单一当前所有者，不提供端到端 exactly-once 播放：若已提交音频后进程在 ack 前崩溃，重试可能重复，不能声称设备播放去重。

## 验证

- Rust 测试在 `tests*.rs`，使用注入 `now_ms`，不 sleep、不联网；覆盖持久化重启、离线、重复请求、取消、竞争 claim、过期 lease、循环合并、容量隔离、参数注入、未知能力与时间歧义。
- 集成负责人执行 `cargo test --manifest-path server/Cargo.toml skills::` 与总体编译；仅静态格式检查不等于这些测试已运行。真实 MCP/TTS/设备播放需另外验收。
