# emOS + EchoMuse Dot 2 对接标准化分析：ESPHome Native API / homeCore / Moli OS 可借鉴点

> 状态：分析稿（2026-10-24）。目标：回答"如何借鉴这三个项目，让 emOS + EchoMuse Dot 2 的扩展与对接更省事，**而不是继续在 xiaozhi-server 里手戳 HA 相关功能**"。
> 对齐依据：`docs/echomuse.md`（明确"不做 HA/ESPHome 转发面"）、`docs/research/mqtt-udp-transport-assessment.md`（结论"不加 broker"）、`docs/research/wyoming-provider.md`（Wyoming 作为 STT/TTS 标准协议调研）。

---

## 1. 问题定义：你现在的"手戳 HA 功能"到底是什么

xiaozhi-server 的核心定位是**语音/AI 编排**（ASR → LLM/技能 → TTS）+ **设备传输**（EchoMuse 的 `/em/*` 双 WebSocket 走 mic 16k / speaker 48k 二进制协议）。但"手戳的 HA 功能"指的是你还在服务端**手写设备控制与家居对接逻辑**，典型如 `docs/echomuse.md` 里的：

- BLE GATT 代理（`ble.rs` / `ble_http.rs` / `ble_request.rs`）：扫描、连接、读写、CCCD 通知——一整套手搓的 IoT 控制面；
- 逐设备的 `volume` / `command` / `settings` / `system/reboot` 等管理端点；
- 未来还会冒出来的"灯光/开关/传感器/场景/自动化"等家居实体与规则。

这些本质上就是 **Home Assistant 在做的事**，只是你把它重写了一遍、且只服务于 EchoMuse 这一族设备。`docs/echomuse.md` 已经主动划了线："无 HA/ESPHome 转发面（那是官方控制器的用法）"——即**服务端不打算当 HA 的桥**。问题是：不手戳、又不当桥，那设备/家居集成怎么办？答案就是下面三个项目分别提供的"标准件"。

---

## 2. 三个项目速写（对照表）

| 维度 | **ESPHome Native API** | **homeCore** | **Moli OS** |
|---|---|---|---|
| 是什么 | **设备侧协议**（protobuf over TCP/6053，mDNS `_esphomelib._tcp`，noise_psk 加密，实体模型：sensor/switch/light/climate/cover/fan/binary_sensor/camera + 自定义 `user_service`） | **本地优先自动化中枢**（Rust/Tokio + redb，内嵌 rumqttd，REST+WS API，规则引擎 + 场景 + 模式，插件 SDK Rust/Python/Node/.NET，签名插件市场）⚠️ **无 ESPHome 原生集成**（支持 Z-Wave/Hue/Lutron/WLED/Sonos 等插件桥接，ESPHome 不在列） | **agent-native 智能家居 OS**（Rust，单二进制 15.9 MiB RSS，MCP streamable HTTP + REST + SSE，24 个集成，内置带"护栏"的助手）🔒 **仅借鉴、不集成**（见 §4.3） |
| 角色 | 设备 ↔ 控制端的**契约** | 家居状态的**真相源 + 规则/场景引擎**（接 ESPHome 设备需自写插件） | 给**智能体（语音助手）**用的家居控制面（参考范式） |
| 生态成熟度 | 极高（HA 官方采用；Rust crate `esphome_native_api`=ubihome，client+server+加密；同名 `Nafaya` 库是 JS/npm，**非 Rust**） | 中（新但架构完整，有 hc-mcp 客户端） | 新（社区小，但定位精准） |
| 与你栈的契合 | 协议可移植；Rust 客户端现成 | Rust/Tokio/单二进制，契合度最高 | Rust/单二进制，契合度高 |
| 体积/代价 | 仅协议，无运行时代价 | 第二个常驻服务 + redb/SQLite 数据卷 + 自写 ESPHome 插件的成本 | 不部署（仅取设计思路） |

**关键事实（已核实，2026-10-24 复核）**：
- ESPHome Native API 的 **Rust** 实现 = `ubihome/esphome-native-api`（crate 名 `esphome_native_api`）：MIT、v3.0.0（23 个稳定版）、~258k SLoC、tokio 异步、**同时支持 Server + Client + noise_psk 加密**、proto 按 feature 固定版本（`version_2025_12_1` / `version_2026_6_2` 等），并被 `ubihome/ubihome` 实际用于"把跑 OS 的设备暴露给 Home Assistant"。它的 `EspHomeServer` 是**带实体管理的服务端抽象**（`add_entity(id, Entity::BinarySensor(...))` 自动算 FNV-1 key、管理实体注册与消息路由）——这正是把 Dot 2 呈现成标准 ESPHome 节点的关键部件。
- 同名库分清：`Nafaya/esphome-native-api` 其实是 **npm / JavaScript** 库（`npm i esphome-native-api`，带 `Client`/`Discovery`/`Connection` 实体类），**不是 Rust**；只能作参考客户端 / mac 管理台工具，**不进 Rust 服务端栈**。
- 即"设备端（Server）和客户端（Client）都能用 Rust 写"，对你栈唯一可直接落地的 Rust 选择就是 ubihome 这个 crate。
- homeCore 架构里客户端列表含 `hc-mcp`、语音助手——即它天然支持被智能体通过 MCP 驱动；**但关键约束（2026-10-24 核实）：homeCore 没有 ESPHome 原生集成**，其插件（hc-hue/hc-zwave/hc-wled/hc-sonos…）均经 MQTT 桥接自有协议，ESPHome 不在支持列表。因此"我们的 `EspHomeServer` 桥 ↔ homeCore"不能无缝对接，必须额外写一个 homeCore 插件（`hc-plugin-sdk-rs`）把 ESPHome 实体翻成 homeCore 的 MQTT fabric——这是集成 homeCore 的硬成本（详见 §4.2）。
- Moli OS 把 MCP/REST/SSE 作为"一个端口三种表面"暴露，且"护栏（guard）"会先把智能体命令放进受保护房间等待人类确认——这正是语音助手做高风险操作（锁门/关阀）所需的安全机制。

