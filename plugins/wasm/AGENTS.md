# WASM 插件平台（契约 0.2.0）

继承[插件层指引](<../AGENTS.md>)。**本文件是契约 0.2 与宿主平台的当前权威说明**；[方案 v4 设计长文](<../../docs/designs/wasm-plugin-platform.md>)是 v0.1 时代的历史记录，其中的 WIT 清单、阶段表和测试计数不作现行事实。唯一契约源是 [`server/wit/plugin.wit`](<../../server/wit/plugin.wit>)，以源码与测试为准。

## 底座纯净性（用户 2026-11-08 定的硬边界）

宿主**构建期不含任何业务**，也没有「业务 feature 默认关闭」这回事：业务代码、依赖、feature 名、cfg 门控、静态注册表条目都不物理存在。宿主只提供插件依赖的**通用能力**——按字符串做通用路由、连接管理、报文/帧分帧、事件分发、沙箱、生命周期、artifact 与配置存储；不认识任何扩展点、topic、设备或协议名字。新增业务只能靠安装插件，不得改宿主或重新编译。

- `implements` / `subscriptions` / `endpoints` / `provides` 全由 manifest 声明，宿主按字符串透传与排他解析。
- 入站一律事件驱动，出站一律声明式效果（宿主执行、结果回灌）。
- 协议语义在插件侧：HTTP/WS 由宿主分帧，TCP/UDP 只搬原始字节。
- 当前仓库**尚未完全达标**：静态注册表、native 业务 feature 与 bridge 策略桥仍在，按三阶段计划剥离（阶段二/三未完成）。

## 契约 0.2.0（`xiaozhi:plugin@0.2.0`）

`world plugin { import host-core; import host-net; import host-timer; import host-system; export lifecycle }`（旧 `interface host` 单接口已废弃）。

| 面 | 内容 |
|---|---|
| 类型 | `subscription`、`protocol`、`endpoint-spec`、`resource-limits`、`config-field`、`service-method`、`service-decl`、`event{topic,correlation-id,payload}`、`effect-kv-delete`、`effect-http-request`（取代 `effect-http-get`） |
| manifest | 增 `subscriptions / endpoints / resources / config-namespace / provides`；`config_namespace` 必须等于插件 id |
| registration | 增 `services` |
| 入口 | `handle-event(ev: event)`（旧 `(event:string, payload:string)` 已废）、新增 `handle-service(service, method, payload: list<u8>)` |
| 版本门 | `CONTRACT_MAJOR_VERSION=0` / `CONTRACT_MINOR_VERSION=2`；主段或次段不符即拒装（`contract_version()`） |

`effect-lan-status` / `effect-lan-set-dp` 是 v0.1 legacy，仅保留到阶段三删除，新插件不要使用。

## 模块（文件即职责）

| 模块 | 职责 |
|---|---|
| `contract.rs` + `contract/validate.rs` | bindgen 绑定、投影 DTO、**装载期强校验**（topic 尾部通配、bind 白名单、资源上限、服务名、UI JSON） |
| `host_api.rs` | `HostContext` 聚合根（kv/config/bus/services/net/timers/fs/mdns/facts）+ `SystemFacts` 抽象；平台不反向依赖会话层 |
| `instance.rs` | 四个 Host trait 实现、`LoadedPlugin`、类型化事件与 `handle_service` |
| `registry.rs` | `WasmHost`：条目表、Runner、装配与回收、快照投影 |
| `lifecycle.rs` | Staged → Registered → Active 三态、`Phase` 子状态、boot 恢复 |
| `event_bus.rs` | topic 精确 + 尾部 `.*` 逐级前缀匹配、顶层键值过滤、有界队列、`publish_allowed`（插件只能发 `<id>.*`） |
| `services.rs` | 服务表 + 调用，深度 ≤4、自调用拒绝、多提供者备选 |
| `net/{mod,http,tcp,udp,ws}.rs` | 宿主代监听，句柄不透明 `u64`；HTTP/1.1（无 chunked，5s 应答超时→504）、WS 走 axum `ws`、TCP/UDP 原始字节；UDP 用 `try_clone` 双句柄（std 供同步出站、tokio 供收包） |
| `timers.rs` / `fs.rs` / `mdns.rs` / `config_store.rs` | 定时器（单插件 ≤64）/ `<data>/plugins/<id>/` 文件沙箱 / `ServiceDaemon` 广播（本地 IP 自包含探测）/ 插件配置命名空间与 secret 打码投影 |
| `effects.rs` / `permissions` | `log / kv-put / kv-delete / http-request` + legacy lan；权限门未声明即拒 |
| `artifacts.rs` / `sandbox.rs` / `resolver.rs` / `kv.rs` | SHA-256 钉版 + attestation / fuel·epoch·内存 / 排他扩展点 / 隔离 kv |
| `bridge*` / `waterfall.rs` | 遗留策略桥与事件瀑布，阶段二删除，⛔ 新代码不要再接 |

