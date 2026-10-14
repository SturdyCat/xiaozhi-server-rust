# MQTT+UDP Transport 需求真实性与接入成本评估

> 评估人：架构师 高见远（bob / software-architect）
> 评估对象：为 `xiaozhi-server-rust` 增加 MQTT+UDP 承载的提案
> 权威依据：固件仓库 `../xiaozhi-esp32`（HEAD = `d588e48`，官方 `78/main`）**实读**
> 服务端仓库：`/Users/admin/Desktop/notip.com.cn/xiaozhi-server-rust`
> 核实方式：逐文件精读 + `grep -E`/`git log -S` 追溯；**所有判定带`文件:行号` 证据**，未读到的一律标注「未验证」。

---

## 1. 结论摘要

> **结论：现在不做，也不必做技术验证 PoC。当前唯一真实缺口是「固件编译期二选一」，不是服务端能力缺失。**

1. **MQTT+UDP 在固件里是编译期互斥的替代方案**，不是与WebSocket 并存的第二承载（`main/CMakeLists.txt:136-140`、`main/Kconfig.projbuild:27-36`）。**`default = CONNECTION_TYPE_MQTT_UDP`**——即官方默认固件走 MQTT，但**用户在编译自己的固件时已经选了 WebSocket**，这不是服务端能补的。
2. **服务端不需要 MQTT broker。** 固件的 MQTT 是「设备当客户端连 broker」，服务端只作为 **broker 的另一侧**收发 topic。若自建 broker，等于新增一个常驻中间件（ mosquitto/EMQX），违背「纯 Rust 单二进制 + 镜像要小」的项目约束。
3. **`trait Transport` 抽象层完全够用**（`server/src/app/transport.rs:40-65`），6 个方法对 MQTT+UDP 语义无缺口，**但需要新增一个「MQTT topic → 单条有序流」的内部合并器**（约 80 行），不是改trait 签名。
4. **UDP 音频是真实的可靠性退化**：固件侧丢包**只打warning 不重传**（`mqtt_protocol.cc:206-208`），且设备**收不到自己丢包的任何反馈**。在 16kHz/60ms Opus 单声道语音场景下丢一个包 = 丢 60ms 语音，可闻但不至于毁掉一轮对话。
5. **需求侧不成立。** 本项目是 N5105 单机Docker 自部署（`docker-compose.yml` 单服务、`cpus: "3.5"`、无broker）。MQTT+UDP 的三类收益——省流量、省连接数、穿 NAT——**当前一条都不需要**。
6. **有一个真实的、但极小的痛点**：固件 `mqtt` 配置下发链路已通（`main/ota.cc:88-101`），而服务端**有意不下发 `mqtt` 字段**（`server/src/app/ws/ota.rs:20`）。这是**正确的克制**，不是缺口。

**触发重新评估的条件见 §7.3。**

---

## 2. 固件侧能力核实表（问题 1–5）

### 2.1 问题 1：MQTT+UDP 到底承载什么？

**结论：文本走 MQTT（双向），音频走 UDP（双向），两条通道完全分离。**

| 方向 | 承载 | 代码位置 | 载荷 |
|------|------|---------|------|
| 设备 → 服务端 **文本** | **MQTT** publish | `mqtt_protocol.cc:103-111` | `SendText` → `mqtt_->Publish(publish_topic_, text)` |
| 服务端 → 设备 **文本** | **MQTT** 消息回调 | `mqtt_protocol.cc:62-90` | `OnMessage(topic, payload)` → cJSON 解析 |
| 设备 → 服务端 **音频** | **UDP** | `mqtt_protocol.cc:113-135` | `SendAudio` → `udp_->Send(encrypted)` |
| 服务端 → 设备 **音频** | **UDP** | `mqtt_protocol.cc:192-227` | `udp_->OnMessage` → 解密 → `on_incoming_audio_` |

**关键细节 1：UDP 是「16 字节头 + AES-128-CTR 加密的裸 Opus」，不是 v1/v2/v3 二进制协议。**

`mqtt_protocol.cc:119-134` 逐行拆解：

```
std::string nonce(aes_nonce_);// 16 字节，作为 AES-CTR 的 nonce 复用同一个 buffer
*(uint16_t*)&nonce[2]  = htons(data.size());// header[2:4]  = 净荷长度（网络序）
*(uint32_t*)&nonce[12] = htonl(++local_sequence_); // header[12:16] = 序号（网络序）
// memcpy 到输出头部 → 然后 mbedtls_aes_crypt_ctr(data) 追加在头部之后
udp_->Send(encrypted);
```

接收侧 `mqtt_protocol.cc:193-200` 校验：
- `data.size() >= sizeof(aes_nonce_)`（16）
- `data[0] == 0x01`（包类型）

所以**头部布局**（推断，与固件写偏移量一致，未在服务端找到对照实现佐证）：

| 偏移 | 含义 |
|------|------|
| `[0]` | 包类型，必须 `0x01` |
| `[2:4]` | 净荷长度 u16 网络序 |
| `[4:12]` | 保留（复用为 CTR nonce 的一部分） |
| `[12:16]` | 序号 u32 网络序 |

⚠️ **对服务端实现的重要含义**：这套头部与 `server/src/app/protocol.rs` 的 `BinVersion` v1/v2/v3 帧格式**是两套完全独立的东西**。`send_binary` 不能复用现有的 v1/v2/v3 封装逻辑，**需要为 MQTT+UDP 单独写一套 UDP 音频封装 + AES-CTR**。现有 `downlink/pacer.rs` 的帧预算逻辑（按 ESP 20 包硬约束）也需要重新核对。

**关键细节 2：丢包后如何处理 —— 只打 warning，不重传、不反馈。**

`mqtt_protocol.cc:201-208`（设备侧**接收**下行音频，即服务端的 UDP 下行丢包）：

```cpp
uint32_t sequence = ntohl(*(uint32_t*)&data[12]);
if (sequence < remote_sequence_) {
    ESP_LOGW(TAG, "...old sequence...");  return;// 陈旧包：丢弃
}
if (sequence != remote_sequence_ + 1) {
    ESP_LOGW(TAG, "...wrong sequence...");  // 序号不连续：只告警，不补发、不请求
}
...
remote_sequence_ = sequence;
```

