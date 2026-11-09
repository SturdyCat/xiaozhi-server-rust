# esphome.official — ESPHome 桥实体目录策略 + Native API 协议语义

- **插件 id**：`esphome.official`（官方默认实现，`official=true`）
- **版本**：0.5.0 · **契约**：`xiaozhi:plugin@0.2.0`（api-version `0.2`）
- **扩展点**：`connector.esp-home`
- **权限**：`kv`
- **定位**：阶段三 ESPHome 切片。宿主在桥启动期派发目录征询，本插件决定「哪些实体对外
  暴露 + 显示名覆盖」这一**有界控制面决策**；同时承接 Native API 的**协议语义**——明文帧 /
  服务端面消息 protobuf（S3k/S3r）、**Noise NNpsk0 加密握手 + AEAD 传输帧**（S3t）与
  **设备侧会话**（S3u：握手之后的消息派发与实体面应答）。
  状态采集、音量命令执行与 BLE 代理仍是原生数据面（P8c 真机回归后逐域收口）。

## 功能描述

ESPHome 桥把 EchoMuse Dot 2 的状态暴露为标准 ESPHome 实体（5 个只读 `binary_sensor` +
1 个可写 `number`）。`[esp_home].engine = "wasm-policy"` 时，桥在绑定端口**之前**派发
`connector.esp-home.prepare`，本插件返回逐实体的放行与改名决策；宿主逐索引校验后构建
目录计划：未放行的实体不进 `ListEntities`、不发状态、命令被忽略（fail-closed，不静默降级）。

官方默认策略是**全放行直通**（逐实体 `exposed=true`、不改名）——与 `engine = "native"`
的行为逐字节一致；需要隐藏/改名实体的部署上传自建插件替换即可，宿主不变。

执行模型遵循插件平台铁律：插件是「纯决策函数 + 效果列表」，不持有任何 I/O 句柄。
目录策略面只收到实体目录元数据（索引/object_id/名称/种类/可写性），拿不到设备 ID、
令牌、状态值和任何报文；协议面（帧 / noise）收到的只是字节流，TCP 连接与 I/O 由宿主
`net` 服务承载。

S3t：加密协议语义同样在插件侧——Noise NNpsk0（`Noise_NNpsk0_25519_ChaChaPoly_SHA256`、
prologue `NoiseAPIInit\0\0`）的 ClientHello/ServerHello/msg1/msg2 与传输帧封装全在 guest
纯 Rust 栈内实现（`noise_crypto.rs` / `noise_wire.rs` / `noise.rs`），临时密钥经宿主
`host-core.random` 获取熵（guest 自身无随机源，**不做确定性降级**），握手状态（含 psk /
临时私钥 / 传输密钥）JSON 序列化在插件隔离 kv，事件间无内存态。

S3u：**设备侧会话**（responder）——HA 客户端连入后的消息派发与实体面应答也在插件侧：
`HelloRequest→HelloResponse`、`DeviceInfoRequest→DeviceInfoResponse`、`PingRequest→
PingResponse`、`ListEntities→逐实体定义 + Done`、`SubscribeStates→全量快照 + 置订阅态`、
`Disconnect→应答并关闭`；命令类（`SwitchCommandRequest` / `NumberCommandRequest`）只解码
不回包（数据面仍 native）。`apply_states` 在订阅态下把状态变化封装成对应实体的状态响应帧
（未订阅只更新目录不产出帧）。明文 / 加密（NNpsk0）双模式统一在 `session.rs` 状态机，
会话状态 JSON 序列化在插件隔离 kv——事件间无内存态，与 noise / tuya LAN 编排同纪律。

## 使用方法

### 事件（宿主 → 插件）

