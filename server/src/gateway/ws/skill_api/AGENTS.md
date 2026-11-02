# 技能目录与任务管理 HTTP

继承[网关指引](<../AGENTS.md>)。实现见[父模块](<../skill_api.rs>)，纯粹复用共享目录与 `SkillSystem`，不自行解释计划或运行设备动作。由父路由 merge `routes()`；每个 handler 均显式沿用管理鉴权。

- `GET /api/skills/catalog`：每次读 SQLite 保存快照，返回 revision、`recognition=model_only`、effective/intents/slots 与能力 metadata。只序列化声明式技能数据，不返回模型路径、凭据或整份配置。`source=command/default/custom` 指定义来源：default 仅表示与当前发布种子完全相同，不能区分手工输入相同定义的历史。默认值真种入配置，显式空不补回；不造 demo 技能或样例响应。
- `GET /api/skills/capabilities`：返回真实 `skills::capabilities()`，每项含 `api/description/execution/args_schema`。这是能力注册描述，不证明设备已提供 MCP 工具或音频可投递。
- `GET /api/skill-tasks?device_id?&limit=20&offset=0`：管理者可跨设备查询；limit 1..100，offset 0..10000。`GET /api/skill-tasks/{id}?device_id?`：id 必须规范 UUID，设备过滤不匹配与不存在均 404。
- `POST /api/skill-tasks`：`{device_id,request_id?,args:{text,...schedule,timezone?}}`，时间字段在 `after/at/interval/hourly/daily/workdays/weekly/monthly/yearly` 中恰选一种，禁止额外顶层字段与 args.device_id；`request_id` 用于稳定重试幂等（建议调用者明确提供）。参数解析与容量由 SkillSystem 唯一校验；时区仅 UTC/Asia/Shanghai，通用 API 缺省 UTC、默认技能显式 Asia/Shanghai。
- 日历参数原样透传：hourly 为分钟字符串/整数 0..59，daily/workdays 为钟点，weekly 为 `{weekday,time}`，monthly 为 `{day,time}`，yearly 为 `{month,day,time}`；数值字段支持合法整数/中文数字。工作日是周一至周五、不含法定节假日推断；月31日跳过无31日的月份，年2月29日跳至下个闰年，不夹紧到月底。`at` 接受 RFC3339、今天/明天/后天/大后天加上午/下午/晚上钟点；无日期钟点取严格晚于当前时刻的最近一次。管理请求不可指定 now_ms。
- 新增成功与按 id 取消保留底层真实 `speech`，只供上层陈述实际日期/频次/结果，不在 HTTP 中生成或播放。语音按文本取消的歧义/零匹配拒绝与当前设备查询由 SkillSystem 处理；HTTP 取消仍明确指定任务 id 和 device_id，不扩成通用能力执行入口。
- `POST /api/skill-tasks/{id}/cancel`：body `{device_id}`；先验证任务绑定，返回 `{state:"ok",task_id,task_state:"cancelled",cancelled,...}`。不接受任意 capability api 或代码执行。取消不撤回已经提交的音频。
- 成功任务新增返回 `task/deduplicated/request_id/played=false/socket_written=false`；任务状态 completed 只指调度阶段完成，实际 outbox 投递另看 delivery state，均不证明扬声器播放。
- dev 模式允许真实本地 SQLite CRUD，不启动投递、不绕过真实推理禁用；本模块永不主动 tick、领取 outbox 或投递音频。生产投递另由运行期/会话负责。
- SQLite 操作在限流 4 并发的 spawn_blocking 内，10 秒超时后容量保持直到工作退出；超时可能已写入，重试需相同 request_id。输入错误 400、鉴权 401、找不到 404、幂等冲突 409、容量/并发 429、存储故障 500、超时 504。仅已明确标记的 `SkillActionError.speech` 通过 `clock_clarification` 400 返回安全澄清字段，其余原始错误不反射为 speech；SQLite 故障仍统一脱敏 500。
- 测试在[外置测试](<tests.rs>)使用回环 HTTP 与真实临时 SQLite；未触发真实远程训练、模型推理或设备音频，不能称为 NAS 已部署能力。