---

## 3. 核心论点：把系统拆成三层，每层用标准件

```
┌─────────────────────────────────────────────────────────────┐
│  Layer 3  自动化中枢（可选：homeCore 按需；Moli 仅借鉴不部署）  │
│  实体注册表 + 状态 + 规则/场景 + 集成（Hue/Sonos/Tuya/...）    │
│  暴露 MCP / REST / WS 给语音助手                              │
└───────────────────────────▲─────────────────────────────────┘
                            │ MCP / REST（只传"意图+结果"）
┌───────────────────────────┴─────────────────────────────────┐
│  Layer 2  xiaozhi-server（纯语音/AI 编排 + 可选 EspHomeServer 桥）│
│  ASR→LLM→TTS→技能；设备传输（音频走 WS 二进制）               │
│  可选模块：同进程起 EspHomeServer，把 Dot 2 实体暴露成标准节点  │
└───────────────────────────▲─────────────────────────────────┘
                            │ ESPHome Native API（实体/控制面）
┌───────────────────────────┴─────────────────────────────────┐
│  Layer 1  emOS / EchoMuse Dot 2（设备）                       │
│  语音卫星（mic/speaker/LED/按键/唤醒）+ 通过 BLE 代理的 IoT   │
│  以"ESPHome 实体"形态暴露自身与所控外设                        │
└─────────────────────────────────────────────────────────────┘
```

要点：**音频永远走你现有的 WebSocket 二进制通道**（`docs/echomuse.md` 的 16k/48k 协议），**不做成 MQTT/UDP**（你的 `mqtt-udp-transport-assessment` 已判定"不加 broker"）。标准协议只接管"实体/控制/自动化"这一层，不碰语音流。

> **关于"谁来说 native-api"**：emOS Dot 2 固件是自定义实现，把 Server 下沉到固件是重改动；更现实的路径是**在 xiaozhi-server 宿主机进程内起 `EspHomeServer` 充当桥**，把 Dot 2 自身 + BLE 外设映射为实体——零固件改动，且桥做成**独立可选模块**，核心语音路径零改动（详见 4.1）。这与 `docs/echomuse.md` 的"不做 HA 转发面"不冲突：桥只把 **emOS/EchoMuse 自身可控面** 以标准实体自描述出去，不做"把 HA 命令透传到任意第三方设备"的通用转发。

---

## 4. 三个集成点的具体借鉴方案

### 4.1 ESPHome Native API → 做"设备契约"，消灭手搓控制面

**借鉴点**：用 ESPHome Native API 取代你在 EchoMuse 里手写的 per-device 控制端点。

- **最现实的落地：宿主机侧 `EspHomeServer` 桥接，而不是等固件改**。emOS Dot 2 跑的是自定义固件，把 native-api Server 下沉到固件是重改动；但 xiaozhi-server 本身跑在 Linux 宿主机（N5105）上，完全可以在**同一个进程内起一个 `EspHomeServer`（来自 `esphome_native_api`）监听 6053**，把 Dot 2 自身 + 它经 BLE 代理所控的外设，映射成标准 ESPHome 实体暴露出去。HA / homeCore / Moli / 任意 native-api 客户端连上来，看到的就是一个普通 ESPHome 节点——**零固件改动、零 HA 专用代码**。
  - Dot 2 自身实体：`sensor`（mic 电平 / 在线时长 / CPU）、`number`/`switch`（speaker 音量、LED、播放/暂停）、`button`（动作键 / 打断）、`binary_sensor`（唤醒态 / 聆听态 / 播报态）；
  - **Dot 2 作为 BLE 代理（关键未来需求）**：Dot 2 经其无线电代理 nearby 蓝牙设备（如小米蓝牙灯 / 开关 / 传感器 / 锁）。桥须实现 ESPHome 的 `bluetooth_proxy` 消息族（`BluetoothDeviceRequest` / `BluetoothGATTRead·Write·NotifyRequest` / `BluetoothConnectionsFreeRequest`…），让任意 native-api 客户端（含未来的小米设备类适配器）复用 Dot 2 的无线电；同时把被代理的小米设备**注册为本桥内的标准子实体**（`switch`/`light`/`lock`/`sensor`），与 Dot 2 自身实体同处一个 ESPHome 节点——**这就是把 `ble.rs` 那套 GATT 控制面换成标准实体模型**，命令进来经桥翻译成 xiaozhi-server 内部动作，状态变更经桥回推。详见 §4.1.2。
  - 自定义语音指令：native-api 的 `user_service`（任意参数）下发"会话开始/结束/注入文本"，不污染实体模型；`subscribe_logs` / `camera_image` 现成可用。