| 事件 | payload（JSON） | 结果键 | 决策内容 |
|---|---|---|---|
| `connector.esp-home.prepare` | `{req_id, entities:[{index, object_id, name, kind, writable}]}` | `esphome-result:<req_id>` | 回传 `{route:"native", entities:[{index, exposed, name?}]}` |
| `connector.esp-home.frame-pack` | `{req_id, message, ...}` 见下 | `esphome-frame:<req_id>` | **明文帧编码**（S3k/S3r）：`message ∈ {hello_request, hello_response, disconnect_request/response, ping_request/response, device_info_request, list_entities_request, list_entities_binary_sensor_response, list_entities_number_response, list_entities_done_response, subscribe_states_request, binary_sensor_state_response, number_state_response, switch_command_request, number_command_request}`；消息负载由 guest 手写 protobuf（prost 语义：声明序 + 默认值跳过；float = fixed32 位模式）；空载荷消息 = 类型字节帧。回包 `{ok, message, type, frame_hex}`。字段：`hello_request{client_info, api_version_major/minor}`、`hello_response{api_version_major/minor, server_info, name}`、`list_entities_binary_sensor_response{object_id, key, name, device_class}`、`list_entities_number_response{object_id, key, name, min_value, max_value, step, unit_of_measurement, mode}`、`binary_sensor_state_response{key, state, missing_state, device_id}`、`number_state_response{key, state, missing_state, device_id}`、`switch_command_request` / `number_command_request {key, state, device_id}`。S3r 覆盖 native `esp_home` 桥实际使用的服务端消息全集，与参考 crate prost 编码逐字节对拍 |
| `connector.esp-home.frame-parse` | `{req_id, data_hex}` | `esphome-frame:<req_id>` | **明文帧解码**（S3k/S3r）：`0x00 ‖ varint(载荷长度) ‖ type ‖ payload` 布局（长度标记不含类型字节——参考实现编码器/解码器不对称，`+1` 在解码侧还原）；加密帧（`0x01`）显式报错。回包 `{ok, type, message?, payload_hex, hello?, hello_request?, number_command?, switch_command?}`——`type=2` 解出 `hello`，`type=1` 解出 `hello_request`，`type=51` 解出 `number_command`（HA 拖音量滑块的真实方向），`type=33` 解出 `switch_command` |
| `connector.esp-home.noise` | `{req_id, op, session, ...}`（S3t，见下） | `esphome-noise:<req_id>` | **Noise NNpsk0 加密会话**：`op ∈ {start, step, seal, open, close}`。`start{role:initiator\|responder, psk_hex(32B), server_name?, server_mac?}` → initiator 回 `out_hex = ClientHello(01 00 00) + msg1 帧`，responder 回 `out_hex = ServerHello 帧`（`01 ‖ name ‖ 00 [‖ mac ‖ 00]`）；`step{data_hex}` 喂对端原始字节（可半包/粘包）→ `{phase: await-server-hello\|await-msg2\|await-msg1\|ready, out_hex, peer_name?, peer_mac?}`；`seal{message,...}` 或 `{type, payload_hex}` → `frame_hex`（`01 ‖ u16 BE 密文长 ‖ AEAD`）；`open{data_hex}` → `messages:[{type, message?, payload_hex, hello?, hello_request?, number_command?, switch_command?}]`；`close` 回收会话。会话状态 `esphome-noise-state:<session>` 在插件隔离 kv（含握手凭据材料），并发会话上限 64，失败一律 `ok:false`（对端 MAC 失败按 aioesphomeapi 口径识别 `Handshake MAC failure` 错误帧，不降级明文） |
| `connector.esp-home.session` | `{req_id, op, session, ...}`（S3u，见下） | `esphome-session:<req_id>` | **设备侧会话（responder）**：`op ∈ {open, feed, state, close}`。`open{psk_hex?, identity, entities}` → 明文（无 psk）或加密（有 psk，临时密钥经宿主 `host-core.random`）会话；加密模式回 `out_hex = ServerHello 帧`，`identity` 字段进 DeviceInfoResponse（`api_encryption_supported` 随 psk 有无自动置位）；`feed{data_hex}` 喂对端原始字节（可半包/粘包）→ `{phase, out_hex, messages:[{type, message?, payload_hex, hello_request?, number_command?, switch_command?}], subscribed, closed}`——已知请求类自动回包（Hello/DeviceInfo/Ping/ListEntities/SubscribeStates/Disconnect），命令类只解码不回包；`state{updates:[{key, state}]}` → 订阅态下回 `out_hex = 状态响应帧`（未知 key / 类型漂移拒绝）；`close` 回收状态。会话状态 `esphome-session-state:<session>` 在插件隔离 kv（加密模式含握手凭据材料），并发会话上限 64；**fail-closed**：协议/认证失败即销毁状态（不可重试旧状态或重用 nonce），重复 open、未知 op、非法 session 字符集均显式 `ok:false` |