- **陈旧包（`sequence < remote_sequence_`）丢弃**。
- **序号跳变（丢包）只 `ESP_LOGW` 告警，然后照常把这一包解码播放**。
- **没有任何重传、NACK、或向服务端上报缺口的机制。**

→ **服务端丢一个 UDP 包 = 设备少播 60ms 语音，服务端完全不知情**（服务端 UDP socket 收不到任何反馈）。这一点在 §5 详细展开。

**关键细节 3：`hello` 里服务端必须回的东西（含加密材料）。**

设备发（`mqtt_protocol.cc:170-177`）：
```json
{"type":"hello","version":3,"transport":"udp",
 "audio_params":{"format":"opus","sample_rate":16000,"channels":1,"frame_duration":60}}
```
注意 `version: 3` 与 WebSocket 路径的 `version: 1`（`websocket_protocol.cc:110`）**不同**。

服务端必须回（`mqtt_protocol.cc:237-277` `ParseServerHello`，缺任一项直接 `return` 等不到、超时 10s）：

| 字段 | 必需性 | 用途 | 代码 |
|------|--------|------|------|
| `transport` | 必需 | 必须等于 `"udp"`，否则拒绝 | `:239-242` |
| `session_id` | 可选 | 后续所有消息回带 | `:244-248` |
| `audio_params.sample_rate` | 可选 | 下行采样率（固件据此resample） | `:251-257` |
| `udp.server` | **必需** | UDP 目标地址 | `:264` |
| `udp.port` | **必需** | UDP 目标端口 | `:265` |
| `udp.key` | **必需** | hex 字符串，AES-128 密钥 | `:266,273` |
| `udp.nonce` | **必需** | hex 字符串，16 字节 CTR 初始向量 | `:267,271` |

>⚠️ 注意 `:264-267` 四次 `cJSON_GetObjectItem(...)->valuestring` **无空指针检查**——任一项缺失即空指针解引用崩溃。服务端必须严格按契约填满。

**关键细节 4（⚠️ 最重要的发现）：固件从不 `Subscribe`，下行文本消息无处可来。**

- 全仓 `grep -rn "Subscribe" --include="*.cc" --include="*.h"` = **0 命中**。
- `git log --all -S "Subscribe" -- main/` 定位到 `e2c5578 "fixbug: getting empty value"`（2025-01-19），该提交**删除了** `subscribe_topic_` 成员与唯一一处调用：
  ```diff
  -    subscribe_topic_ = settings.GetString("subscribe_topic");
  -    if (!subscribe_topic_.empty()) {
  -        mqtt_->Subscribe(subscribe_topic_, 2);
  -    }
  ```
- 当前 `MqttProtocol` 只有 `publish_topic_`（`mqtt_protocol.h:41`），**没有 subscribe topic 字段**。

**这意味着两件事**（必须如实说明，这比附和一个假设有价值得多）：

1. **`Mqtt` 组件（`78/esp-ml307`，来自 `main/idf_component.yml:14`）内部必然隐式订阅了某个 topic。** 该组件源码**不在本仓库**（`main/idf_component.yml` 声明依赖，`managed_components/` 未提交），我**未验证**其内部订阅的具体 topic 是什么。
2. **因此「服务端要往哪个 topic 发布下行文本」这个问题，在当前固件 HEAD 上无法从本仓库确定。** 若组件是「订阅 publish_topic 自身」（自回环），那服务端只需往设备 publish 的同一 topic 发布即可；若是「订阅 `{prefix}/{client_id}/down` 之类」，则需要 client_id 参与。这是**做 PoC 前必须先读 `esp-ml307` 组件源码才能回答的前置未知**。

**对评估的影响**：这不是「服务端加个 Transport 实现」能绕过的——**它需要先读第三方组件源码**才能确定下行寻址方式。

### 2.2 问题 2：与 WebSocket 是互斥还是并存？

**结论：编译期互斥，一台设备只能二选一，且默认是 MQTT+UDP。**

三处证据：

| 证据 | 位置 | 内容 |
|------|------|------|
| 编译期选源文件 | `main/CMakeLists.txt:136-140` | `if(CONFIG_CONNECTION_TYPE_MQTT_UDP) list(APPEND SOURCES "protocols/mqtt_protocol.cc")` **`elseif`(CONFIG_CONNECTION_TYPE_WEBSOCKET)`→`"protocols/websocket_protocol.cc"`，二选一 |
| 运行期选实现类 | `main/application.cc:368-372` | `#ifdef CONFIG_CONNECTION_TYPE_WEBSOCKET` → `WebsocketProtocol`，`#else` → `MqttProtocol` |
| Kconfig 声明 | `main/Kconfig.projbuild:27-36` | `choice CONNECTION_TYPE` / `default CONNECTION_TYPE_MQTT_UDP` |

**加固证据（最有说服力的一处）**：`main/boards/common/wifi_board.cc:116-125`

```cpp
WebSocket* WifiBoard::CreateWebSocket() {
#ifdef CONFIG_CONNECTION_TYPE_WEBSOCKET
    ...
#endif
    return nullptr;          // ← MQTT_UDP 构建下，CreateWebSocket 直接返回 nullptr
}
```

MQTT_UDP 构建时 `CreateWebSocket()` 返回空指针——**不是运行期「两种都支持」，而是编译期把另一条路彻底删掉**。`CreateMqtt()`/`CreateUdp()`（`:127-133`）则**没有** `#ifdef` 保护，恒返回 `new EspMqtt()`/`new EspUdp()`。

同时 `main/Kconfig.projbuild:38-46` 里 `WEBSOCKET_URL`（默认 `wss://api.tenclass.net/xiaozhi/v1/`）和 `WEBSOCKET_ACCESS_TOKEN`（默认 `test-token`）**均 `depends on CONNECTION_TYPE_WEBSOCKET`**——即 WS 配置项在 MQTT 构建下不存在。

> **这直接推翻了提案的前提。** 「服务端支持 MQTT 后设备就能用MQTT+UDP」不成立：用户必须**重新编译固件**（改 Kconfig 或 `menuconfig`）才能切到 MQTT_UDP，而当前用户的固件是 WebSocket 构建。服务端能力建设**不能替代**固件重编译，反而是固件重编译后服务端才有需求。

### 2.3 问题 3：协议端口与参数怎么配？

**MQTT broker 地址：服务端 OTA 端点下发 → 设备写入 NVS `mqtt` 命名空间。**