- **加密与发现**：noise_psk（ChaCha20Poly1305 / X25519 / SHA-256，就是 ESPHome 的 noise_psk）+ mDNS 发现，免手写 token/握手；`docs/echomuse.md` 的 `token` 校验可降级为 native-api 的 psk。
- **Rust 落地**：服务端直接用 `esphome_native_api` 的 `EspHomeServer`（实体管理抽象：`add_entity(id, Entity::BinarySensor(...))` 自动算 FNV-1 key）+ `EspHomeApi`（客户端，需主动查询别家设备时）。**npm 的 `esphome-native-api`(Nafaya) 仅作参考/管理台，不进服务端**。
- **⚠️ 实测约束（2026-10-24 核实 docs.rs latest）**：本 crate 的 `EspHomeServer` **目前只实现 `BinarySensor` 一种服务端实体**——`Entity` 枚举仅 `BinarySensor` 变体，switch/number/light/sensor 等均标注"待扩展"。即开箱只能把 Dot 2 的**状态**（聆听/播报/在线/唤醒）暴露成二进制传感器；**音量/静音/LED 这类"可控"实体尚不能直接 `add_entity` 注册**。两条出路：
  1. **状态先行（先落地）**：用 `EspHomeServer` + `binary_sensor` 先把 Dot 2 的聆听/播报/在线态暴露出去，验证"发现 + 加密 + 状态回推"闭环（阶段 A PoC 即此）；
  2. **控制面自建**：要真正控音量/静音/LED，需用 crate 的**低层 `esphomeapi::EspHomeApi` + `proto` 模块**（握手/加密/全量 protobuf 消息已生成）自行实现实体列表与命令分发（`ListEntitiesSwitchResponse` / `SwitchCommandRequest` 等），相当于在 crate 之上补一层"富实体服务端"——比直接用 `EspHomeServer` 工作量大，但协议与加密仍由 crate 托管，不重写。
- **参考实现**：`ubihome/ubihome` 就是"用这个 crate 把 Linux 盒子变成 ESPHome 节点的 HA 边缘代理"的现成范例（`ubihome-api` 组件基于 `EspHomeServer`，配 YAML 即可把 shell 传感器/按钮暴露给 HA），其 `EspHomeServer` + 实体注册模式可直接照抄（注意它当前能暴露的也是 binary_sensor 类状态）。

**收益**：你不再为"新设备类型"加端点；任何 ESPHome 生态客户端开箱即用。这**不违反** `docs/echomuse.md` 的"不做 HA 转发面"——桥只把 **emOS/EchoMuse 自身可控面** 以标准实体自描述出去，不做"把 HA 命令透传到任意第三方设备"的通用转发；且桥是独立可选模块，核心语音路径零改动。

### 4.1.1 阶段 B 控制面（低层自建）设计：在 `esphomeapi` + `proto` 之上补"富实体服务端"

> 决策（2026-10-24）：阶段 B 走**低层自建**——`EspHomeServer` 当前仅支持 binary_sensor，可控实体（音量/静音/LED/打断）须用 crate 的 `esphomeapi::EspHomeApi` + `proto` 自行实现。协议握手与 noise_psk 加密仍由 crate 托管，**不重写协议**。

**模块落点**：`server/src/device/esp_home/`（device 域，与 `echomuse` 并列），feature 名 `esp_home`，配置段 `[esp_home]`。

> 📌 本节是**动手前的设计规格**；实际实现与它有偏差（未拆 `entities.rs`、`actions.rs` 是自由函数而非 trait、`admin.rs` 里其实没有 volume/mute 动作）。**以 §4.1.1.1（阶段 A）与 §4.1.1.2（阶段 B）的实施记录为准**。

- `config.rs`：`EspHomeConfig { enabled, port(=6053), encryption_key(base64), device_name, friendly_name, suggested_area }`；
- `server.rs`：TCP accept 循环 + 每连接 `EspHomeApi` 会话状态机（低层自建核心）；
- `entities.rs`：实体定义表（object_id → 类型/device_class/单位）+ FNV-1 key 生成（用 crate 的 `hash` 模块）；
- `actions.rs`：翻译缝 → 真正入口是 `device/echomuse/{mod.rs 的 hub_ctl/hub_live}` 与 `system.rs::volume_command`（不是 `admin.rs`）；
- `state.rs`：`Arc<DeviceState>`（聆听/播报/在线/音量/静音…）+ `broadcast::channel` 供状态变更回推。

**协议状态机（每条 native-api 连接）**：
1. `HelloRequest` → `HelloResponse`（name / api_version）；
2. `ConnectRequest`(password) → `ConnectResponse`；
3. `DeviceInfoRequest` → `DeviceInfoResponse`（mac / model / manufacturer / friendly_name）；
4. `ListEntitiesRequest` → 逐条 `ListEntities{Switch,Number,Light,Button,BinarySensor}Response`（带 FNV-1 `key`）+ `ListEntitiesDoneResponse`；
5. `SubscribeStatesRequest` → 各实体当前 `*StateResponse`；
6. `SwitchCommandRequest` / `NumberCommandRequest` / `LightCommandRequest` / `ButtonCommandRequest` → 经 `EspHomeDeviceActions` 翻译为内部动作，并回推新状态；
7. `PingRequest` → `PongResponse`；`GetTimeRequest` → `GetTimeResponse`。
（消息类型与字段名以 `proto` 模块生成为准，实现前对照 docs.rs `esphome_native_api::proto` 逐项核对。桥还须实现 `bluetooth_proxy` 消息族——`BluetoothDeviceRequest` / `BluetoothGATTRead·Write·NotifyRequest` / `BluetoothConnectionsFreeRequest` 等，这是 Dot 2 作为 BLE 代理的硬性要求，详见 §4.1.2。）

**状态回推**：Dot 2 内部状态变化（聆听/播报/在线、音量/静音）经 `state.rs` 的 broadcast 通道，server 任务订阅后发对应 `*StateResponse` 给所有连接。

**加密**：`EspHomeApi` builder 注入 `encryption_key`（noise_psk，base64）；空串 = 明文（仅局域网可信环境）。