- `req_id` 由宿主为每次调用生成，必须原样写回结果键；缺失时插件返回业务性错误，宿主显式失败。
- `index` 越界、`name` 为空或超长（>128 字节）都会被宿主拒绝（fail-closed）。
- `entities` 缺失或超 64 条按业务性错误终止本次决策。
- 帧编解码事件与 tuya.official 的 LAN 事件同口径：输入/解码错误写结果通道（`ok:false`），对拍方可见。
- noise 事件的 psk 由调用方随事件载荷传入（与 tuya `lan-connect` 的 `local_key` 同口径；
  生产接管时改由插件配置命名空间 `config-secret` 提供）；`session` 只允许 ASCII 字母数字
  与 `-_.`，合法长度 1..=64。临时密钥每次 `start` 都从宿主随机源现取。

### 动作（管理台 / 语音工具）

| 动作 | payload | 返回 |
|---|---|---|
| `ping` | 任意文本 | 插件 id、扩展点、契约版本与参数回显 |

### kv 键布局（插件隔离命名空间内，宿主强制 `<id>:` 前缀）

| 键 | 值 | 语义 |
|---|---|---|
| `esphome-result:<req_id>` | 结果 JSON | 本次目录决策的交付物 |
| `esphome-frame:<req_id>` | 结果 JSON | 明文帧编解码结果（S3k/S3r，纯函数证据面） |
| `esphome-noise:<req_id>` | 结果 JSON | noise 事件结果（S3t） |
| `esphome-noise-state:<session>` | 握手状态 JSON | 加密会话状态（含 psk / 临时私钥 / 传输密钥 / 流式接收缓冲；只留插件隔离 kv，`close` 回收） |
| `esphome-session:<req_id>` | 结果 JSON | 设备侧会话事件结果（S3u） |
| `esphome-session-state:<session>` | 会话状态 JSON | 设备侧会话状态（身份 / 实体目录 / 订阅态 / 加密握手状态 / 流式接收缓冲；`close` 或协议失败回收） |

### 边界与限制

- `route` 只允许 `native`；数据面不出原生。
- 协议语义面（S3k/S3r/S3t/S3u）是**可执行证据面**：明文帧 + 核心消息 protobuf + Noise
  NNpsk0 握手/传输帧 + 设备侧会话（消息派发与实体面应答）在插件；TCP 连接、状态采集、
  音量命令执行、BLE 代理仍 native。对拍：`plugins/wasm/tests/esphome_frame.rs`（8 项，
  明文帧，esphome-native-api 真连接，PingResponse 逐字节）+ `esphome_noise.rs`（6 项：
  noise-protocol 参考实现双角色逐字节、**真实加密连接**收发 HelloRequest/HelloResponse、
  错误 psk 识别 `Handshake MAC failure`、自检 + 会话生命周期 fail-closed）+
  `esphome_session.rs` / `esphome_session_encrypted.rs`（5 项：明文/加密**真 TCP** 全链
  ——protobuf 载荷与参考 prost 逐字节、加密帧与参考 AEAD 逐字节、半包/粘包注入、
  命令解码不回包、fail-closed 销毁状态 + 生命周期守卫）。
- noise 依赖全纯 Rust（sha2 / hmac / chacha20poly1305 关 getrandom / x25519-dalek），
  wasm32-unknown-unknown 可编；临时密钥熵只来自宿主 `host-core.random`，熵源不可用时
  `start`/`open` 显式失败（**不做确定性降级**——可预测临时密钥会破坏前向保密与 nonce 纪律）。
- 会话事件（S3u）与 noise 事件同口径：psk 随载荷传入（生产接管时改由插件配置命名空间
  `config-secret` 提供）；`session` 只允许 ASCII 字母数字与 `-_.`，合法长度 1..=64；
  身份与实体目录随 `open` 载荷传入（数据面真相源仍 native：`[esp_home]` 配置 + 实体状态
  观测，由宿主桥在后续切片接线）。
- 需 `[esp_home].engine = "wasm-policy"` 且镜像带 `esp_home`（蕴含 `echomuse-plugin`）
  feature 才有调用方；缺 feature/宿主/插件时桥显式报错不启动。
- 插件不持有 PCM / 模型 / 设备句柄；key 恒为宿主静态实体表索引，插件不能增删实体。
- 策略结果全部有界，超出宿主校验范围的字段会被拒绝。

### 构建与分发

```bash
# 单插件
./build.sh                              # → server/tests/fixtures/wasm/esphome.wasm
# 全量（读 plugins-src/manifest.toml）
../../tools/build-system-plugins.sh     # → build/system-plugins/esphome.official.wasm
```

上传只登记 `Staged`，需依次 `POST /api/plugins/esphome.official/register` 与
`POST /api/plugins/esphome.official/activate` 才进入 Active；只有 Active 条目参与扩展点解析。