| 环节 | 位置 | 内容 |
|------|------|------|
| 设备请求 | `main/ota.cc`（`CheckVersion`） | POST 到 `OTA_URL`，带设备信息 |
| 服务端返回 | `server/src/app/ws/ota.rs:191` | **只返回 `websocket` 字段，无 `mqtt`** |
| 设备消费 | `main/ota.cc:88-101` | `cJSON_GetObjectItem(root, "mqtt")` → `Settings settings("mqtt", true)` → 逐键 `SetString` 写 NVS |
| 设备读取 | `mqtt_protocol.cc:40-45` | `Settings("mqtt", false)` 读 `endpoint`/`client_id`/`username`/`password`/`publish_topic` |

**这是纯服务端驱动的，零硬编码。** NVS 键位：`endpoint`、`client_id`、`username`、`password`、`publish_topic`（`mqtt_protocol.cc:41-45`，逐行对应）。

> ⚠️ 注意 `main/ota.cc:96` 遍历时 `if (item->type == cJSON_String)` —— **只接受字符串类型的值**。若 `udp.port` 是数字（固件 `mqtt_protocol.cc:265` 用 `valueint` 读整数），它**不会**被写入 NVS。当前 OTA 的 `mqtt` 对象只含字符串键（endpoint/client_id/username/password/publish_topic），所以不冲突；但**若将来要下发非字符串 MQTT 参数会静默失败**。

**MQTT 端口：硬编码 `8883`**（`mqtt_protocol.cc:93`）：
```cpp
if (!mqtt_->Connect(endpoint_, 8883, client_id_, username_, password_)) {
//                                          ^^^^ 硬编码，无法配置
```
8883 = MQTT over TLS 标准端口。**没有明文 1883 选项**（至少在本仓库范围内）。配合 `ml307_mqtt.h` 确认支持 TLS。

**UDP 目标地址与端口：由服务端在 `hello` 响应里指定**（`mqtt_protocol.cc:259-265, 229`）：
```cpp
udp_server_ = ...["udp"]["server"]->valuestring;
udp_port_   = ...["udp"]["port"]->valueint;
...
udp_->Connect(udp_server_, udp_port_);      // 设备主动向服务端指定的地址发
```

**这是本次评估最重要的架构发现之一**：UDP 是**设备主动连服务端**（设备持有 socket，向服务端 `udp.server:udp.port` 发包并接收）。所以：

- ✅ UDP **能穿 NAT**（设备侧发起，服务端侧需公网可达 UDP 端口）。
- ❌ 但**服务端必须额外开一个 UDP 端口**并在 `hello` 里通告它。`docker-compose.yml` 当前只映射 `"8000:8000"`（TCP），**没有 UDP 映射**。

**注意 UDP 目标地址不可为空**：`:264` 无空指针检查，服务端必须填。

### 2.4 问题 4：打断（abort）路径怎么传？

**结论：打断走 MQTT 文本（JSON），与音频通道无关。下行音频走 UDP。**

| 步骤 | 路径 | 位置 |
|------|------|------|
| 设备侧触发 | `AbortSpeaking(reason)` → `aborted_ = true` → `protocol_->SendAbortSpeaking(reason)` | `application.cc:725-729` |
| 构造 JSON | `{"session_id":"...","type":"abort"[,"reason":"wake_word_detected"]}` | `protocol.cc:34-40` |
| **发送** | `SendText(message)` → **`mqtt_->Publish(publish_topic_, ...)`** | `mqtt_protocol.cc:103-111` |
| 服务端接收 | MQTT 消息 → `on_incoming_json_` → `application.cc:407` 起的 JSON 分发 | — |

三个打断触发点（全部走同一条 MQTT 文本路径）：

| 触发 | 位置 | reason |
|------|------|--------|
| 唤醒词检出（打断播放） | `application.cc:539` | `kAbortReasonWakeWordDetected` |
| 按键打断 | `application.cc:829` | `kAbortReasonNone` |
| 其他（`:262`、`:296`） | `application.cc:262,296` | `kAbortReasonNone` |

**服务端→设备的下行音频走 UDP**（`mqtt_protocol.cc:192-227`），**下行 JSON（含 `tts start`/`tts stop`/`goodbye`）走 MQTT**（`:62-90`）。

**⚠️ 这里有一个真实的、MQTT+UDP 特有的竞态（WS 路径不存在）**：

设备侧收到下行音频的准入门（`application.cc:377-383`）：
```cpp
protocol_->OnIncomingAudio([this](std::vector<uint8_t>&& data) {
    std::lock_guard<std::mutex> lock(mutex_);
    if (device_state_ == kDeviceStateSpeaking) {      // ← 硬门控
        audio_decode_queue_.emplace_back(std::move(data));
    }
});
```
而 `kDeviceStateSpeaking` 只在收到 **`tts` `start`** 消息后设置（`application.cc:411-417`）。

在 WS 路径下，**音频与 `tts start` 走同一条有序 TCP 流**，`tts start` 必然先到。
在 MQTT+UDP 路径下，**`tts start` 走 MQTT、`tts start` 后第一批音频走 UDP**——**两条独立通道没有顺序保证**。UDP 首包可能早于 MQTT 的 `tts start` 到达 → `device_state_` 仍是 `Listening` → **首批音频被静默丢弃**（`:380` 无 else 分支、无日志、无计数）。

这是**引入 MQTT+UDP 后新出现的一类 bug**，服务端必须显式处理（例如 `tts start` 后主动等一个RTT 再发首包，或协议层加个下行 barrier）。**现有 WebSocket 路径完全没有这个问题。**

### 2.5 问题 5：keepalive / 心跳怎么处理？

**结论：`Transport::keepalive` 留空即可（现有默认实现正确），但服务端需要自己实现「空闲超时 → 软关闭」。**