**配置段（`[esp_home]`，默认 `enabled=false`，独立可选）**：
```toml
[esp_home]
enabled = false
port = 6053
encryption_key = ""        # base64 noise_psk；空 = 明文
device_name = "xiaozhi-dot2"
friendly_name = "EchoMuse Dot 2"
suggested_area = "客厅"
```
> ⚠️ 若日后要纳入"标准行为默认开启"，须在 `config/defaults.rs` 的 `BATCHES` 增加 esp_home 对齐项（与 `echomuse` 同机制），否则存量库读不到新默认。

#### 4.1.1.1 阶段 A 实施记录（已落地，含实测踩坑）

阶段 A 已按上述规格落地并跑通真实协议往返（`cargo test --features esp_home`）。实现与规格的偏差、以及四条**必须记住的协议事实**：

**实际文件布局**（比规格更薄：阶段 A 只有状态，没有实体动作，故不需要 `entities.rs` / `actions.rs`）：
- `config.rs`：`EspHomeConfig`（比规格多 `model` / `manufacturer` / `mdns` 三个字段）；
- `server.rs`：accept 循环 + 每连接应答 + 状态回推 + mDNS 广播 + 协议往返测试；
- `state.rs`：4 个 `binary_sensor` 的静态表 + `AtomicBool` 快照 + `broadcast` 变更通道（`entities.rs` 的职责并入此处，阶段 B 加控制面时再拆出 `entities.rs` / `actions.rs`）；
- `mod.rs`：模块声明与 `spawn(cfg, registry)` 入口。

**实体表（4 个，与 `registry::Phase` 一一对应）**：`online`(connectivity) / `listening`(running) / `thinking`(running) / `speaking`(running)。
> 规格里的"已唤醒"**已删除**：`Phase` 没有唤醒态（唤醒后立即进 `Listening`），硬造一个恒为 false 的实体只会在 HA 里变成噪声。四路信号全部只读观测 `device::registry` 的活会话表——**不新建第二份会漂移的状态**。

**实测踩坑（阶段 B 同样适用）**：
1. **服务端必须先 `peek` 首字节再 `start`**。`EspHomeApi::start` 是按"客户端主动连服务端"写的：握手第一步就 `fill_buf()` 并要求已有数据，否则立刻 `HandshakeError::NoData` 断开。服务端 accept 后对端可能一个字节都没发，直接 `start` 会 100% 断连。`peek` 不消费数据（字节留在内核缓冲，`fill_buf` 仍读得到），返回 0 表示对端已断。
2. **`BinarySensorStateResponse.key` 是 `fixed32`，不是 varint**（ESPHome 为让状态包更小改的）。线上是 `0D <4 字节小端>`；按 varint 解会直接解析失败。
3. **protobuf 会省略默认值**：`key=0` / `state=false` 在线上"不出现"，故 `key=0` 的状态帧载荷是**空字节串**。任何"取前 N 字节"的判断都是错的，必须按字段号解析。
4. **`HelloRequest` 字段号是 `client_info=1` / `api_version_major=2` / `api_version_minor=3`**（`client_info` 在 1 号位，容易按直觉写反）。

**验证方式**：`crate` 内部用 `log` 打协议日志（主程序用 `tracing`），故测试里加了 dev-only 的 `env_logger`——否则内部读取循环的终止原因完全静默，只能看到客户端侧莫名的 `UnexpectedEof`。测试本身用裸 TCP + 手写帧扮演 ESPHome 客户端，覆盖 Hello → DeviceInfo → 实体列表 → 订阅快照 → 状态实时回推。

#### 4.1.1.2 阶段 B 实施记录（控制面已落地：音量 `number`）

阶段 B 的控制面已落地（`cargo test --features esp_home` 全绿），但**范围必须按协议的真实能力裁剪**——这是本轮最重要的发现：

**实测能力边界（决定了什么能做、什么不能做）**：

| 意图 | EchoMuse 协议现状 | 落地形态 |
|---|---|---|
| 音量 | ✅ 下发 `{"type":"volume_set","level":0..127}`；上报 `volume_state{level}` | **可写 `number`（0..127，步长 1）** — 已实现 |
| 静音 | ⚠️ **只有上报 `mute_state{muted}`，没有 `mute_set`** | **只读 `binary_sensor`** — 不做假开关 |
| LED | ❌ 除注释外无任何下发实现 | 不暴露 |
| 打断 | ❌ 服务端打断是内部 `Session.abort` 标志，不是下发命令 | 不暴露 |

> 结论：**不要为了"控制面看起来完整"去造 `switch`/`light` 实体**。HA 里点了没反应的控件比没有控件更糟——这与阶段 A 删掉"已唤醒"是同一条原则。

**实际文件布局（阶段 B 新增 `actions.rs`，仍不拆 `entities.rs`）**：
- `state.rs`：实体表升级为 `EntityKind::{Binary, Number}` 的**统一表**（索引即 ESPHome `key`），状态存储拆成 `AtomicBool[BINARY_COUNT]` + `AtomicU32[NUMBER_COUNT]`（存 `f32::to_bits`），变更仍旧走同一条 `broadcast<usize>`；
- `actions.rs`（新）：**翻译缝**——`set_volume(level)` → `hub_ctl(id)` 发 `ControlOut::Json(volume_set)`；`telemetry()` 读 `hub_live(id).payload()` 的 `volume_state`/`mute_state`；`quantize_volume(f32) -> u8` 四舍五入并夹紧 0..127（HA 滑块会发 `0.4`/`126.6`/负数/NaN）；
- `server.rs`：实体列表按 `EntityKind` 分派 `ListEntitiesBinarySensorResponse` / `ListEntitiesNumberResponse`；新增 `NumberCommandRequest` 分支；探针同时读 `registry`（会话态）与 `echomuse`（设备遥测）。

