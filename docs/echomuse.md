# EchoMuse 设备入口（Echo Dot 2 / emOS）

> 当前功能说明；协议权威来源是 EchoMuse 仓库 `docs/device-controller-interface.md`（以代码为准），实现细节以 `server/src/device/echomuse/` 源码与测试为准。

## 上游参考版本（增量比对锚点）

实现与文档所对照的上游快照，升级 EchoMuse 时先 diff 这些再动代码：

- **版本锚点**：[wilbowes/EchoMuse](https://github.com/wilbowes/EchoMuse) **v2.18.1**（2026-10-15 打标，commit `b18a0befb3492f335ac0a71cdd9cd15390b6e9e8`；官方标注配套 controller 2.26.1）。设备在跑的固件即此版 release 资产 `server`（sha256 前 12 位 `2f3523088af1`，本地缓存 `~/emos-install/server.fw`）。核对时间 2026-10-19。
- **重点参考路径**（上游更新时优先逐个 diff）：
  - `emos/README.md` —— emOS 启动链路（init 12 级 LED → wpa_supplicant → 服务表）、安装与启动确认/回滚机制
  - `controller/device_payloads/start_server.sh` —— 设备侧监督脚本正本（A/B 槽软链、15s 退避、3 次失败切槽回滚）
  - `controller/em_shell.py` —— shell plane 文件下发（base64 heredoc 流 + md5 校验 + rename 原子替换）
  - `controller/em_payloads.py` —— payload 清单与推送时机（on_connect / OTA / Maintenance）
  - `controller/em_routes_provision.py` + `static/dashboard.jsx` —— provisioning 串口下发通道
- **比对方法**：`git log v2.18.1..<新tag> -- <路径>` 逐路径看变化；协议字段增删对照 `docs/device-controller-interface.md` 与本侧 `frames.rs`/`routes.rs` 能力协商子集。只升固件（release 资产）不动服务端代码；协议有变才需要改 `server/src/device/echomuse/`。

emOS Web 管理的额外参考锚点是上游 [`858987c272329a41d385835439bd3e2da46719ac`](https://github.com/wilbowes/EchoMuse/tree/858987c272329a41d385835439bd3e2da46719ac)（2026-10-20 核对）：`controller/em_shell.py` 的按需 shell、`device/internal/client/control.go` 的控制消息与 `device/internal/config/config.go` 的字段类型，以及 `emos/README.md` 的日志布局和安全重启。设备 release tag 不一定包含 controller 源码，不能把两者版本号当作同一快照。

## 是什么

把一台刷了 emOS 的 Echo Dot 2（EchoMuse 固件）作为语音卫星**直接接入本服务端**：不经 Home Assistant、不跑 EchoMuse 官方 controller。设备经 mDNS 发现本服务器，走 `/em/control` + `/em/data` 双 WebSocket 接入，与 xiaozhi ESP 设备**共用同一条** SenseVoice ASR → LLM/技能级联 → TTS 流水线。

## 配置（`[echomuse]`，改动需重启）

| 字段 | 默认 | 说明 |
|---|---|---|
| `enabled` | `false` | 挂载 `/em/control`、`/em/data` 路由并广播 mDNS `_emcontroller._tcp`；关闭时零行为变化 |
| `token` | 空 | 校验设备 `X-Em-Token` 头；留空不校验。生产建议设置（同网段任何设备都能按此协议连进来） |
| `ble_proxy` | `false` | 开启蓝牙代理：ack 向声明了 `ble_connect` 的设备回声明 `ble_connect` + `ble_adverts_data`，接入时推送 `bleProxyEnabled`/`bleProxyConnections`（每设备 settings 可覆盖）；关闭时 ack 不声明、`0x06`/`0x08` 按未知帧忽略、管理面 BLE 端点 404——零行为变化 |
| `plain_port` | `8767` | 控制器专用端口（官方接口对齐）：enabled 且 >0 时**额外**监听设备面路由（`/em/*` + `/shell`），mDNS `plain_port` 也广播它；0 = 关闭（仅主端口）。绑定失败只告警、mDNS 回退广播主端口；主端口始终继续服务 `/em/*` |
| `welcome` | 含使用提示 | 连接欢迎语：register + data 认领、会话建立后**立刻**说一次（`client_id="echomuse"` 的会话专属，走唤醒问候同一预制话术链路，合成一次即落盘 `hot-audio/` 的裸 PCM（引擎原样 16k，不重采样）免复合成）；留空 = 不说；每次会话建立（含断线重连）都会说一遍。管理台「配置 → EchoMuse」可编辑 |

mDNS 广播地址**无配置项**：启动时动态探测部署机默认出口的局域网 IPv4，部署在哪台机器、是什么 IP 就广播什么。docker 容器部署会探测到容器内网 IP（设备连不上），请改用 host 网络部署。

**官方端口对齐（`plain_port` 的来由）**：上游 emOS 固件（`emos/init/init.c` 的 `tcp_peer()`）只把「ESTABLISHED 且远端端口 8767/8770」的连接识别为 controller——这是设备串口 banner `controller` 字段的唯一判据，2026-10-20 对 v2.18.1 源码核对为**全固件唯一**按端口判连接的位置（其余逻辑不依赖该状态，纯显示）。自建服务器若只听主端口（如 80/8000），banner 恒显 `not connected`；服务端默认在 8767 额外监听并把 mDNS `plain_port` 指向它，设备经发现自动迁移到官方端口，banner 如实显示 `192.168.x.x:8767`。host 网络部署下 8767 即宿主端口，无需额外映射。

会话侧固定参数：上行 mic 16kHz / 下行 speaker **48kHz**，均 mono `S16_LE` / 40ms / 二进制协议 v1。采样率都是协议常量（不跟随 `[audio]` 里给 ESP 的 24k/60ms）：上游控制器用 ffmpeg 把 TTS 直接解码成 48kHz mono 发 `0x02`，设备端 DAC 也是 48kHz 且**不做重采样**——按 16kHz 发会变成 3 倍速的尖叫声。

## 协议映射（EchoMuse → xiaozhi 会话语义）

| EchoMuse 事件 | 会话语义 | 说明 |
|---|---|---|
| data 面 Text identify（拨通后首帧） | 按 `device_id` 精确认领 data 出站口 | data 面唯一文本帧；此后恒为二进制，再收 Text 忽略不断链 |
| `oww_wake`（设备本地唤醒词命中） | `listen state=detect` → `listen state=start` + 下发 `mic_start{lock_mic:true}` | bounded turn：设备自己做 VAD，结束发哨兵；`detect` 先投（`[wakeup]` 唤醒问候语的触发点，问候关闭时仅多一行会话日志）；detect 的 `text` 携带来源标签 `"本地唤醒词"`，echomuse 层同步打 `触发来源：本地唤醒词` 日志 |
| `button`（动作键） | 同上 | 无唤醒词模型时的兜底入口；detect 的 `text` 携带 `"action 键"`，echomuse 层同步打 `触发来源：action 键` 日志——会话层日志（`收到唤醒上报（唤醒词/来源：…）`）据此区分"叫醒的"还是"按键开的" |
| `0x01`/`0x07` mic PCM（16k S16LE） | 上行 Opus 二进制帧 | 服务端用同一 `OpusFrameEncoder` 编码，攒满 60ms 一包；`0x07` 会话头（session u32 + seq u16 BE）已剥离 |
| `0x04`（VAD 结束） | 补齐残 PCM 后投 typed `EndOfUtterance(Speech)` | core 合并已切段与 VAD flush 尾段、reset 后识别；立即允许下次唤醒，不等回复 EOS |
| `0x05`（无语音超时） | 丢弃残 PCM 后投 typed `EndOfUtterance(NoSpeech)` | core 丢弃已切段与尾段、reset，不触发 ASR/LLM；重复/空闲哨兵幂等 |
| TTS PCM（typed `send_audio`） | `0x02` speaker PCM | 重采样到 48kHz，切 40ms（3840B）一块；`tts stop` 时残块补零 |
| `ServerMessage::Tts { state:"stop" }`（JSON `type=tts,state=stop`） | 残 PCM 补零至一块 `0x02` 后发一条 `0x03` speaker EOS | 按 JSON 结构解析，不匹配伪造 `tts_stop`/文本子串；其余服务器 JSON 不下发 |
| 会话 `close` | `Shutdown` → control 任务退出、连接关闭 | 与 WsTransport 的粘滞 `Closed` 同语义 |

有界捕获 gate 属于当前 control，data claim 捕获同一对象；data 断线仅经同会话探针通过的 `hub_restore_data` 取消本轮，旧 control/data 不得清理替换会话。下行轮询保留有界话语末包与结束事件的顺序，普通 ESP `listen stop` 不变。实现与外置契约回归见 [`turn.rs`](<../server/src/device/echomuse/turn.rs>)、[`uplink.rs`](<../server/src/device/echomuse/uplink.rs>)、[`transport_tests.rs`](<../server/src/device/echomuse/transport_tests.rs>)；默认 feature 音频编解码为编译桩，测试通过不等同真实 sherpa/固件联调通过。

能力协商：`ack.features` 只声明 `mic/speaker/leds/buttons/oww_trigger/oww_local_only`。**不声明** `listen_session`（不走 `0x07` 私听会话）、`audio_mix`（不做音乐面）——按协议"未声明即降级"静默关闭；未知帧型/未知 JSON 双向忽略。`ble_connect` + `ble_adverts_data` 仅在 `[echomuse].ble_proxy=true` **且**设备 register 声明 `ble_connect` 时条件追加（协议要求双向声明才启用）。

**唤醒声音的分工**：设备唤醒时本地先播一声自带提示音（`wake_cue`，音量/开关走管理台「EchoMuse → 唤醒词设置」的 `wakeSoundLevel`）；服务器侧问候语由全局 `[wakeup]` 控制（默认「我在」，支持换行多句随机，EchoMuse 唤醒经 `detect` 代发触发）——两者可并存，嫌设备提示音刺耳就把它调 quiet/关掉，留服务器问候。

### 会话日志口径（默认 `RUST_LOG` 未设 = info 档）

一次"唤醒 → 对话 → 回复"在 info 档只有四类关键行，**收到什么事件、说了什么、下发了什么**一目了然：

```
[echomuse] device G090… 触发来源：本地唤醒词，开轮          （按键则显示 action 键）
session … 收到唤醒上报（唤醒词/来源："本地唤醒词"）
session … 用户：现在几点了（识别 213ms）
session … 回复（耗时 1820ms / 音频 4100ms / LLM 87 字 / 工具 0 个，下发 2 句）：现在是下午三点二十分…
```

分段延迟/水库水位/下载缓冲/吞吐/首帧 TTFA/下行平顺等**每轮诊断明细已全部降为 debug**（异常时除外：断音与吞吐 <0.98× 恒为 warn）。排障时 `RUST_LOG=debug`（或 `RUST_LOG=xiaozhi_server_rust=workflow=debug`）恢复全量。

## 蓝牙代理（`ble_proxy=true`，EchoMuse `ble_connect` / `ble_adverts_data`）

把 Dot 当作服务端的 **BLE 代理**：设备侧被动扫描 + 最多 3 条 GATT 链接（协议上限），服务端采集并直驱外围 IoT 设备。本次对接核对固定快照 [`858987c` 的 bridge.go](https://github.com/wilbowes/EchoMuse/blob/858987c272329a41d385835439bd3e2da46719ac/device/internal/bluetooth/bridge.go)（服务树、通知、slots 和错误结果）及同快照 `docs/device-controller-interface.md` 的 `0x06/0x08` 帧契约；服务端实现见 `server/src/device/echomuse/ble.rs`、`ble_http.rs` 与 `ble_request.rs`。

- **采集**：设备把被动扫描的广播批次经 `0x06` 帧上报（条目 `{"addr","addrType","rssi","data"(base64)}`，旧固件回退 control 面 `ble_adverts` JSON，同样接收），服务端按 addr 合并并附 `seenAt`，上限 256 条按 seenAt 淘汰；`slots` 事件（连接槽位 `free/limit/addrs`）与 `notify`/`disconnected` 事件存进每设备事件环（容量 100，`state` 端点取尾 50）。
- **控制**：管理面 `POST .../ble/gatt` 按 body 构造 `0x08` 请求下发，`req` 自动分配，等待设备同号 `result`（connect/disconnect/services/read/write/slots 六种 `t`；`value` 走标准 base64）后返回，并附服务端的 `device_id/session_id`。`ok:false` 是设备明确拒绝，不是操作成功；`slots` 应答是 `t:slots`、不含 `ok`。设备自守时 connect 20s、ATT 30s 超时先答，服务器 35s 兜底；管理台请求超时为 45s，不在服务端返回前提前放弃。
- **启用链**：服务端 `enabled/ble_proxy` 开启、非空管理 token 与设备 token、设备已批准且声明 `ble_connect`、当前 control 已认证。GATT 还要求设备配置 `bleProxyEnabled/bleProxyConnections` 有效值均为 true（全局基线默认 true，单台 settings 可覆盖）及 data 面已认领；不支持的设备、未批准、凭据缺失、离线或仅扫描模式均明确拒绝，不靠固件静默超时。读取已有广播/事件不要求两项单台开关打开。
- **生命周期**：BLE 链路状态挂在设备会话上——control 连接断开设备即按协议丢弃全部 BLE 链接，服务端同刻清状态，等待中的 GATT 请求回错。hub 条目存活到会话结束，但控制面在线不等于 GATT ready；data 认领取走 `data_rx`，收尾在同一 hub 锁内清 BLE 状态、推进 `data_epoch`、丢弃断线前已排队帧后才归还；旧 GATT 不随数据重拨重放，旧 capture 即使同 control/channel 立即重新认领也拒绝下发。数据面恢复认领后才可再次下发。HTTP 取消或超时只回收服务端等待，不撤销已经排队到固件的操作，必须核对结果、不能盲目重试写入。
- **不做**：无配对/加密（协议规定 Security Request 答 Pairing Not Supported，加密属性 `att:5` 失败）、不做 HA/ESPHome 转发面（那是官方控制器的用法）、`0x06` 遵守"bounded turn 期间不写遥测"的设备侧规则（服务端不干预）。

### BLE 管理面 API（同 `/api/echomuse/*` 鉴权；`ble_proxy=false` 时统一 404 `{"error":"ble proxy disabled"}`）

- `GET /api/echomuse/devices/{id}/ble/adverts?session_id=<当前会话>` → `{"device_id","session_id","updatedAt","count","adverts":[{"addr","addrType","rssi","data","seenAt"}...]}`，广播为被动采集的手动快照，不代表设备当前一定在场或可连接。
- `GET /api/echomuse/devices/{id}/ble/state?session_id=<当前会话>` → `{"device_id","session_id","slots":{free,limit,addrs}|null,"events":[notify|disconnected ...]}`，事件最旧在前、非持久化；刷新不会消费或重新订阅通知。
- `POST /api/echomuse/devices/{id}/ble/gatt`，body 例如 `{"t":"connect","addr":"aa:bb:cc:dd:ee:ff","addr_type":0,"session_id":"当前会话"}`；其他 `t` 为 `disconnect/services/read/write/slots`。`read/write` 使用十进制 ATT handle 1～65535；write 的 value 为标准 base64、最多 512 字节，response 必须为 bool（缺省 true）；实际外设 MTU 更小时仍可能返回 too_long。`addr_type` 为整数 0（public）或 1（random），不能从 MAC 格式猜测。
- GATT 返回 200 + 设备应答并附 `device_id/session_id`，其中 `ok:false` 必须按失败展示；`slots` 为特殊事件应答，无 ok。客户端始终带上当前 session_id，切设备/服务地址/token/外围设备/会话变化会丢弃旧回包与写入确认；服务端捕获会话并在同一 BLE 状态对象中登记与清理请求，不会把旧超时清理应用到新连接。过期 session 返回 409，语法错误 400、离线/代理未就绪 409、超时 504；具体错误由 `{error,code}` 说明，不能自动重试写入。单个代理最多 16 项在途请求，取消/失败/超时均回收。

### 通用 BLE 面板：从发现到控制

入口：**EchoMuse → 已批准设备 → 蓝牙代理**。这是直接使用 EchoMuse GATT 桥的管理面，不是 Home Assistant 蓝牙代理，也不是任意品牌的即用型开关。

1. 在配置中启用 `[echomuse].enabled/ble_proxy`，填写非空 `[server].expected_token` 与 `[echomuse].token`，重启服务端并让设备重新连接完成能力协商；使用 URL 编码后的管理 token 打开 Web 管理台。必须能在 register 看到 `ble_connect`，否则刷新面板不能凭空补上固件能力。
2. 在代理面板保存单设备「代理扫描」和「GATT 连接」开关。设置走已有 settings 部分更新，不覆盖唤醒词/BLE 之外的设置；响应仅说明保存或排队，并非固件已执行。
3. 手动刷新广播，按地址、地址类型、RSSI 与 seenAt 选外围设备，也可手填 MAC。广播可见不意味着支持连接；battery/beacon、私有加密协议及正在被手机占用的设备不一定能控制。
4. 查询槽位，再连接目标；服务发现展示 `services[].uuid/start/end`、`chars[].uuid/handle/value_handle/props` 及 `descs[].uuid/handle`。**characteristic 的 handle 是声明句柄，读写必须用 value_handle**；描述符按自身 handle 访问。UUID 用于理解协议，最终下发的是 ATT handle。
5. 选特征后读取，返回原始 base64 并显示 Hex；写入填 Hex（可有 ASCII 空白，不接收 `0x`、逗号、Unicode 数字等混合表示），选择有响应/无响应模式，核对**代理会话、外围地址、handle、完整字节和 response**后再次确认。任何草稿变化都会使旧确认无效。`response:true` 的成功表示 ATT 写应答，不代表 IoT 业务动作完成；`response:false` 更不代表外围设备已确认。
6. 通知/指示通过相应特征的 CCCD（UUID `2902` 或完整 128 位等价 UUID）开启：选该描述符 handle，按设备支持写 `01 00`（notify）、`02 00`（indicate）或 `00 00`（关闭）；这是普通受确认保护的 write，不存在额外 subscribe 指令。手动刷新事件查看 `notify` 的 addr/handle/value/ind 及断开事件，无自动高频轮询。

**控制协议边界**：服务发现只给出句柄和属性，不会生成品牌指令。开关、亮度、温度等真正的控制值必须来自目标设备协议；示例 Hex/CCCD 不能当作所有 IoT 设备的通用开关。无配对、无加密、无 PIN/bond 管理；要求配对/加密的属性会失败。与真实 IoT 设备联调前先核对 UUID、权限、数据长度及字节序，禁止对未知句柄盲写或把 HTTP 200 当作硬件动作成功。

会话入口无 xiaozhi hello 交换：`session_params()` 固定构造参数后直接进 `run_session`（`device::echomuse::routes::control_session`）。

## 刻意不做的范围

不提供任意 root 终端、文件上传、镜像刷写、恢复出厂或 recovery 重启；`/shell/{device_id}` 仅用于下述固定系统诊断与安全重启，不接受客户端命令字符串。TLS/wss、音乐面（`0x04/0x05` 下行）、Sendspin、设备音频链路参数（EQ/AEC/波束）推送仍不实现。设备链路目前为 `ws://` 明文，受限 shell 必须配置非空设备 token，且应仅在可信局域网使用；不得把 token/同源 IP 检查当作加密。蓝牙代理范围仍是直驱 GATT + 广播采集，无配对/加密、无 HA 转发面。

## 部署前置（一次性，服务端之外）

1. Dot 用 [firebreak](https://github.com/bboe/firebreak) 解锁（风险自负，可变砖）；
2. 用 EchoMuse 官方 controller 向导刷入 emOS 并配网（设备凭据文件持久化 endpoint/token）；
3. 服务端开 `[echomuse].enabled = true`、设置 `token`，与 Dot 同网段重启——mDNS 广播被设备发现后自动接管；
4. 联调：`python3 tools/echomuse_mock_device.py --host <server> --port 8000 --token <token>`（模拟 register → 唤醒 → 上行 PCM → 收 `0x02…0x03`）。

## 管理面 API（`/api/echomuse/*`）

经 `auth_ok` 鉴权（`Authorization: Bearer <token>` 或 `?token=`）；`[echomuse].enabled=false` 时除鉴权外一律 404（`{"error":"echomuse disabled"}`）。设备登记持久化在 `<memory.db_path 父目录>/echomuse_devices.json`（与配置库同卷，默认 `/data`），register 时 upsert，`online`/`connected_secs`/`session_id` 不落盘、由 hub 实时合并。`session_id` 是当前 control 的随机观测标识（非鉴权凭据，离线为 null），同一设备快速重连也会变化；列表游标在快照前读取，并发事件原子递增，避免漏刷新。

- `GET /api/echomuse/devices` → `{"enabled":true,"version":<u64>,"devices":[{"device_id","label","approved","version","board","base_os","capabilities","first_seen","last_seen","online","connected_secs","session_id","settings"}]}`（registry 全表，按 device_id 排序；`connected_secs` 离线为 `null`；`capabilities` 为最近一次 register 上报的能力表；`version` 是设备面变化号，与管理台 WS hello/事件帧同源）。
- 变更通知走管理台 WebSocket `GET /api/admin/ws`（鉴权同管理面 HTTP：`Authorization: Bearer <token>` 或 `?token=`；协议实现见 `server/src/gateway/ws/admin_ws.rs`）：连接建立后服务端**立刻**发 `{"type":"hello","version":<当前版本>}`；设备上线/离线/批准/设置/删除使版本变化即推 `{"type":"echomuse_devices_changed","version":<新版本>}`；客户端可发文本帧——`{"type":"ping"}` 回 `{"type":"pong"}`（应用层心跳，浏览器 WS 无协议层 ping API），未知 type / 非 JSON / 非对象 / 二进制帧回 `{"type":"error","error":"<原因>"}` 且**连接保持**；Close 帧结束。客户端凭事件帧刷新设备列表（`GET /api/echomuse/devices` 的 `version` 用于对齐增量）。原 `GET /api/echomuse/devices/wait?after=<n>` 长挂轮询端点已删除；`[echomuse].enabled=false` 时本通道照常可用（通用管理台事件通道，只是版本不动）。
- `GET /api/echomuse/devices/{id}` → 单台（同上形状）；不存在 404 `{"error":"device not found"}`。
- `POST /api/echomuse/devices/{id}/approve`，body `{"label":"客厅音箱"}`（label 可省）→ 200 + 该设备 JSON（`approved=true`，label trim 后非空才改名）；不存在 404。
- `POST /api/echomuse/devices/{id}/command`，body `{"action":"wifi","ssid":"...","psk":"..."}`：在线（hub 有 control 通道）返回 202 `{"queued":true}` 并下发 `wifi_change`；不在线 409，action 非法 / wifi 缺 ssid 400。旧 `action:reboot` 不再下发固件不认识的消息，改用下述需要确认的系统重启接口。WiFi 当前仍只报告排队，不代表切换结果已确认。
- `POST /api/echomuse/devices/{id}/settings`，body 为**部分更新**的每设备配置，与已有键合并保存。白名单：`wakeWordEnabled`/`wakeSound`/`bleProxyEnabled`/`bleProxyConnections`（bool）、`owwModel`（string）、`owwOnDevice`（`off|shadow|on`）、`wakeSoundLevel`（`quiet|medium|loud`）、`owwThreshold`（大于 0、不超过 1 的数字）、`wakeMic`（0~6 整数）；白名单外、类型错或越界 400。旧持久化的本地唤醒 bool / 提示音数字在推送时兼容转换，不再发送会导致 Go 整条 config 解析失败的旧类型。在线返回 202 并排队 `config`（`wakeWordEnabled` 单独一条）；离线返回 200 保存待下次接入。`delivered:true` 只代表下发通道接受，不等于设备执行确认。
- **唤醒词模型（`owwModel`）语义**（对照上游 wilbowes/EchoMuse @858987c）：存储为空表示**设备默认** `hey_jarvis_v0.1`（上游 `device/internal/config/config.go` 的 `OWW_MODEL` 缺省，人读唤醒词 "hey jarvis"，面板据此显示默认唤醒词）；修改即随 `config` 下发 `owwModel`，设备 `Apply` 只认**非空**值——所以"恢复默认"必须显式发 `hey_jarvis_v0.1`，发空串不会重置。设备本地评分（`owwOnDevice=on`）只认设备已预装的分类器：官方内置共四款 `hey_jarvis_v0.1`/`alexa_v0.1`/`hey_mycroft_v0.1`/`hey_rhasspy_v0.1`（上游 `em_oww_assets.py` `STOCK_MODELS`，开通时全部预装；`timer`/`weather` 是意图模型不算唤醒词），切到设备没有的模型会听不到唤醒词（上游 #191）；本服务器不做上游 shell 面的模型自动分发，设备只消费本地已装的分类器，面板下拉列官方四款并追加服务端 `oww-models/` 的已上传清单（`GET /api/files?path=oww-models`，端点见下方"文件管理 API"一节），已存自定义值原样保留。人读名规则同上游 `display_name`：去 `_v<数字>` 后缀、下划线转空格。自定义模型全流程见下节。
- **唤醒词面板官方信息移植**（对照上游 `controller/static/dashboard.jsx` Wake word 卡，HEAD e2f2f4e）：「唤醒词检测位置」下拉用官方标签与顺序（`On this Echo` 官方推荐 / `On the controller`），`shadow` 是开发者诊断档、仅设备已处于该档时作为选项显示（服务端仍接受三值，恢复需设备先处于 shadow 或走 API）；选中档位附官方解释文案。灵敏度滑杆移植官方量程 **0.1~0.975、步进 0.025、Precise↔Eager 反向**（左严格右积极；上限 0.975 因 openWakeWord 分数是低于 1.0 即饱和的 sigmoid、判定为 `score≥threshold`，1.0 永不唤醒，官方实测 18,021 帧峰值 0.999），轨外历史值读数保真、把手钉在端点，拖动才写入量程内新值；服务端白名单仍是 `(0,1]` 不变。提示音附官方语义（听到唤醒词播放短促上升音），档位仅开关打开时显示。
- `DELETE /api/echomuse/devices/{id}`（客户端用等价别名 `POST .../delete`，NetworkModule 只有 GET/POST）→ 200 `{"removed":true,"device_id"}`；不存在 404。只删静态登记（批准/唤醒词设置一并清除），不断开在线会话，设备再次 register 会重新登记——用于清理 mock/联调设备。

### 文件管理 API（`/api/files/*`）

通用**服务端数据目录**（`<memory.db_path 父目录>`，默认 `/data`）维护入口：管理台"文件"页的数据源，也是自定义唤醒词模型的上传通道（见下节）。实现与边界见 `server/src/gateway/files/AGENTS.md`。鉴权与 `/api/echomuse/*` 相同（`Authorization: Bearer <token>` 或 `?token=`，`expected_token` 空 = 不校验）；**`[echomuse].enabled` 不作用于此处**。错误一律 JSON `{"error"}`；路径相对数据根，`..`/`.` 段、前导 `/`、`//`、反斜杠、NUL 一律 400，符号链接外逃拒绝。

| 端点 | 语义 |
|---|---|
| `GET /api/files?path=` | 列目录（path 缺省 = 根）→ `{"root":"/abs/root","path":<rel>,"entries":[{"name","dir","size","modified","protected"}]}`；目录在前按名排序，`modified` 为 Unix 秒；缺失 404，路径指向文件 400 |
| `POST /api/files/mkdir`，body `{"path":"a/b"}` | 建目录（父目录一并建；目录已存在幂等 200；同名文件 409） |
| `POST /api/files/upload?path=<必填>&overwrite=` | body 为裸文件字节：自动建父目录；已存在未覆盖 409（`overwrite=1`/`true` 覆盖）；空 body 400、超 64MB 413；受保护名 403 |
| `POST /api/files/delete`（body `{"path"}`）/ `DELETE /api/files?path=` | 删文件或目录（递归）；缺失 404；受保护 403；根目录 400。POST 别名给 Kuikly 客户端（只有 GET/POST） |
| `GET /api/files/download?path=` | 附件下载（octet-stream + `Content-Disposition: attachment`）；缺失 404，目录 400 |

`data.db`/`data.db-wal`/`data.db-shm` 受保护：**任意层级**最后一段命中即禁删、禁覆盖（403，那是 SQLite 配置+记忆库文件）。`oww-models/` 是唤醒词模型目录（不预创建，上传/建目录按需生成）；服务端只负责存储与列出，**不做**服务端→设备的模型分发。

### 自定义唤醒词流程（oww_forge 训练 → 分类器进设备 → `owwModel` 指向）

自定义唤醒词 = 任意 openWakeWord 兼容 `.onnx` 分类器，官方训练工具是上游同仓库的 `oww_forge/`（发布镜像 `ghcr.io/wilbowes/echomuse-forge`，README 2026-10-22 核对）。产物约 1MB、dnn/32 架构，形状硬约束（上游 `controller/em_oww_check.py`）：单 float 输入 `[N,16,96]`、单 float 输出 `[N,1]`——窗口非 16 时控制器能跑但 Echo 设备端不能。

1. **训练**：`cd oww_forge && docker compose -f docker-compose.deploy.yml up -d forge-ui`（`http://<host>:8769`；NVIDIA GPU 用 `:latest` CUDA 镜像，其余含 Apple Silicon 一律 `:latest-cpu`，Docker on macOS 无 GPU 透传）；CLI 等价 `forge.py assets|new|piper-voices|google-tts|build|test`。首次资产下载约 25GB（ACAV100M 负样本特征 ~17GB、验证集、AudioSet/FMA、MIT RIR、piper LibriTTS-R）。
2. **创建**：`forge.py new "hey biscuit"`（UI 可逗号分隔多个拼写变体覆盖口音，训练前可试听每个拼写），生成 `data/wakewords/<name>/config.yml`。选 **3–4 音节**短语（短词天生弱）；可调 `n_samples`（默认 30k，难词 50k–100k）、`custom_negative_phrases`（把实测误唤醒词加进去重训，最便宜的修复）、`target_false_positives_per_hour`/`max_negative_weight`（误唤醒 vs 漏唤醒的权衡）；口音/语言走 `piper-voices --language`、Google TTS 混入或真实录音（20–50 条即有可测提升）；非英语在 config 设 `language`。
3. **训练与验证**：`forge.py build <name>`（GPU 约 1–2h，CPU 过夜；可中断续跑，`build --from-step augment|train`），产物 `data/models/<name>.onnx`。`forge.py test <name> --wav <录音>` 验证：自己声音近 1.0、普通语音近 0.0；不达标回第 2 步加负样本/变体/录音重训。
4. **分类器进服务端 `oww-models/`，再自行进设备**：设备按 `owwModel` 值取**文件名主干**在 `/data/local/share/echomuse/oww/` 找 `<主干>.onnx`（上游 `device/internal/wakeword/shadow/open.go` `ModelStem`：以 `.onnx` 结尾取 basename 主干，否则原样透传；主干同时是评分 key，须唯一、仅字母数字 `_.-`、≤64 字符）。上游控制器会先上传（`POST /api/oww_models/upload` 落 `oww_models/` 数据卷）再经 shell 面自动同步资产、**装好才切**；本服务器只覆盖「上传进服务端数据卷」这一段——训练产物经管理台文件页（或 `POST /api/files/upload?path=oww-models/<主干>.onnx`）传到服务端 `oww-models/`，设备设置页下拉即会列出；但**没有** 服务端→设备的自动分发，模型文件仍需自行进入设备本地 `/data/local/share/echomuse/oww/`（如 TWRP/adb push）——设备只认本地已装的分类器。若该设备从未装过 OWW 运行时还需 `libonnxruntime.so`/`melspectrogram.onnx`/`embedding_model.onnx` 三件（上游镜像 `/app/models/oww_runtime`，或用官方 controller 开通一次自动装齐）。
5. **设置 `owwModel`**：**必须先确认设备本地已装同名分类器再下发模型**——服务端上传只解决「有地方放」，不等于「设备已装」；顺序反了且 `owwOnDevice=on` 时设备听不到任何唤醒词（只剩按钮；上游 #191，上游有 hold-back 保护而本服务器没有）。面板下拉列官方四款 + 服务端 `oww-models/` 已上传的自定义模型（已存自定义值原样显示）；设置自定义值走 `POST /api/echomuse/devices/{id}/settings`，body `{"owwModel":"<主干>.onnx"}`（上游风格绝对路径亦可，只要 `.onnx` 结尾且主干与文件名一致）；确认 `owwOnDevice=on`、`owwThreshold` 合适（oww_forge 建议值仅 advisory）。恢复默认显式发 `hey_jarvis_v0.1`。
6. **验收**：`GET .../system/logs?source=messages` 应出现 shadow 加载成功行（`onnxruntime …, model <主干>`；加载失败只 log 一次并退回按钮唤醒，不会主动告警）；最终以喊新唤醒词有响应为准，把设置下发成功当设备已换词是错误结论。

### emOS Web 管理（安全子集）

EchoMuse 设备卡展开管理区后，显式读取运行状态、系统信息或日志，不对每台设备高频轮询。运行状态来自该设备本次 control 会话，不写入配置库；列表与 runtime 共用 `session_id`，快速重连即使未观察到离线，也会使旧快照及在途回包失效。缺失测量为 `null`（UI 显示未知），下线后不冒充实时测量。物理麦克风静音只读，不能与输出音量为零混淆。

| 管理接口（均在 `/api/echomuse/devices/{id}` 下） | 语义 |
|---|---|
| `GET /runtime` | `{device_id,session_id,online,stats,listen_state,volume_state,mute_state,updated_at}`；保存设备上报的最新状态，stats 只保留已知标量字段（每帧最多 16 KiB、字符串最多 256 字节），无上报为 null；`connected_secs` 不是设备 uptime |
| `POST /volume`，body `{"level":0}` | 仅已批准的在线 emOS，整数 0~127，且设备声明 speaker；下发 `volume_set`，202 `{queued:true,applied:false,level}`；实际音量以稍后 `volume_state` 为准，不持久化为开机音量 |
| `GET /system/info` | 固定的 emOS / 内核 / uptime / 内存 / 磁盘诊断命令，200 `{device_id,output,exit_code:0}`；不接受 command/path/url |
| `GET /system/logs?source=messages` | 固定源 `messages`（运行期）、`messages_last`（关机保存）、`last_kmsg_prev`（上次内核）、`supervisor`（EchoMuse 启动监督）；返回同上并附 source，只取有界尾部 |
| `POST /system/reboot`，body `{"confirm":true}` | 通过受限 shell 向 emOS PID 1 发 TERM，使用 init 自己的同步/停止服务/只读重挂流程；202 `{dispatched:true,rebooted:false}` 仅确认重启请求已发出，实际重启必须观察重新连接 |

系统诊断/日志/重启仅限**已登记、已批准、当前在线且上报 `base_os=emos`** 的设备；同时要求非空 `[server].expected_token` 和 `[echomuse].token`。网页用 `?token=<URL 编码后的管理 token>` 打开管理台（对整个 token 做 `encodeURIComponent`，尤其是 `+`、`=`、`&` 等字符；勿附无值的裸查询参数）。设备 shell 回拨除设备 token 外，还须匹配当前已认证 control 的 TCP 来源 IP 和会话探针，且必须存在服务端发起的待处理请求；不接受自发 shell。每台设备一次只允许一项操作，超时、断开、取消均释放占用；单次 15 秒总超时，命令输出与完成标记聚合上限 64 KiB，未收到完成标记或退出非零不返回成功。

系统接口错误以 `{error,code}` 回报：未知设备 404、未批准 403、不支持的 OS 422、离线/占用 409、未配置凭据 503、超时 504、传输或命令执行失败 502。设备 token 不会发给浏览器，管理接口继续使用统一管理鉴权。诊断日志可能含网络标识，复制或分享前应人工检查。

**验证边界**：本次覆盖自动化测试与 Web/macOS 编译，不等同于浏览器交互或真机验证。受保护管理功能的现成凭据入口在 Web；macOS 原生宿主目前未提供管理 token 输入，编译通过不代表可连接非空管理 token 的服务端。WiFi、BLE、shell 拨回、日志路径与 PID 1 重启仍需在目标固件上验收；不要对未核实的日志内容、上线状态或 API 202 声称设备已经执行成功。`tools/echomuse_mock_device.py` 当前只覆盖语音回路。emOS OTA 仍刻意未实现：它需要读取每台设备本机 boot、保留内核/DTB、重建镜像、校验刷写和 trial 回滚，不能套用 ESP OTA，也不能发布通用 boot.img。

### 本地验证

```bash
# 仓库根目录：含受限 shell loopback WS、会话替换与并发事件回归
~/.cargo/bin/cargo test --manifest-path server/Cargo.toml
DOCS_RS=1 ~/.cargo/bin/cargo check --manifest-path server/Cargo.toml --features sherpa
~/.cargo/bin/cargo fmt --manifest-path server/Cargo.toml --check
# client 目录：直接引用生产纯codec，运行地址/handle/Hex/Base64、响应及确认身份回归（无UI/网络/硬件）
./gradlew :bleProtocolTests:jvmTest
# Web 发布与 macOS 公共源码编译（不是原生宿主链接或交互测试）
./gradlew :apps:h5App:publishWeb -PwebSourceMap=false :shared:compileKotlinMacosArm64
```

BLE 回归包括真实 loopback HTTP 路由的鉴权/配置/批准门禁、六种 GATT 请求到 `0x08` 帧及模拟固件应答、`ok:false` 与无 ok 的 slots、大小限制、取消/deadline、快速重连相同 req 不串会话。固件应答来自测试 channel，不是真实蓝牙无线链路，也没有对实际 IoT 设备写入；还需在目标固件上按面板步骤核对扫描、连接、读写与 CCCD 通知。UI 复用 Kuikly 的 observable/observableList 和有界 Scroller，编译通过不等于浏览器交互验收。

## 维护约束

- 帧型按**方向**分命名空间（`0x04/0x05` 下行是音乐帧、上行是哨兵），解析只走 `frames::parse_uplink`；`0x06`/`0x08` 是蓝牙代理帧（`ble_proxy` 开启才翻译，`ble.rs` 消费）。
- `/em/data` 认领：真机固件拨通后**先发一条 Text identify** `{"type":"identify","device_id":…}`（data 面唯一约定的文本帧，上游 `device/internal/client/data.go` 同款），服务端读首帧按其 device_id 精确认领（多台设备并存也对号入座）；mock/测试路径可省 identify，此时显式 `?device_id=` 优先，否则仅当恰好一台未认领时取它。identify 之后 data 面恒为二进制，再收到的 Text 一律忽略不断链。认领取走 `data_rx`，data 任务退出经 `hub_restore_data` 归还（探针校验同会话，防旧认领顶掉新 register 的条目）——链路闪断后设备重拨立即再认领，不必等下次 register；hub 条目存活到会话结束（管理面在线判定/GATT 下发依赖这一点）。
- `GET /em/status` 只读状态查询：`{"enabled":bool,"devices":[{"device_id","connected_secs"}]}`，devices 取 hub 当前 key（= 处于会话中的在线设备）；关闭时也返回 200 + `enabled:false`（排查用，不 404、无鉴权）。
- 帧编解码、能力协商形状、transport 翻译、hub 认领/归还、蓝牙代理状态机与 GATT 请求构造的回归在 `server/src/device/echomuse/tests.rs`、`hub_tests.rs` 与 `ble_tests.rs`（channel 直驱不起真实 WS）；真机联调用 `tools/echomuse_mock_device.py`。注册表 `APP_SECTION_HOT` 已登记 `echomuse: RestartOnly`（护栏测试会抓遗漏）。