| 关注点 | 固件行为 | 位置 | 服务端对应 |
|--------|---------|------|-----------|
| MQTT 心跳 | `mqtt_->SetKeepAlive(90)`（90 秒） | `mqtt_protocol.cc:56` | **无**（broker 侧PINGREQ） |
| MQTT 重连 | `MQTT_RECONNECT_INTERVAL_MS 10000`（宏，定义于 `mqtt_protocol.h:19`）——⚠️ **该宏在本文件内无使用点，实际重连由 `esp-ml307` 组件负责（未验证）** | `mqtt_protocol.h:19` | 无 |
| 会话空闲超时 | `IsTimeout()`：**120 秒**无任何下行消息即视为超时 | `protocol.cc:111-122` | — |
| 通道存活判定 | `IsAudioChannelOpened()` = `udp_ != nullptr && !error_occurred_ && !IsTimeout()` | `mqtt_protocol.cc:298-300` | — |
| `hello` 应答超时 | **10 秒**（`xEventGroupWaitBits(..., pdMS_TO_TICKS(10000))`），超时→ `SERVER_TIMEOUT` | `mqtt_protocol.cc:180-185` | 服务端必须在此窗口内回 `hello` |

`server/src/app/transport.rs:53-55` 的注释已预判正确：
> 「自带心跳机制的承载（如 MQTT）留空即可。」

`keepalive` 的默认实现 `async {}`（`transport.rs:63-65`）**直接可用，零改动**。

⚠️ 但服务端仍需实现 `last_incoming_time_` 对等物：固件在 **120 秒**无下行后自行 `IsTimeout()` 关闭，**服务端若不主动关，会留下一个设备已放弃的僵尸会话**。WS 路径靠 TCP/`Closed` 帧解决，MQTT+UDP 路径下**UDP 无连接、服务端无法感知对端消失**，必须自建 idle 计时器（建议略小于 120s，如 100s）。

---

## 3. `trait Transport` 够不够用（问题 6）

### 3.1 逐方法对照

| # | 方法（`transport.rs`） | MQTT+UDP 需求 | 够用？ |
|---|---------------------|---------------|--------|
| 1 | `send_text(&mut self, json: String) -> Result<()>` | MQTT publish（文本全部走这里） | ✅ **完全够** |
| 2 | `send_binary(&mut self, data: Vec<u8>) -> Result<()>` | UDP send（音频全部走这里）——但需**内部加 16 字节头 + AES-CTR + 序号自增** | ⚠️ **签名够，实现需在Transport 内部藏加密状态**。`aes_ctx`/`local_sequence_` 不在签名里，但可以作为 `MqttUdpTransport` 的私有字段——**不构成 trait 缺口** |
| 3 | `recv(&mut self) -> IncomingFrame` | **需合并两个来源**：MQTT topic 消息（文本）+ UDP datagram（音频） | ⚠️ **签名够，但语义需内部合并**（见§3.2） |
| 4 | `try_recv(&mut self) -> Option<IncomingFrame>` | 流水线间隙轮询打断（`abort` 走 MQTT 文本） | ✅ **够**（合并器提供 `try_recv`） |
| 5 | `keepalive(&mut self)` | MQTT 自带心跳 | ✅ **够，且默认实现直接用**（`transport.rs:63-65`） |
| 6 | `close(&mut self)` | 发 `goodbye`（MQTT，`mqtt_protocol.cc:146-150`）+ 关 UDP | ✅ **够**（默认实现留空也能跑，但建议实现） |

**额外需求 vs 现有 trait：**

| 需求 | 是否需要 trait 改动 |
|------|------------------|
| UDP socket 生命周期管理 | ❌ 不需要，`MqttUdpTransport` 的 `Drop` 自己关 socket |
| AES-CTR 上下文 | ❌ 不需要，私有字段 |
| `Closed` 粘滞语义 | ❌ 复用现有语义（`transport.rs:18-20` 已定义） |
| **MQTT topic 订阅** | ⚠️ **不需要改 trait**，但需要「谁来订阅」的设计决策（见 §3.3） |
| **服务端侧 broker** | ⚠️ **trait 无能解决——这是部署架构问题**（见 §3.4） |
| **UDP 空闲超时** | ⚠️ 不需要 trait 改动，但需要在 `recv()` 里实现计时 |

### 3.2 核心问题：「单条有序上行流」假设成立吗？

**结论：对会话流水线而言成立；对传输层内部不成立，需要合并器——但不需要改trait。**

现状（`transport.rs:44-46`）：
>「阻塞收取下一条上行帧（会话主循环）」

WS 是**一条双向有序字节流**，这个抽象天然贴合。MQTT+UDP 是**两个独立无序通道**：

```
设备侧来源                服务端侧
─────────────────────────────────────────────────────────
JSON 文本上行──► MQTT ──► [ 合并器 ] ──► recv()/try_recv()
                                    └──► mpsc::Sender<IncomingFrame>
音频上行    ──► UDP ──► [UDP 后台task] ──┘
```

**最小改动方案（约 80 行，`MqttUdpTransport` 内部私有，不触碰 `trait Transport`）**：

```rust
// server/src/app/transport/mqtt_udp.rs（新增，单文件）
pub struct MqttUdpTransport {
    mqtt_tx: mqtt publish handle,          // 下行文本
    udp_tx:  UdpSocket,                   // 下行音频
    rx:  mpsc::Receiver<IncomingFrame>,   // 上行合并流
    _udp_rx_task: JoinHandle<()>,         // 后台收包 task（持有 socket）
    closed: bool,                         // 复用粘滞 Closed 语义
}
// 实现要点：
//   1. 一个后台 tokio task 持有 UdpSocket，循环 recv_from → 校验 16B 头
//      → 查设备 IP 过滤 → AES-CTR 解密 → tx.send(Binary(decrypted))
//   2. MQTT 订阅回调（rumqttc eventloop 在另一个 task）→ tx.send(Text(payload))
//   3. recv() = self.rx.recv().await；try_recv() = self.rx.try_recv()
//   4. send_binary() =加头 + AES-CTR 加密 + udp_tx.send()
```

**这个方案的好处**：
- `trait Transport` **一行不改**，`Session<'a, T: Transport>`（`session.rs:108`）的**泛型静态分发零影响**——`run_session::<MqttUdpTransport>` 和 `run_session::<WsTransport>` 并列即可，`app/ws/mod.rs:279` 那种「构造即用」的形态保持。
- 复用现有 `Closed` 粘滞语义与 `try_recv` 的 `None`/`Closed` 区分（`transport.rs:18-20` 已论证过，MQTT+UDP 完全适用）。

**这个方案带进来的新语义（需要额外设计，不是 trait 能挡住的）**：