**新增协议事实（与 4.1.1.1 的四条并列）**：
5. `ListEntitiesNumberResponse` = **49**，`NumberStateResponse` = **50**，`NumberCommandRequest` = **51**；`key` 同样是 `fixed32`。
6. 音量的 `min_value=0.0` 属默认值，**线上被省略**——客户端必须按"缺省即 0"处理，不能要求字段存在。
7. **命令下发后不要立即回推状态**。设备会自己上报 `volume_state`，桥由 2s 探针读真实值再推；提前把命令值回声给客户端，会在设备拒绝/截断时把 HA 滑块弹回错值。代价是最多 2s 的滑块"回弹延迟"，换来状态永远是设备真实值。
8. 遥测字段缺失（设备还没上报过）时**保持上次值、不写 0**，否则 HA 里会出现"音量突然归零"的假变化。

**`online` 语义修正**：阶段 A 用 `registry.sessions()` 判定在线，但那实际是**语音会话**表——设备连着聊完天后会变成"离线"。阶段 B 改为 `echomuse::hub_status()` 非空（= 与服务器保持着 control 连接），这才是 HA 用户理解的"在线"。`listening`/`thinking`/`speaking` 仍取 `registry::Phase`。

**多设备**：桥只代理 hub 里第一台设备（Dot 2 是唯一设备）。多台并存需要"多实例或多设备配置"，当前明确不做（见 §7）。

**测试覆盖**：`plaintext_roundtrip_and_state_push` 扩到 6 个实体（5 binary + 1 number），覆盖实体定义的类型参数（min/max/step）、数值状态回推、以及 `NumberCommandRequest` 后连接仍正常应答；`number_command_only_targets_volume` 验证命令只落到音量实体、其它 key 一律忽略；`quantize_volume_rounds_and_clamps` 覆盖边界值。

### 4.1.2 Dot 2 作为 BLE 代理接入小米设备（一等未来需求，强化桥的必要性）

**需求（用户明确，2026-10-24）**：当前仅 Dot 2 一台设备；后续必让 Dot 2 当 BLE 代理，控制 / 读取小米蓝牙设备（灯 / 开关 / 传感器 / 锁等）。这恰好是 ESPHome `bluetooth_proxy` + 标准实体模型的杀手级场景——它把"手搓小米对接"从"为每个小米设备写端点"变成"写一个小米设备类适配器 + 复用代理无线电"。

**两种模型（选其一落地）：**
- **模型 A — 桥内运行小米 BLE 适配器（推荐，零重 hub）**：在 xiaozhi-server 的 `esp_home` 模块内跑一个小米 BLE 客户端，复用 Dot 2 的无线电（经现有 `/em/*` WS GATT 通道，或经桥自身的 `bluetooth_proxy` 消息族）。发现小米设备后，把每类小米设备**注册为本桥内的标准 ESPHome 子实体**（`switch`/`light`/`lock`/`sensor`…），与 Dot 2 自身实体同处一个节点。语音助手经统一实体模型驱 Dot 2 + 小米，无需手搓 per-device 端点。**扩展 = 加一个"小米设备类适配器"**（解析某类小米 GATT / MiOT），不是加端点。
- **模型 B — 外接 hub 跑小米逻辑（HA 零代码，但被排除）**：HA 原生支持 ESPHome `bluetooth_proxy` + 小米集成（Xiaomi Miio / passive BLE），连上桥即可用 Dot 2 的无线电控小米。但 `docs/echomuse.md` 已排除 HA（2.1 GiB、不做 HA 转发面），故不采用；仅作"若未来放宽 HA 限制"的备选。

**对桥的硬性要求（已落地，见 §4.1.3）：**
- `bluetooth_proxy` 消息族已实现（`Subscribe/UnsubscribeBluetoothLeAdvertisements`、`BluetoothDeviceRequest`、`Gatt GetServices/Read/Write/Notify`、`ConnectionsFree`），见 `server/src/device/esp_home/bluetooth.rs`。
- 设备侧能力**不等同** ESPHome 全集：配对 / 清缓存 / 描述符读写设备没有实现，桥一律明回错误、并**不宣告**对应 feature 位（§7 不谎报能力）。
- 实体表**动态注册**（小米设备上线即注册子实体、离线即摘牌）**尚未做**：它依赖"哪一类小米设备"的确定答案（MiBeacon `0xFE95` 广播型 vs MiOT 连接型，解析与鉴权完全不同），没有具体型号与真机时写出来的只是猜的假实体——按 §7 的原则，宁缺不假。

**为什么这强化"不碰 homeCore / 不部署 Moli"的取向**：小米 BLE 协议（MiOT / 各厂商 GATT）必须有人实现。若交给 homeCore，需再写 `hc-xiaomi-ble` 插件；而本方案让小米适配器跑在桥内、以标准实体暴露，**homeCore（若日后采用）只需一个 `hc-esphome` 插件就能把 Dot 2 + 小米一次性全覆盖**——集成成本被桥摊薄。即：桥是所有设备（Dot 2 自身 + 小米 + 未来）的单一标准出口，hub 只是可选的消费方。

### 4.1.3 `bluetooth_proxy` 实施记录（2026-10-24 落地，含实测约束）

**模块**：`server/src/device/esp_home/bluetooth.rs`（约 700 行）；设备访问经 `echomuse/ble.rs` 新增的 crate 内门面 `gatt_call` / `read_snapshot`（桥**不复制**一套 hub+BleLink 交互，复用 HTTP 路由那套审批/会话/数据链校验）。

**设备侧真实能力（`bridge.go` + `ble_request.rs` 实测，勿乐观外推）**：

