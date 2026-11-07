# HTTP 与 WebSocket 网关

[路由表](<mod.rs>)是端点注册入口；鉴权与握手也在此，生产会话交给 session。管理台 UI 不维护另一套路由语义。

- 协议与 hello 见[协议说明（从上级入口导航）](<../AGENTS.md>)；下行版本跟随设备 hello，而不是服务端配置独立选版本。
- 配置 GET 打码、PUT/POST 部分更新、revision 冲突与热生效回报，见[配置端点说明](<config/AGENTS.md>)。
- 设备/会话/最近对话/用量/延迟是进程内观测态，重启清零；online 来自生命周期，不是心跳，也不是“今日统计”。长期原文留存属于 memory。
- 健康接口与插件清单共用宿主 capability 聚合，不另写 fold；探测只做本地检查，不构建引擎或请求外网。`GET /api/plugins` 另附 `ui.mode`（pure-base/full）与 `device_plugins[]` 注册投影；pure-base 只输出核心 ASR/VAD/TTS/LLM 能力，业务能力与静态 schema 不进入管理台，动态 UI 只能来自 WASM manifest。`device_plugins[]` 保留兼容设备门控键，契约由 [plugins_tests](<plugins_tests.rs>) 钉死。
- Kuikly NetworkModule 只用 GET/POST；保留配置 POST=PUT、overlay/preset 删除 POST 别名，不能只测 curl 的 PUT/DELETE。
- 人格 preview 接受未保存草稿，返回完整 effective；记忆 recall 是当场验证入口，clear 必须显式 scope 或 all。
- 固件使用[固件子路由](<../../device/firmware/http.rs>)，按功能/型号匹配；不要把统一插件 action/job 设计当成已存在路由。
- NLU 模型闭环见 [nlu_build](<nlu_build.rs>) 与 [候选诊断](<nlu_model.rs>)：构建只读取 SQLite 保存态并核对 `expected_revision`，每次生成独立版本目录和校验清单，不改生产开关。训练执行服务内嵌受信脚本，依赖镜像仅供 ML 环境；发布脚本修复需发布新服务。`POST /api/nlu/model/test` 只接受受管 `build_id`，校验产物指纹后独立真实推理，不经过规则、不会执行技能或保存配置；训练、状态、日志、候选测试均使用管理鉴权，开发模式禁止真实训练/推理。
- [技能管理 API](<skill_api.rs>)：`GET /api/skills/catalog`、`GET /api/skills/capabilities`、`GET/POST /api/skill-tasks`、`GET /api/skill-tasks/{id}`、`POST /api/skill-tasks/{id}/cancel` 全部管理鉴权；复用稳定技能系统，不接受通用能力执行/脚本入口。真实 SQLite 工作隔离、并发/分页/请求体受限，超时后台可能已提交，创建须用同一 request_id 重试。dev 允许 CRUD 不启动调度/播音；completed 只是调度状态，submitted 不是 played。详情与迁移见[默认技能清单](<../../../../docs/model-only-skills.md>)。
- **WASM 插件端点**（[plugins.rs](<plugins.rs>)，feature `wasm-plugins` 门控；未编入 = 路由不存在）：`GET /api/plugins` 追加 `wasm` 键（动态插件清单 + 扩展点解析，与静态能力清单同页合并）；`POST /api/plugins/{id}/actions/{name}` 统一动作（白名单在宿主，未声明动作 400）；`POST /api/plugins/extensions/{point}/preference` 扩展点首选（`{"implementation": id|null}`，进程内态，P3 持久化）。端点契约测试在 plugins/tests.rs（回环 HTTP + 真实夹具）。
- 服务器全局实时日志流（[log_stream.rs](<log_stream.rs>)）：tracing INFO+ 事件经采集层进环形缓冲（500 行）+ 广播，`GET /api/logs` WS 客户端先收 backlog 帧再逐行收实时帧，供管理台与 AI 边调 API 边看日志；单次训练另有聚焦流 `GET /api/nlu/model/build/log`（终态帧后服务端关闭）。
- 管理台实时推送通道（[admin_ws.rs](<admin_ws.rs>)）：`GET /api/admin/ws` 双向 WS（`/api/ws` 已被设备协议占用故独立路径，鉴权同 [`auth_ok`]，浏览器用 `?token=` 兜底）——连接即发 `hello`（EchoMuse 设备面版本号，源 `echomuse::subscribe_devices_version` 的 watch；**并内嵌 `server_status` 服务器状态快照**，首连即有数据、客户端不主动拉取），版本变化推 `echomuse_devices_changed`，服务器状态变化（出口 IP 解析完成，源 `plugins::geo::subscribe_status_version` 的 watch）推 `server_status_changed`（`status` 与 hello 快照同源同形，快照唯一出口 `plugins::geo::status_snapshot`），客户端 `ping` 回 `pong`（应用层心跳），未知/非法输入回 `error` 帧且连接保持；纯逻辑（帧构造/`reply_for`/`admin_ws_loop`）channel 直驱可测，已替代已删除的 `/api/echomuse/devices/wait` 长挂轮询；`[echomuse]` disabled 时通道照常可用（不 404）；同一通道还承载**管理 API 的 WS RPC 复用**（`{"type":"rpc","id","method","path","body"}` → `{"type":"rpc_result","id","status","body"}`，tower oneshot 打进本模块同一份路由表，业务零复制；仅 `/api/` 前缀、拒绝 `/api/admin/ws` 自身、单请求默认 30s 超时（帧可带 `timeout` 秒按请求延长，钳制上限 120s——NLU 模型测试/节点测试等长端点用））——管理台 JSON API 通讯始终走 WS（web 与 macApp），大文件传输（OTA/文件上传下载）与设备协议除外。

当前接口以路由与测试为准；管理操作见[管理台用法（从上级入口导航）](<../AGENTS.md>)。

## 上级入口

[上级 AGENTS](<../AGENTS.md>)

## 直属下级

- [config](<config/AGENTS.md>)