| 新语义 | 现状 | 说明 |
|--------|------|------|
| **跨通道无序** | WS 单流天然有序 | §2.4 的 `tts start` vs UDP 首包竞态 |
| **UDP 丢包静默** | WS 由 TCP 保证 | 服务端无反馈回路，需自检seq 连续性 |
| **UDP 无连接 ⇒无 `Closed`** | WS 有 Close 帧 | 必须自建 idle 超时（§2.5） |
| **下行寻址不确定** | WS 一条连接即身份 | 需知道往哪个 topic 发布（§2.1 关键发现 4） |

### 3.3 一个前置未知：下行 MQTT 寻址

§2.1 关键发现 4 已述：固件**从不 `Subscribe`**，`subscribe_topic_` 于2025-01-19 被删。订阅行为隐藏在 `78/esp-ml307` 组件内（该组件**不在本仓库**，`main/idf_component.yml:14` 声明、`managed_components/` 未提交）。

**未验证**：设备到底订阅哪个 topic。这**必须先读 `esp-ml307` 组件源码**才能回答。可能性：
- (a) 订阅 `publish_topic_` 自身（自回环）→ 服务端往设备 publish 的同topic 发布即可；
- (b) 订阅 `{prefix}/{client_id}/down` 派生 topic → 需 client_id 参与路由；
- (c) 其他。

**这条不确定性直接决定「能不能做」的可行性，不该猜。**

### 3.4 一个 trait 解决不了的问题：服务端需要一个 MQTT broker

固件 `mqtt_protocol.cc:93` 是**设备当 MQTT 客户端**去连 broker：
```cpp
mqtt_->Connect(endpoint_, 8883, client_id_, username_, password_);
```

`endpoint_` 来自 OTA 下发的 NVS（§2.3），**指向一个 broker**。服务端**不是** MQTT 端点，而是 broker 的对端。要让设备连上，服务端侧必须二选一：

| 方案 | 说明 | 代价 |
|------|------|------|
| A. 内嵌 broker | 服务端内起一个 MQTT broker 监听 8883 | 引入 broker 依赖（`rumqttd`）+ 与 axum 共存 + 镜像变大 + **违背「纯 Rust 单二进制」初衷**（虽然 rumqttd 是纯 Rust） |
| B. 外部 broker | 部署 mosquitto/EMQX，服务端作为 MQTT 客户端连上去 | **多一个常驻中间件**，违背项目约束；且要处理 broker 持久化/鉴权/ACL |
| C. `ESP-MQTT` 直连 | 若 `esp-ml307` 支持「直连自定义 TCP 端点」模式 | **未验证**——`MqttProtocol` 代码写死了 `Connect(endpoint_, 8883, ...)`，走的是标准 MQTT |

**A 和B 都会打破当前「单 crate、单二进制、单容器」的部署形态**（`README.md:3`：「纯 Rust 单二进制」；`docker-compose.yml`：单服务、`ports: ["8000:8000"]`）。这是**架构级代价，不是加个 crate 的问题**。

---

## 4. 依赖与成本评估（问题 7）

### 4.1 依赖选择：rumqttc vs paho

| 维度 | `rumqttc`（纯 Rust） | `paho-mqtt`（C 绑定） |
|------|---------------------|----------------------|
| 语言 | 纯 Rust，async/tokio 原生 | C 库 + tokio 兼容层（`paho-mqtt` 的 `AsyncClient`） |
| 与项目契合 | ✅ 与 `tokio = features=["full"]`、`rustls` 一致 | ⚠️ 需系统装 `libpaho-mqtt`，**破坏「纯 Rust 单二进制」** |
| 编译体积 | 小（纯 Rust 增量编译） | 需 vendored C 源码或动态链接 |
| 内存 | 可控 | C 侧额外缓冲 |
| TLS | `rustls`（项目已有 `rustls 0.23` + `webpki-roots`） | 需 openssl 或 mbedtls |
| N5105 影响 | 可忽略 | 增加动态库依赖与运行时解析开销 |
| 项目约束契合 | ✅ **Rust-first** | ❌ 引入 C 库，违反约束 |

**结论：选 `rumqttc`。** 理由：项目 `server/Cargo.toml` 已明确 Rust-first（`rustls` 而非 `openssl`、`audiopus` 仅 `sherpa` feature 开启）、单 crate 无 workspace（`Cargo.toml` 无 `[workspace]` 段，`server/Cargo.toml` 是唯一 manifest）、Docker 镜像要小。`paho` 需要 C 库，直接违反这些约束。

**若最终决定不做（本文结论），则不引入任何依赖——这是最省的结果。**

### 4.2 若要做的真实工作量

| 组件 | 规模 | 说明 |
|------|------|------|
| UDP 音频封装（16B 头 + AES-128-CTR + seq） | ~150 行 | 收发双向，含 seq 连续性校验与丢包计数 |
| MQTT 上行合并器（mpsc + 后台 task） | ~80 行 | §3.2 方案 |
| `MqttUdpTransport`（6 个 trait 方法） | ~250 行 | |
| broker（内嵌 `rumqttd` 或外部部署文档） | ~150 行 + 部署 | 架构级改动 |
| `hello` 协商扩展（udp.server/port/key/nonce） | ~60 行 | 复用 `ws/ota.rs` 或新端点 |
| `ota.rs` 下发 `mqtt` 字段 | ~30 行 | `ota.rs:20` 当前有意省略 |
| UDP 空闲超时 | ~30 行 | |
| **跨通道顺序 barrier（§2.4竞态）** | ~50 行 | **易漏，且漏了就是难查的音频丢失 bug** |
| 文档/固件配置指引（用户得改 Kconfig 重编译） | — | |
| **合计** | **约 750 行 + 一个常驻中间件** | **不是「实现一份 trait」的量级** |

> 对比：`app/ws/`（含 `ota.rs`/`firmware.rs`/`health.rs`/`metrics.rs`/`router.rs`）整个目录才是这个量级。**这是新增一个与 `ws/` 同量级的传输网关**，而非填空。

---

## 5. UDP 可靠性判断（问题 8）

### 5.1 事实链