| 能力 | 设备 | 桥的处理 |
|---|---|---|
| `connect` / `disconnect` | ✅ | `BluetoothDeviceRequest` → 对应 `t` |
| `services` / `read` / `write` | ✅ | 一一映射 |
| `slots`（连接槽） | ✅ 设备主动上报 | 读快照缓存（不主动查，避免排 35s 队列） |
| `notify` | ⚠️ 设备**自动**推送，无"按 handle 订阅"动作 | 记下客户端要的 handle，从事件流里过滤转发 |
| `pair` / `unpair` / `clear_cache` | ❌ | 回 `error=-1` |
| 描述符读写 | ❌ | 回 `error=-1` |

**四条必须记住的协议事实**：
1. **`bluetooth_proxy_feature_flags` 才是现行判据**：aioesphomeapi 在 `api_version >= 1.9` 时只读 `bluetooth_proxy_feature_flags`，`legacy_bluetooth_proxy_version` 仅旧客户端兜底。故 `api_version_minor(10)` 下必须设前者，只设后者等于没宣告。
2. **能力位只报真实具备的**（`PASSIVE_SCAN|ACTIVE_CONNECTIONS = 0b011`）。谎报 `PAIRING`(8) / `CACHE_CLEARING`(16) 会让客户端把必然失败的按钮摆到用户面前——与阶段 A 删掉"已唤醒"、§7 不做假开关是同一条原则。
3. **`BluetoothGattCharacteristic.handle` 必须取设备侧 `value_handle`**，不是声明 `handle`。取错会让后续 read/write 全打到错误属性上（这是个静默错误，不会报错只会读到垃圾）。
4. **`address` 是 MAC 的 6 字节按大端放进 u64**（`aa:bb:cc:dd:ee:ff` → `0xaabbccddeeff`），不是把字符串哈希；UUID 的 `uuid` 字段是 128-bit 拆成两个 u64、各按**大端**（客户端按 `x.to_bytes(8,"big")` 还原）。

**广播的语义降级（必须知道）**：设备给的是**按 addr 合并后的最新快照**（`adverts: HashMap<addr, entry>`），不是逐包事件流。桥按 `(addr, data)` 去重后上推，避免设备周期重报同一批广播把客户端刷爆；代价是 RSSI 单独变化不触发上报。

**架构决定：BLE 代理跑独立任务**。一次 `connect` 最长等 35s（`GATT_WAIT`），若在连接主循环的 `select!` 里 await，整个连接期间连实体状态回推都会停摆。故主循环只做 `bluetooth::is_bluetooth(&msg)` 判定 + 转发，慢操作全在 worker 里串行。

**为小米适配器铺好的地基**：`parse_ad()` 已完整解析 BLE AD structures，其中 `0xFF` 厂商数据（company id `0xFE95` = 小米）与 `0x16` 服务数据都是**解析后**的 `(id, payload)`——小米 MiBeacon 的接入点就在这里，不需要再写一遍广播解析。小米适配器的剩余工作只有两件：① 认领具体设备型号；② 把 MiBeacon/MiOT 的属性映射成实体并订阅变更。

### 4.2 homeCore → 是否有帮助、怎么集成/借鉴（HA 的轻量替代品，但无 ESPHome 原生集成）

**结论先说**：homeCore 对你"**跨品牌家居中枢 + 规则引擎 + 仪表盘**"这个需求有帮助，是 HA 的好替身；但它**不是 Dot 2 的必需品**，且**不能原生消费我们的 `EspHomeServer` 桥**——要接得自写插件。因此定位为：**可选的第二层（仅当你真要管 Dot 2 之外的家居时才上），平时只借鉴它的设计思路**。

**是否对项目有帮助？**
- ✅ **有帮助的场景**：你未来要管 Dot 2 之外的家居设备（灯光/开关/窗帘/温控等跨品牌），要规则自动化、场景、给非语音用户用的仪表盘——homeCore 的 Rust/Tokio + redb + 规则引擎（22 触发 / 16 条件 / 40 动作 + Rhai）+ 插件市场正好承接，且单二进制本地优先，契合你栈（HA 2.1 GiB 出局）。
- ⚠️ **对 Dot 2 本身无直接帮助**：Dot 2 经我们的 `EspHomeServer` 桥已是标准 ESPHome 节点，任何原生 API 客户端都能驱，**不依赖 homeCore**。homeCore 的价值在于"把 Dot 2 与其它家居统一进一个真相源 + 加自动化"，而非让 Dot 2 能被控制。
- ❌ **关键缺口**：homeCore **无 ESPHome 原生集成**（核实 2026-10-24：其插件支持 Z-Wave/Hue/Lutron/WLED/Sonos/… 经 MQTT 桥接，ESPHome 不在列表）。即"桥 ↔ homeCore"不能即插即用。

**如何集成（若采用，需自写插件）：**
1. 部署 homeCore 第二容器（rumqttd + redb + REST/WS/MCP），本地优先无云；
2. 写 `hc-esphome` 插件（Rust，`hc-plugin-sdk-rs`）：插件内部作为 ESPHome **客户端**连我们的 `EspHomeServer` 桥（或把桥直接以 homeCore 插件形态运行），把发现的实体注册进 homeCore 的 MQTT fabric，命令双向翻译（homeCore → 桥 → xiaozhi 内部动作；状态反向回推）；
3. xiaozhi 经 `hc-mcp`（MCP 服务器）驱动家居，或 xiaozhi 直接经 ESPHome 桥驱 Dot 2、homeCore 只管"其它家居"。
- 集成成本 = 一个 homeCore 插件（hc-plugin-sdk-rs）+ 一个常驻服务 + 数据卷运维。属于"值得但非必需"的投资，且依赖阶段 A/B 的桥先落地。
- **小米设备的摊薄效应**：因为桥会把 Dot 2 + 小米（经 §4.1.2 适配器）汇成单一 ESPHome 节点，homeCore 若日后采用，只需一个 `hc-esphome` 插件即可一次性覆盖全部设备，无需另写 `hc-xiaomi-ble`——集成成本被桥显著降低。这进一步说明：hub 只是桥的可选消费方，桥才是所有设备的标准出口。

