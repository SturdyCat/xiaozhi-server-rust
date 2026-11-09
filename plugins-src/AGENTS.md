# 插件源根（plugins-src/）

真实 WASM 插件的源码根，契约 `xiaozhi:plugin@0.2.0`。**用户插件不编译进主系统，wasm 产物一律不入 git**——源码是唯一入库物。guest 统一经 [xiaozhi-plugin-sdk](<../server/crates/xiaozhi-plugin-sdk/AGENTS.md>)；15 个业务插件由 [tools/build-system-plugins.sh](<../tools/build-system-plugins.sh>) 按本表构建，产物落 `build/system-plugins/`（16 个 `.wasm`，含 `hello.official` 探针），交镜像/部署流水线导入，boot 扫描数据卷并恢复注册与激活状态。

⛔ **契约升级后必须重新上传**：`wasm_boot.rs::import_system_plugins_from` 只在数据卷**尚无同 id 产物**时导入系统插件，不会覆盖已存在的旧产物。0.2 是破坏性升级，旧契约产物装载校验失败，插件列表会变空；需逐个重走 `POST /api/plugins/upload/raw?id=<id>` → `register` → `activate`（同版本重传会 409，必要时升版本号）。完整纪律见 [wasm/AGENTS.md](<../plugins/wasm/AGENTS.md>)。

## 直属下级

按重构前功能拆分的官方插件，**一域一插件、一插件一扩展点**；底座不编译业务面，功能与业务的策略面一律由这些插件提供。数据面（PCM、模型推理、设备协议、HTTP/SSE、路由与后台 worker）仍是编译期原生能力，插件只补控制/策略与菜单/动作/配置字段。0.2 起插件还可声明订阅 topic、宿主代监听的 endpoint、资源配额、配置命名空间与 `provides` 服务；这些能力由 manifest 声明，宿主不解释业务语义。

- [weather](<weather/AGENTS.md>)：`skill.weather`，NMC 天气官方默认插件（P1c 试点，唯一完整 WASM 化的业务）。
- [asr](<asr/README.md>)：`asr.provider`，识别结果策略收口。
- [tts](<tts/README.md>)：`tts.provider`，合成前文本/语速/发音人有界放行。
- [llm](<llm/README.md>)：`llm.provider`，对话前用户文本放行。
- [vad](<vad/README.md>)：`vad.provider`，分段阈值有界回传。
- [tuya](<tuya/README.md>)：`connector.tuya`，涂鸦候选选择与策略编排 + 云面承接（S3c/S3d/E）+ LAN 三代协议语义（S3f–S3j：0x55AA/0x6699 帧编解码、v3.3/v3.4/v3.5 控制命令与会话协商，rustuya 黄金帧对拍）+ v3.4/v3.5 会话编排（S3m/S3n/S3o/S3p：`lan-connect` 经宿主 `tcp-connect` + `io.tcp.*` 回灌、双布局流式帧切分/心跳节拍/数据面解帧/确定性 IV 派生/指数退避自动重连/设备上报 DP 状态（S3q，解析与 native 单源），rustuya 模拟设备端到端）。
- [echomuse](<echomuse/README.md>)：`device.echomuse`，会话入口欢迎语放行。
- [esphome](<esphome/README.md>)：`connector.esp-home`，ESPHome 桥实体目录放行与改名 + 明文帧编解码 + **服务端面消息全集**（实体目录/状态/控制应答，S3r：HelloResponse、ListEntities*、*StateResponse、Number/SwitchCommand；prost 逐字节对拍）与客户端消息解码 + **Noise NNpsk0 加密握手与 AEAD 传输帧**（S3t：initiator/responder 双角色、ClientHello/ServerHello/msg1/msg2、传输帧 `type‖len‖payload`；临时密钥走宿主 `host-core.random`，握手状态在插件隔离 kv；noise-protocol 参考实现双角色逐字节 + esphome-native-api **真实加密连接**对拍，含错误 psk 的 `Handshake MAC failure` 识别）。 + **设备侧会话**（S3u：responder 消息派发与实体面应答，明文/NNpsk0 双模式，`esphome-session` 结果通道 + 状态落插件隔离 kv，真 TCP 对拍 5 passed）。
- [soul](<soul/README.md>)：`prompt.soul`，人格语气与表达预算。
- [wakeup](<wakeup/README.md>)：`wakeup.strategy`，问候目录索引选择。
- [debug](<debug/README.md>)：`debug.policy`，节点测试/日志流/测试台展示预算。
- [skill](<skill/README.md>)：`skill.policy`，技能准入与步数预算。
- [memory](<memory/README.md>)：`memory.policy`，召回准入与条数/字数预算。
- [nlu](<nlu/README.md>)：`nlu.catalog`，意图/技能候选目录筛选。
- [greeter](<greeter/README.md>)：`demo.greeter`，替换演练与链路冒烟。

`hello.official`（源在 `tools/wasm-fixtures/hello`）是拆分前包揽 14 个扩展点的系统级探针，不承担业务。**必须保持 Registered 不激活**：它 `official=true` 且声明 14 个点，激活后每个点会有两个官方实现，`pick_official` 按注册序取第一个会抢走多个扩展点的解析。

## 约定

- **每个插件目录必须自带 `README.md`**（功能描述与使用方法是插件的交付面，不是可选项）：
  必含 `## 功能描述` 与 `## 使用方法` 两节——事件/payload、动作、配置字段、kv 键布局、
  权限与扩展点、边界限制都要写清。结构由 `plugins/plugin_docs_tests.rs`
  强制检查（新增插件缺文档 = 测试失败）。测试件（tools/wasm-fixtures）不在此列。
- 插件 id 形如 `<域>.<名>`（`weather.official`），manifest 声明实现的扩展点与权限；`config_namespace` 必须等于插件 id。宿主装载期强校验见 `plugins/wasm/contract/validate.rs`（manifest / registration / endpoint / subscription）。
- 与原生实现共享纯逻辑源时用 `#[path]` 单源引用（绞杀者方向：P8 反转为原生从这里取）。
- 出站走 `effect-http-request {request-id, method, url, headers, body, timeout-ms}`，多步流程上下文编码在 `request-id`（`|` 分隔），类型化事件的 `correlation-id` 承载关联；全站级大列表不进 kv（值上限 64KB）。