| # | 事实 | 证据 |
|---|------|------|
| 1 | 固件对**下行** UDP 丢包只 `ESP_LOGW`，不重传、不请求补发 | `mqtt_protocol.cc:206-208` |
| 2 | 固件对**上行** UDP 丢包完全无感（UDP 不反馈） | 协议设计 |
| 3 | 服务端**收不到任何丢包反馈**（UDP 无连接、无 ACK） | 协议设计 |
| 4 | 序号是唯一的完整性线索（u32，网络序） | `mqtt_protocol.cc:121,201` |
| 5 | 音频参数：16kHz / 单声道 / **60ms Opus 帧** | `application.h:45`、`mqtt_protocol.cc:175` |
| 6 | 设备解码队列 `audio_decode_queue_` 无容量上限、无序号重排 | `application.cc:379-382, 644-646` |
| 7 | WS 路径是TCP：`Ordered + Reliable` | `app/protocol.rs` v1/v2/v3 帧 |

### 5.2 定量估算

单声道 16kHz Opus，官方小智默认码率约 **24–32 kbps**（`frame_duration: 60ms`）。一个60ms 帧 ≈ **180–240 字节**。60 秒语音 ≈ 50–60 帧 ≈ **3.6–4.8 MB**（上行+下行各一路）。

**丢一个包 = 丢 60ms 语音**。Opus 在 60ms 帧下对单包丢失的韧性较好（可做 PLC 隐藏），但**连续 2–3 包丢失（180ms）会明显可闻**，连续 5 包（300ms）用户会察觉断句。

### 5.3 判断：**这是真实退化，但在「局域网/单用户自部署」场景下可接受；在「公网/多设备」场景下不可接受。**

**支持「可接受」的**：
- 本项目定位是**局域网自部署**（`docker-compose.yml` 单容器本地 N5105，无公网入口配置）。
- 局域网丢包率通常 <0.1%，且 WiFi 环境下突发丢包多与干扰相关，通常持续 <100ms。
- 语音场景对 1 个 60ms 包的容忍度天然较高（人类听觉对 60ms 单帧缺失不敏感）。
- 小智固件官方本身就选了这个方案（`default CONNECTION_TYPE_MQTT_UDP`），说明其工程上可接受。

**支持「不可接受」的**：
- **无任何恢复机制**：TCP 会重传，UDP 完全不会。这是**能力上的净损失**，不是「换个实现方式」。
- **服务端失去可观测性**：丢包在两端都不可见，排障时无法区分「设备没说话」和「网络吞了包」。这与本项目已有的五阶段延迟分位（`events.rs:44-58`）、`app/metrics.rs:132-157` 的可观测性投入严重不匹配。
- **§2.4 的跨通道竞态是确定性丢包**（首包早于 `tts start` 必丢），不是概率性的。
- 移动端（4G/WiFi 切网）场景下丢包率可达 1–5%。

**结论**：
> UDP 在**能力上是真实退化**（丢失可靠性 + 丢失可观测性，两者都是 TCP 已有而 UDP 没有的）。在本项目当前「局域网 + 少设备 + 少并发」的部署形态下，**退化的实际影响小到可以承受**——但这是**用部署形态换来的**，不是协议本身更好。
>
> **明确表态**：如果本项目要走向「公网暴露 + 多设备」，**不应该选 UDP**。如果长期保持局域网自用，UDP 的收益（省连接数）在 1–5 台设备规模下**等于零**，那退化就是纯亏。

---

## 6. 需求真实性分析（问题 9–10）

> **这是全文最重要的部分。**

### 6.1 问题 9：谁会用？换 MQTT+UDP 解决什么真实痛点？

**部署事实（全部来自仓库实读）**：

| 事实 | 证据 |
|------|------|
| 纯 Rust 单二进制，为N5105 级低功耗主机 Docker 部署而生 | `README.md:3` |
| 单容器、单服务，无broker/无网关/无消息队列 | `docker-compose.yml`（唯一 `services.xiaozhi`） |
| 端口只映射 8000/tcp | `docker-compose.yml` `ports: ["8000:8000"]` |
| CPU 配额 3.5 核（留 0.5 核给宿主） | `docker-compose.yml` `cpus: "3.5"` |
| 内存唯一数据卷是模型目录，无 broker 数据 | `docker-compose.yml` volumes |
| 有管理台、per-device overlay、多设备注册表 | `README.md:13`、`app/registry/` |
| 配置文件有 `expected_token`（可空=不校验） | `ws/mod.rs` 鉴权段 |
| **无任何文档提到设备规模、并发数、用户数** | 全仓 grep 无结果 |

**从上述可推导的用户画像**：**单人/小家庭自建自用**，部署在局域网，设备数**个位数到十几台**（管理台支持多设备，但 CPU/内存配额说明不是商用规模）。

**逐条对照 MQTT+UDP 的三类典型收益**：

| 典型收益 | 本项目是否需要 | 判据 |
|---------|--------------|------|
| **省流量**（按连接计费的云 IoT 场景） | ❌ **不需要** | 自部署、局域网、不按流量计费。WebSocket 走 TCP 内网，流量成本为零。MQTT+UDP 省的那点带宽在局域网里**没有任何财务意义**。 |
| **省连接数**（C10k+ 公网服务） | ❌ **不需要** | N5105 单机、`asr.max_concurrent` 是个位数（`plugins/asr/mod.rs:46-53`）、设备数十台以内。WebSocket 一连接一会话，当前**远未触及瓶颈**。 |
| **穿NAT**（设备在 NAT 后、公网直连） | ⚠️ **存疑，见下** | 需分情况。 |

**「穿 NAT」这一条要仔细拆**：

MQTT+UDP 确实能穿 NAT（设备主动外连，服务端回包走已建立的映射）。**但 WebSocket 也能穿**——本项目当前的接入路径是：

```
设备 → HTTP POST /api/ota（拿到 ws:// 地址 + token）→ 设备 WS Connect → 语音会话
```

WS Connect 由**设备主动发起**，同样穿 NAT（只要设备能出网到服务端）。设备侧 WebSocket URL 是服务端通过 OTA 下发的（`ws/ota.rs:191`），**不硬编码**——`ota.rs:84` 甚至专门处理「设备可能连不上」的情况（url兜底 localhost）。

**所以：MQTT+UDP 在穿 NAT 上相对 WebSocket 没有增量优势。** 两者都要求「设备能主动出网到服务端」。区别只在「服务端要额外开一个 UDP 端口」。

> ⚠️ 关键点：MQTT+UDP 穿 NAT 是真的，但 WebSocket **也**能穿。提案隐含的「MQTT+UDP 能穿 NAT 而 WS 不能」这个前提**不成立**。