**如何借鉴（不集成时也能取的设计）：**
- **插件 SDK + 隔离进程 + MQTT fabric 的扩展模型** → 我们的 `EspHomeServer` 桥本就做成隔离可选模块（feature `esp_home`），与此同构；
- **规则引擎 / 词汇表（`vocabulary` 统一属性名与单位）** → 若未来要自动化，用轻量 Rhai 式脚本 + 统一属性词汇表，而非手搓 if/else；
- **API-first（REST + WS + MCP 三表面）暴露形态** → 若未来 xiaozhi 要对外暴露设备 API，照此统一端口 / 多客户端。

### 4.3 Moli OS → 🔒 仅借鉴、不集成（agent-native 控制面的参考范式）

**明确边界**：Moli OS **不集成**到本项目——不部署 Moli 实例、不接它的 24 个集成、不把它当运行时的家居中枢。它在这里的唯一价值是**设计思路参考**，因为你的核心诉求（语音助手本身是家居主要操作者）与 Moli 生而为一。

**可借鉴的点（落到 xiaozhi 技能层 / 桥层，而非引入 Moli）：**
- **护栏（guard）机制**：智能体命令先入"受保护房间"等人类确认——直接借鉴进 xiaozhi 的"控制家居"技能：锁门 / 关阀 / 撤防等高风险意图先经确认（护栏 / 角色 ACL）再执行。这是语音助手做物理世界操作的安全底线（见阶段 D）。
- **agent-native：MCP 一等公民 + 单端口三表面（MCP/REST/SSE）**：印证你既有方向——xiaozhi 技能本就以 MCP 工具驱动家居；若未来要暴露 API，统一端口多表面。
- **单二进制极简 footprint（15.9 MiB RSS）**：印证"轻量本地优先"偏好，桥与可选模块都应保持小、零云。
- **不借鉴**：Moli 的整套 OS / 集成运行时、助手实现、自动化编辑器——这些是它的产品，不是你的需求。

---

## 5. 决策建议（直接给结论）

1. **不要嵌入 Home Assistant。** 2.1 GiB、非你栈、且 `docs/echomuse.md` 已明令"不做 HA 转发面"。
2. **必做：用 ESPHome Native API 做"设备契约层"。** 在 xiaozhi-server 宿主机起独立可选 `EspHomeServer` 桥（§4.1 / §4.1.1），把 emOS/EchoMuse Dot 2 自身 + BLE 外设标准化成 ESPHome 实体，并**实现 `bluetooth_proxy` 让 Dot 2 当 BLE 代理控小米**（§4.1.2，小米设备经适配器注册为本桥标准子实体）——这一步**独立就能消灭手搓 BLE/音量/指令端点**，任何 ESPHome 客户端（HA / homeCore / 自研）都能驱，零固件改动。这是整个方案的核心交付，**不依赖任何第二服务**；扩展小米 = 加一个设备类适配器，不是加端点。
3. **homeCore：可选、且需自写插件，不强行上。** 仅当你要管 Dot 2 之外的跨品牌家居、要规则 / 场景 / 仪表盘时才集成（§4.2 的 `hc-esphome` 插件路径）；否则只借鉴其设计（隔离插件模型 / 规则引擎 / API-first）。它的价值是"统一其它家居 + 自动化"，不是让 Dot 2 可被控制。
4. **Moli OS：仅借鉴、不集成。** 取其护栏（高风险确认）、agent-native MCP、轻量 footprint 思路进技能层与桥层；不部署、不接其集成（§4.3）。
5. **停止手搓，转为桥接**：xiaozhi-server 只保留语音/AI + 设备传输；家居实体 / 规则 / 跨品牌接入交给（可选的）homeCore 或原生 API 客户端；助手经 MCP / 原生 API 发意图。扩展成本从"在核心里加端点"降为"写一份设备 profile（+ 可选一份 hub 插件）"。

---

## 6. 分阶段落地（验证前置、分层推进；A/B 必做，C 按需，D 取 Moli 思路）

- **阶段 A — 状态暴露（✅ 已落地，见 §4.1.1.1）**：低层 `EspHomeApi` + `proto` 自建（`EspHomeServer` v3.0.0 是空壳），监听 6053，暴露 `binary_sensor`（在线 / 聆听 / 思考 / 播报），noise_psk 加密 + mDNS，状态实时回推。已用裸 TCP 手写帧跑通真实协议往返。
- **阶段 B — 控制面 + BLE 代理（✅ 已落地，见 §4.1.1.2 / §4.1.3）**：
  - ✅ **音量 `number`（0..127）**：命令 → `volume_set` 下发，状态 ← 设备 `volume_state` 上报；静音因协议无 `mute_set` 只做只读 `binary_sensor`。
  - ✅ **`bluetooth_proxy` 消息族**：广播订阅/上推、连接/断连、服务发现、读、写、notify 转发、连接槽；能力位只报真实具备的 `PASSIVE_SCAN|ACTIVE_CONNECTIONS`。
  - ⬜ **小米设备类适配器**：`parse_ad()` 地基已就位（`0xFF` 厂商数据里 company id `0xFE95` 就是入口），**剩余两件**：① 确认具体型号（MiBeacon 广播型 vs MiOT 连接型，解析与鉴权完全不同）；② 属性 → 实体映射 + 变更订阅。需要型号与真机后才能写对。