权限名：`kv / config / pub / services / net / timer / fs / mdns`，兼容保留 `http-out:<host> / speak / device-frames / schedule / lan-socket`。

## 并发模型（0.2 重写点）

条目表锁**只做查找/增删，绝不跨 guest 调用**。每插件一个 `Runner { instance: Arc<Mutex<LoadedPlugin>>, permissions, phase }`（单 Store 串行 = 正确粒度）+ 一条有界入站队列 + 泵任务（`spawn_blocking` 跑 wasmtime）。socket / 定时器 / 总线一律 `try_send`，依赖环上不阻塞。`arm()` 在激活期装配订阅/端点/服务，`release_entry()` 在 deactivate/remove/unload 全量回收（订阅、服务、端点、定时器、mDNS、入站队列、kv、配置）。`NioSink` / `TimerSink` 由 `InboxTable` 统一实现，避免 NetManager 与注册表互相持有；HTTP 应答走 guest 直接调 `http-respond`。

## 生命周期与运维纪律

`upload/raw` → **Staged**（只登记不装载）→ `register` → **Registered** → `activate` → **Active**；`deactivate` 回 Registered，`remove` 仅非 Active 可移除。同 id 重传要求版本严格递增，否则 409（禁用插件允许升级产物）。

⛔ **契约升级是破坏性的，远端已部署插件不会自动迁移**：`wasm_boot.rs::import_system_plugins_from` 只在数据卷**尚无同 id 产物**时导入镜像随附的系统插件，刻意不覆盖用户升级/停用状态。因此升级宿主后旧契约产物仍在数据卷里、装载校验失败，插件列表会变空。必须重新走一遍：

```bash
./tools/build-system-plugins.sh          # 重建 build/system-plugins/*.wasm（15 个）
# 逐个 POST /api/plugins/upload/raw?id=<id> → /register → /activate
```

完整纪律：改 WIT → 重建 SDK 与全部 guest（夹具 + `plugins-src/*`）→ 重建 wasm 产物 → 重新上传注册。**宿主与 guest 必须同版本**，否则 activate 直接失败。`GET /api/plugins` 验收口径：15 个插件 / 14 Active / 14 扩展点 resolved；`hello.official` 是探针，声明 13 个扩展点且 official=true，**必须保持 Registered 不激活**（否则每个点两个官方实现，`pick_official` 按注册序取第一个会抢走多个解析）。

## 未完成（勿写成已实现）

- `GET /api/plugins` 仍只投影旧 manifest 字段；0.2 新增的 `endpoints` / `services` / 事件总线统计 / 配置命名空间投影**尚未接入 HTTP 层**。
- 服务端返回 `ui.mode = "base"` / `static_business: false`，客户端 `PluginMetaState` 判断 `"pure-base"` 才隐藏静态业务导航，存在漂移。
- 阶段二：剥离 NLU/command/wakeup/soul/memory/AIUI/weather 等静态业务与 feature；阶段三：把设备业务（ESPHome → Tuya → EchoMuse）迁到插件、删 legacy lan effect。