**现有 WebSocket 在局域网直连下的实际不足**（诚实列举）：

| 不足 | 严重度 | 是否值得用 MQTT+UDP 解决 |
|------|--------|------------------------|
| TCP 头开销（每帧 60ms 音频多 40+ 字节，约 3% 带宽） | 极低 | ❌ 不值得 |
| TCP Nagle/延迟抖动（`app/downlink.rs:35` `DownlinkPacer` 已在专门处理） | 已缓解 | ❌ 已有专门机制 |
| 单连接单设备，跨设备广播需自己实现 | 低 | ❌ 管理台有 registry，不是问题 |
| 长连接占用（每设备一个 TCP） | 低（几十台） | ❌ 非瓶颈 |

**结论：零个真实痛点。** 三类收益要么不需要、要么已由现有机制覆盖。**唯一能算作「痛」的是 §2.4 的跨通道竞态——而那是引入 MQTT+UDP 后新造出来的问题，不是现有 WebSocket 的问题。**

### 6.2 问题 10：有没有更简单的替代方案？

**有，而且便宜得多。**针对每一个可能动机：

| 动机 | 更简单的解法 | 成本 |
|------|-------------|------|
| **设备 NAT 后连不上** | ① 服务端本来就监听 `0.0.0.0`（`docker-compose.yml` 注释：服务器永远监听 0.0.0.0），确认端口映射即可<br>② 同一局域网：直接用内网 IP，`ws/ota.rs:84` 已有url 兜底逻辑<br>③ 跨网：端口转发 / frp / cloudflared tunnel | **分钟级配置，零代码** |
| **内网设备走 MQTT** | 单独跑一个 mosquitto，设备仍走 WS。本项目**完全不受影响**——MQTT 是设备的选项，不是服务端的义务 | **零代码**（`docker run mosquitto`） |
| **省流量** | 局域网内无流量成本。若真在计费网络：调 `[audio].downlink_sample_rate` / `downlink_frame_duration_ms` 即可（`ws/mod.rs:290` 配置项），比换传输层见效大得多 | **改配置，零代码** |
| **想验证固件 MQTT+UDP 能力** | 固件默认值就是 `CONNECTION_TYPE_MQTT_UDP`，改一行 Kconfig 就能编译出 MQTT 固件；服务端**继续跑 WS**（两者互不影响），只是没有对端 | **编译成本，无服务端成本** |
| **设备数变多，WS 撑不住** | 当前 `asr.max_concurrent`（`plugins/asr/mod.rs:46-53`）是可配闸门，真到瓶颈时先看 ASR 推理并发和 CPU 配额（`cpus: "3.5"`）——**N5105 的 CPU 通常比传输层先到瓶颈** | **先扩 CPU** |

**核心判断**：
> 任何一个动机都有**成本低一到两个数量级**的替代方案。而 MQTT+UDP 的成本是「新增一个与 `app/ws/` 同量级的传输网关 + 一个常驻 broker + 固件重编译 + 一类新引入的竞态 bug」。**投入产出比不成立。**

---

## 7. 建议的落地路径或触发条件（问题 11–13）

### 7.1 结论（问题 11）

> **不做。** 不是「先技术验证」，是**当前完全不做**。

**理由（按权重排序）**：

1. **【决定性】前提不成立**：MQTT+UDP 与 WebSocket 在固件里**编译期互斥**（`CMakeLists.txt:136-140`、`application.cc:368-372`、`wifi_board.cc:116-125`）。用户当前的固件是 WS 构建，服务端建MQTT 能力**对现有设备零收益**。要产生收益，用户得**先改固件重编译**——那么正确的产品动作是告诉用户「你可以自己编译 MQTT 固件，官方 MQTT 服务端在 xiaozhi.me」，而不是在自己的服务端里重建官方生态。

2. **【决定性】需求不成立**：三类收益（省流量/省连接数/穿 NAT）在 N5105 单机自部署、几十台以内、局域网场景下**一条都不需要**。而「WS 不能穿 NAT」这个隐含前提经核实**不成立**（WS 由设备主动发起，同样能穿）。

3. **【决定性】成本被低估**：「实现一份 Transport」只需 ~500 行，但真实成本是 **~750 行 + 一个常驻 broker + 固件重编译指引 + 一类新竞态 bug**，且要打破「纯 Rust 单二进制、单容器」的项目定位（`README.md:3`、`docker-compose.yml`）。

4. **【加重】不可观测性退化与项目投入相悖**：UDP 丢包在设备端（`mqtt_protocol.cc:206-208`）和服务端**双向不可见**。本项目已投入五阶段延迟分位（`events.rs:44-58`）、TurnTrace（`events.rs:78-143`）、metrics 聚合（`metrics.rs:132-157`）。引入一个「静默丢包」的传输通道，与整个可观测性方向相反。

5. **【加重】存在未解的前置未知**：固件从不`Subscribe`（全仓 0 命中，`e2c5578` 删除了 `subscribe_topic_`），下行寻址方式藏在 `78/esp-ml307` 组件内（**该组件不在本仓库，未验证**）。**连「往哪个 topic 发下行」都还答不出来**——这是任何实现的第一步。

6. **【加重】跨通道竞态是确定性丢包**：`tts start`（MQTT）先于 UDP 首包的窗口内，设备 `application.cc:377-383` 会**静默丢弃**音频。WS 路径完全无此问题。这是引入后必踩、难查的坑。

### 7.2 如果被要求做：最小落地路径（**不建议，但备此案**）

> ⚠️ 下列路径**不建议执行**，仅备主理人/用户坚持时的参考。**每一步都必须先做前置验证，不可跳步。**

**阶段 0（前置未知消解，必做，约 0.5 天）**
- 读 `78/esp-ml307` 组件源码，确认：`Mqtt::Connect`/`OnMessage` 内部订阅的 **topic 到底是什么**（§2.1 关键发现 4）。
- 若订阅 topic 无法从 `publish_topic`/`client_id` 派生 → **直接终止**（服务端无法定位下行目标）。

**阶段 1（只做文本控制，不碰音频，约 1 周）**
- 只让 `send_text` / 文本上行走通，音频**仍要求设备走 WS**。
- 但因固件编译期互斥（§2.2），**此阶段在物理上无法单独验证**——只能靠真机+改固件。**这是本路径最大的结构性问题：无法分层验证。**