- **阶段 C — homeCore 中枢（按需，条件触发）**：仅当确认要管 Dot 2 之外的跨品牌家居 + 规则 / 仪表盘时执行。起 homeCore 第二容器，写 `hc-esphome` 插件（§4.2）把我们的 ESPHome 桥接进 homeCore 的 MQTT fabric；随之把"其它家居"接入交给 homeCore，Dot 2 仍由原生桥驱。**若短期无跨品牌需求，跳过本阶段**——桥本身（A/B）已满足 Dot 2 对接。
- **阶段 D — 助手经原生 API / MCP 驱家居 + 护栏（必做，取 Moli 思路）**：LLM 技能的"控制家居"意图命中原生 ESPHome API（或阶段 C 的 homeCore MCP 工具）；高风险操作（锁门 / 关阀 / 撤防）接入**护栏确认**（借鉴 Moli 的 guard，非部署 Moli）。音频仍走 WS 二进制，不碰 broker。

---

## 7. 风险与"不做"清单（延续你的护栏文化）

- ❌ **不为语音流引入 broker**：`mqtt-udp-transport-assessment` 已判定；homeCore 的 MQTT 仅作其内部 fabric，不承载 xiaozhi 音频。
- ❌ **不把 emOS 强行"全盘 ESPHome 化"**：native-api 只接管实体/控制面，音频与有界对话（bounded turn）留在 WS 二进制通道；只用 `user_service` 透传语音指令。
- ❌ **不双写家居状态**：中枢是唯一真相源，xiaozhi-server 不缓存家居状态，只经 MCP 查询/触发。
- ❌ **桥不做通用 HA 转发**：native-api 桥只把 **emOS/EchoMuse 自身可控面** 以标准实体自描述出去（即"设备自述"），**不**做成"把 HA 命令透传到任意第三方设备"的通用转发桥——这正是 `docs/echomuse.md` 的"不做 HA 转发面"。桥做成独立可选模块（`esp_home_bridge` feature），核心语音路径零改动。
- ⚠️ **桥的位置优先宿主机**：native-api Server 优先跑在 xiaozhi-server 宿主机进程（独立模块），不强行下沉到 emOS 固件；固件侧仅作为远期可选（Go port protobuf）。
- ⚠️ **第二服务运维**：中枢是常驻容器 + 数据卷，保持本地优先、无云；部署形态从"单容器"变为"语音容器 + 中枢容器"，需在 `docker-compose.yml` 增加服务。
- ⚠️ **npm 库不进服务端栈**：`Nafaya/esphome-native-api` 是 JS/npm，仅作参考/管理台用途；服务端用 Rust crate `esphome_native_api`(ubihome)。
- ❌ **不为小米（或其它 BLE 设备）手搓 per-device 端点**：Dot 2 是**唯一 BLE 代理**，所有被代理设备统一经 §4.1.2 的"设备类适配器"注册为本桥标准子实体；新增一类设备 = 加一个适配器，不是加端点、不是加路由。小米 MiOT / 各厂商 GATT 协议只实现一次、沉淀在适配器层。
- ❌ **不暴露"没有下发路径"的可写实体**：静音（无 `mute_set`）、LED、打断（内部 `Session.abort` 不是下发命令）当前都只有"读"没有"写"——一律做成只读或不做。**在 HA 里点了没反应的假控件比没有控件更糟**（同阶段 A 删"已唤醒"的逻辑）。要变成可写，前提是先有协议下发实现，不是为了 UI 完整而在桥里造。
- ❌ **不谎报 BLE 代理能力**：`bluetooth_proxy_feature_flags` 只报设备真实具备的位（`PASSIVE_SCAN|ACTIVE_CONNECTIONS`）。配对 / 清缓存 / 描述符读写设备侧没有实现，就既不宣告、请求时也明确回 `error`——**客户端收到错误比收到"成功但无效果"安全**。同理 notify 只能转发设备自动推送的事件，不能声称"按 handle 订阅"。
- ⚠️ **桥只代理一台设备**：`hub_status()` 取第一台（Dot 2 是唯一设备）。多台并存需要多实例或多设备配置，当前明确不做。

---

## 8. 一句话结论

**不塞 HA、不加 broker；必做是在 xiaozhi-server 宿主机起独立可选的 ESPHome Native API 桥（低层 `esphome_native_api::esphomeapi` + `proto` 自建，因为上游 `EspHomeServer` 在 v3.0.0 还是空壳），把 emOS/EchoMuse Dot 2 自身 + BLE 外设标准化成 ESPHome 实体，并实现 `bluetooth_proxy` 让 Dot 2 当 BLE 代理（能力位只报真实具备的）——消灭手搓控制面、零固件改动。** 当前进度：**阶段 A（状态）、阶段 B 的音量控制、以及 `bluetooth_proxy`（扫描/连接/读写/notify 转发）均已落地**；唯一剩下的是**小米设备类适配器**，它需要先确认具体型号与真机（MiBeacon 广播型 vs MiOT 连接型），不能靠猜写。**homeCore 仅在要管跨品牌家居 + 规则/仪表盘时按需集成（只需一个 `hc-esphome` 插件即覆盖 Dot 2 + 小米全部）；Moli 仅借鉴其护栏 / MCP / 轻量思路、不部署。xiaozhi-server 退守纯语音/AI 编排，经原生 API / MCP 驱动家居，高风险操作接护栏确认。** 扩展与对接的成本从"在核心里加端点"降为"写一份设备 profile + 设备类适配器（+ 可选一份 hub 插件）"。