**阶段 2（接UDP 音频，约 1–2 周）**
- UDP 封装 + AES-CTR + seq 校验 + 空闲超时 + 跨通道顺序 barrier。
- 需要 `docker-compose.yml` 加 UDP 端口映射 + `ota.rs` 下发 `mqtt` + `hello` 回 `udp.*`。

**阶段 3（broker 决策，约 1 周+ 部署复杂度）**
- 内嵌 `rumqttd` vs 外部 mosquitto——**这是架构决策，不是实现细节**，需主理人拍板。
- 内嵌则 `rumqttd` 与 axum 共存、`hello` 中 `key`/`nonce` 的下发时机、安全模型（MQTT 明文 vs TLS 8883）都要重新设计。

**阶段 4（可靠性与回归，约 2 周）**
- 丢包/乱序/重连/时钟漂移测试。WS 路径的所有测试用例（v1/v2/v3 帧、pacer 预算、OTA、management 页）需全部重跑对比。

**阶段 0 之前的投入产出是负的；阶段 0 本身就有终止风险。建议不要启动。**

### 7.3 触发重新评估的条件（问题 13）

**满足任意一条时，重新评估（先做阶段 0 前置验证，不直接开工）：**

| # | 触发条件 | 为什么是分界点 |
|---|---------|--------------|
| **T1** | 用户要求**设备在 NAT 后/跨公网**接入，且**明确表示端口转发/tunnel 方案不可接受** | MQTT+UDP 唯一有实质差异的场景（但需先确认 WS 真的不够——本文核实是够的） |
| **T2** | 设备规模 **> 100 台**，或单设备并发会话数导致 **WS 连接数成为实测瓶颈** | 「省连接数」开始有实际意义（当前个位数~十几台，远未到） |
| **T3** | 出现**按流量计费**的部署（云主机 + 按GB 计费），且音频流量占成本显著比例 | 「省流量」开始有财务意义 |
| **T4** | 用户明确要求跑**官方 xiaozhi 固件**（而非自己编译 WS 版），即需要与官方生态兼容 | 这时正解是**接入官方 MQTT 服务端**，不是自建 |
| **T5** | 出现**必须用 MQTT 的既有设备**（如某厂商硬件固化了 MQTT，出厂无法改） | 唯一「不做就完全不可用」的情形 |

**不构成触发条件**（**明确排除**）：
- ❌「多设备支持更好」——WebSocket 已有 `registry` + per-device overlay。
- ❌「架构更完整/更先进」——`transport.rs:1-3` 的注释已说明设计意图是「接新设备形态时实现一份 trait 即可」，**这是为将来预留，不是现在必须填的坑**。
- ❌「proto文件里有 MQTT 字样」——那是注释/文档（7 处命中全是注释，`ws/ota.rs:20` 是「有意省略」的说明）。
- ❌ 「省流量」——局域网无流量成本。

---

## 8. 未验证声明

以下各项**未能在本次评估中验证**，不应被当作已确认事实：

| # | 未验证项 | 原因 | 影响 |
|---|---------|------|------|
| 1 | **`78/esp-ml307` 组件内部行为**（`Mqtt`/`Udp` 类的真实 API、`Connect`/`Publish`/`OnMessage` 实现、**是否隐式 Subscribe 及订阅哪个 topic**） | 组件由 `main/idf_component.yml:14` 声明依赖，源码在 ESP-IDF component registry，**`managed_components/` 未提交到本仓库**，本地无副本（已尝试 `find` 未找到） | **决定性**：下行寻址方式未知，见 §2.1 关键发现 4、§3.3 |
| 2 | **UDP 16 字节头的完整字段布局**（尤其 `[1]`、`[4:12]` 的含义） | 仅从 `mqtt_protocol.cc:119-134` 的写入偏移**推断**，无服务端对照实现可交叉验证（官方服务端未开源） | 中：实现时需与官方 PoC 交叉验证 |
| 3 | **AES-CTR 的精确语义**（`nonce` 复用为 16B IV、每次包`nc_off=0`/`stream_block=0` 是否意味着每包重置密钥流、`stream_block` 为何每包清零） | 同上，仅从代码推断 | 中：错则音频解不开 |
| 4 | **MQTT 重连是否真由 `MQTT_RECONNECT_INTERVAL_MS` 控制** | 宏定义于 `mqtt_protocol.h:19`，但本文件内无使用点，实际逻辑在组件内 | 低 |
| 5 | **`udp_->Connect()` 是「UDP connect」还是纯 bind** | 实现在组件内（未验证）；固件只调用了 `Connect(server, port)` | 中：影响服务端要不要处理 ICMP/源地址变化 |
| 6 | **设备的实际部署数量、用户身份、是否公网暴露** | 仓库无任何相关文档或配置（已 grep `README.md`、config 示例、docs） | 中：§6 的用户画像是**从部署形态推断**，非实测 |
| 7 | **官方服务端（api.tenclass.net）的 MQTT 行为** | 未实测官方服务端 | 低：我们的目标不是复刻官方 |
| 8 | **ML307 蜂窝模块路径** | `mqtt_protocol.cc:7-8` include 了 `ml307_mqtt.h`/`ml307_udp.h`，但当前 `wifi_board.cc:127-133` 走 `EspMqtt`/`EspUdp`；蜂窝板型的实现未读 | 低：WiFi 板型是主路径 |

---

## 9. 一句话总结

> 固件侧的 MQTT+UDP **实现完整、加密完备、已被官方设为默认**，但它与 WebSocket **编译期互斥**、需要一个**本项目不愿引入的常驻 broker**、带来**无反馈的 UDP 丢包**与**跨通道确定性丢包竞态**，而它解决的三类收益（省流量/省连接数/穿 NAT）**在 N5105 单机自部署场景下一条都不需要**（且「WS 不能穿 NAT」这个前提经核实不成立）。
>
> **`trait Transport` 够用（6 方法无缺口，只需一个内部合并器），架构上无障碍——但正因为架构无障碍，才更该克制：这是一次没有需求支撑的 ~750 行 + 一个中间件投入。**
>
> **建议：不做。** 保留 `transport.rs` 已有的抽象作为将来扩展点（这正是它设计的用途），把这份评估的触发条件（§7.3）留给用户，让**需求**来决定，而不是让提案来决定。
