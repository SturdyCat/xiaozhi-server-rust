# EchoMuse 设备入口（Echo Dot 2 / emOS）

> 当前功能说明；协议权威来源是 EchoMuse 仓库 `docs/device-controller-interface.md`（以代码为准），实现细节以 `server/src/app/echomuse/` 源码与测试为准。

## 上游参考版本（增量比对锚点）

实现与文档所对照的上游快照，升级 EchoMuse 时先 diff 这些再动代码：

- **版本锚点**：[wilbowes/EchoMuse](https://github.com/wilbowes/EchoMuse) **v2.18.1**（2026-10-15 打标，commit `b18a0befb3492f335ac0a71cdd9cd15390b6e9e8`；官方标注配套 controller 2.26.1）。设备在跑的固件即此版 release 资产 `server`（sha256 前 12 位 `2f3523088af1`，本地缓存 `~/emos-install/server.fw`）。核对时间 2026-10-19。
- **重点参考路径**（上游更新时优先逐个 diff）：
  - `emos/README.md` —— emOS 启动链路（init 12 级 LED → wpa_supplicant → 服务表）、安装与启动确认/回滚机制
  - `controller/device_payloads/start_server.sh` —— 设备侧监督脚本正本（A/B 槽软链、15s 退避、3 次失败切槽回滚）
  - `controller/em_shell.py` —— shell plane 文件下发（base64 heredoc 流 + md5 校验 + rename 原子替换）
  - `controller/em_payloads.py` —— payload 清单与推送时机（on_connect / OTA / Maintenance）
  - `controller/em_routes_provision.py` + `static/dashboard.jsx` —— provisioning 串口下发通道
- **比对方法**：`git log v2.18.1..<新tag> -- <路径>` 逐路径看变化；协议字段增删对照 `docs/device-controller-interface.md` 与本侧 `frames.rs`/`routes.rs` 能力协商子集。只升固件（release 资产）不动服务端代码；协议有变才需要改 `server/src/app/echomuse/`。

## 是什么

把一台刷了 emOS 的 Echo Dot 2（EchoMuse 固件）作为语音卫星**直接接入本服务端**：不经 Home Assistant、不跑 EchoMuse 官方 controller。设备经 mDNS 发现本服务器，走 `/em/control` + `/em/data` 双 WebSocket 接入，与 xiaozhi ESP 设备**共用同一条** SenseVoice ASR → LLM/技能级联 → TTS 流水线。

## 配置（`[echomuse]`，改动需重启）

| 字段 | 默认 | 说明 |
|---|---|---|
| `enabled` | `false` | 挂载 `/em/control`、`/em/data` 路由并广播 mDNS `_emcontroller._tcp`；关闭时零行为变化 |
| `token` | 空 | 校验设备 `X-Em-Token` 头；留空不校验。生产建议设置（同网段任何设备都能按此协议连进来） |
| `ble_proxy` | `false` | 开启蓝牙代理：ack 向声明了 `ble_connect` 的设备回声明 `ble_connect` + `ble_adverts_data`，接入时推送 `bleProxyEnabled`/`bleProxyConnections`（每设备 settings 可覆盖）；关闭时 ack 不声明、`0x06`/`0x08` 按未知帧忽略、管理面 BLE 端点 404——零行为变化 |

mDNS 广播地址**无配置项**：启动时动态探测部署机默认出口的局域网 IPv4，部署在哪台机器、是什么 IP 就广播什么。docker 容器部署会探测到容器内网 IP（设备连不上），请改用 host 网络部署。

会话侧固定参数：上行 16kHz/二进制协议 v1、下行 16kHz/60ms/v1（mic 是 16k PCM 协议常量；TTS 解码后直接出 16k PCM，无重采样）。

## 协议映射（EchoMuse → xiaozhi 会话语义）

| EchoMuse 事件 | 会话语义 | 说明 |
|---|---|---|
| `oww_wake`（设备本地唤醒词命中） | `listen state=start` + 下发 `mic_start{lock_mic:true}` | bounded turn：设备自己做 VAD，结束发哨兵 |
| `button`（动作键） | 同上 | 无唤醒词模型时的兜底入口 |
| `0x01`/`0x07` mic PCM（16k S16LE） | 上行 Opus 二进制帧 | 服务端用同一 `OpusFrameEncoder` 编码，攒满 60ms 一包；`0x07` 会话头（session u32 + seq u16 BE）已剥离 |
| `0x04` / `0x05`（VAD 结束/无语音哨兵） | 残段补零成整帧后 `listen state=stop` | `0x05` 与 `0x04` 同路径（服务器 VAD 兜底切段不受影响） |
| 下行 Opus 帧（`send_binary`） | `0x02` speaker PCM | 解码回 16k，切 40ms（1280B）一块 |
| `tts stop` 文本 | 残段补齐 + `0x03` speaker EOS | 其余服务器 JSON（stt/llm/tts 句起）设备不消费，不下发 |
| 会话 `close` | `Shutdown` → control 任务退出、连接关闭 | 与 WsTransport 的粘滞 `Closed` 同语义 |

能力协商：`ack.features` 只声明 `mic/speaker/leds/buttons/oww_trigger/oww_local_only`。**不声明** `listen_session`（不走 `0x07` 私听会话）、`audio_mix`（不做音乐面）——按协议"未声明即降级"静默关闭；未知帧型/未知 JSON 双向忽略。`ble_connect` + `ble_adverts_data` 仅在 `[echomuse].ble_proxy=true` **且**设备 register 声明 `ble_connect` 时条件追加（协议要求双向声明才启用）。

## 蓝牙代理（`ble_proxy=true`，EchoMuse `ble_connect` / `ble_adverts_data`）

把 Dot 当作服务端的 **BLE 代理**：设备侧被动扫描 + 最多 3 条 GATT 链接（协议上限），服务端采集并直驱外围 IoT 设备。协议权威来源同上（`0x06`/`0x08` 帧、GATT 请求表与错误码清单），服务端实现见 `server/src/app/echomuse/ble.rs`。

- **采集**：设备把被动扫描的广播批次经 `0x06` 帧上报（条目 `{"addr","addrType","rssi","data"(base64)}`，旧固件回退 control 面 `ble_adverts` JSON，同样接收），服务端按 addr 合并并附 `seenAt`，上限 256 条按 seenAt 淘汰；`slots` 事件（连接槽位 `free/limit/addrs`）与 `notify`/`disconnected` 事件存进每设备事件环（容量 100，`state` 端点取尾 50）。
- **控制**：管理面 `POST .../ble/gatt` 按 body 构造 `0x08` 请求下发，`req` 自动分配，等待设备同号 `result`（connect/disconnect/services/read/write/slots 六种 `t`；`value` 走标准 base64）后原样返回——`ok:false` 也是有效应答（设备自守时：connect 20s、ATT 30s 超时先答；服务器 35s 只兜链路静默死亡）。
- **启用链**：配置开启 + 设备声明 `ble_connect` + 设备配置 `bleProxyEnabled`/`bleProxyConnections` 两键都在（接入时基线推送 `true`，单台 POST settings 可覆盖，例如关连接留扫描）。GATT 请求只对 registry 里最新 register 声明过 `ble_connect` 的设备下发（409 `device not ble capable`）。
- **生命周期**：BLE 链路状态挂在设备会话上——control 连接断开设备即按协议丢弃全部 BLE 链接，服务端同刻清状态，等待中的 GATT 请求立即按离线回错。hub 条目存活到会话结束（data 认领取走 `data_rx`、data 任务退出归还），设备完全在线期间管理面随时可下发。
- **不做**：无配对/加密（协议规定 Security Request 答 Pairing Not Supported，加密属性 `att:5` 失败）、不做 HA/ESPHome 转发面（那是官方控制器的用法）、`0x06` 遵守"bounded turn 期间不写遥测"的设备侧规则（服务端不干预）。

### BLE 管理面 API（同 `/api/echomuse/*` 鉴权；`ble_proxy=false` 时统一 404 `{"error":"ble proxy disabled"}`）

- `GET /api/echomuse/devices/{id}/ble/adverts` → `{"device_id","updatedAt","count","adverts":[{"addr","addrType","rssi","data","seenAt"}...]}`（按 addr 排序；离线 409）。
- `GET /api/echomuse/devices/{id}/ble/state` → `{"device_id","slots":{free,limit,addrs}|null,"events":[notify|disconnected ...]}`（最旧在前；离线 409）。
- `POST /api/echomuse/devices/{id}/ble/gatt`，body `{"t":"connect","addr":"aa:bb:cc:dd:ee:ff","addr_type":0}` / `{"t":"read","addr":...,"handle":3}` / `{"t":"write","addr":...,"handle":3,"value":"AQID","response":true}` / `{"t":"slots"}` 等 → 200 + 设备 result 原文；body 非法 400，设备未声明能力/离线 409，等待超时 504。

会话入口无 xiaozhi hello 交换：`session_params()` 固定构造参数后直接进 `run_session`（`app::echomuse::routes::control_session`）。

## 刻意不做的范围

`/shell` 面、TLS/wss、音乐面（`0x04/0x05` 下行）、Sendspin、WiFi 配置推送、设备音频链路参数（EQ/AEC/波束）推送均不实现。TLS 是二期项（协议文档给了完整 CA/时钟钳制规范）；无 CA 的设备拨 `ws://` 明文，首版即够用。蓝牙代理已实现但范围受限（直驱 GATT + 广播采集，无配对/加密、无 HA 转发面），见上文。

## 部署前置（一次性，服务端之外）

1. Dot 用 [firebreak](https://github.com/bboe/firebreak) 解锁（风险自负，可变砖）；
2. 用 EchoMuse 官方 controller 向导刷入 emOS 并配网（设备凭据文件持久化 endpoint/token）；
3. 服务端开 `[echomuse].enabled = true`、设置 `token`，与 Dot 同网段重启——mDNS 广播被设备发现后自动接管；
4. 联调：`python3 tools/echomuse_mock_device.py --host <server> --port 8000 --token <token>`（模拟 register → 唤醒 → 上行 PCM → 收 `0x02…0x03`）。

## 管理面 API（`/api/echomuse/*`，一期）

经 `auth_ok` 鉴权（`Authorization: Bearer <token>` 或 `?token=`）；`[echomuse].enabled=false` 时除鉴权外一律 404（`{"error":"echomuse disabled"}`）。设备登记持久化在 `<memory.db_path 父目录>/echomuse_devices.json`（与配置库同卷，默认 `/data`），register 时 upsert，`online`/`connected_secs` 不落盘、由 hub 实时合并。

- `GET /api/echomuse/devices` → `{"enabled":true,"version":<u64>,"devices":[{"device_id","label","approved","version","board","base_os","capabilities","first_seen","last_seen","online","connected_secs","settings"}]}`（registry 全表，按 device_id 排序；`connected_secs` 离线为 `null`；`capabilities` 为最近一次 register 上报的能力表；`version` 是设备面变化号，作 `/wait` 游标）。
- `GET /api/echomuse/devices/wait?after=<n>` → **事件保持式变更通知**（管理台的"推送"通道；Kuikly commonMain 只有 HTTP，无跨端 WS/SSE 客户端）：`after` 与当前版本不一致立即回 `{"version","changed":true}`；一致则挂起，直到 hub 登记/移除或批准/设置/删除使版本变化（立即醒）或 25s 超时回 `{"version","changed":false}`。客户端（EchoMuse 页）收到 `changed:true` 刷新列表后挂下一轮，无固定轮询间隔。
- `GET /api/echomuse/devices/{id}` → 单台（同上形状）；不存在 404 `{"error":"device not found"}`。
- `POST /api/echomuse/devices/{id}/approve`，body `{"label":"客厅音箱"}`（label 可省）→ 200 + 该设备 JSON（`approved=true`，label trim 后非空才改名）；不存在 404。
- `POST /api/echomuse/devices/{id}/command`，body `{"action":"reboot"}` 或 `{"action":"wifi","ssid":"...","psk":"..."}`：在线（hub 有 control 通道）返回 202 `{"queued":true}` 并经 `ControlOut::Json` 下发 `{"type":"reboot"}` / `{"type":"wifi_change","ssid","psk"}`；不在线 409 `{"error":"device offline"}`；action 非法 / wifi 缺 ssid 400。
- `POST /api/echomuse/devices/{id}/settings`，body 为**部分更新**的每设备配置，白名单：`wakeWordEnabled`/`owwOnDevice`/`wakeSound`/`bleProxyEnabled`/`bleProxyConnections`（bool）、`owwModel`（string）、`owwThreshold`/`wakeSoundLevel`/`wakeMic`（number），白名单外或类型不符 400。先持久化到 registry `settings` 字段；设备在线 202 `{"persisted":true,"delivered":true}` 并即时下发 `config`（`wakeWordEnabled` 按协议**单独一条**），离线 200 `{"persisted":true,"delivered":false}`（设备下次接入时 control_session 自动推送）。
- `DELETE /api/echomuse/devices/{id}`（客户端用等价别名 `POST .../delete`，NetworkModule 只有 GET/POST）→ 200 `{"removed":true,"device_id"}`；不存在 404。只删静态登记（批准/唤醒词设置一并清除），不断开在线会话，设备再次 register 会重新登记——用于清理 mock/联调设备。

遗留限制：`reboot`/`wifi_change` 与蓝牙代理的设备侧行为均需真机验证——命名按 EchoMuse `device-controller-interface.md`，设备不认识的 type/字段按协议忽略（静默无效果，不会报错）。BLE 采集/控制的联调可在 mock 设备基础上补 `0x06`/`0x08` 模拟（`tools/echomuse_mock_device.py` 当前只覆盖语音回路）。emOS OTA 管理刻意未实现：官方控制器经 `/shell` root shell 读 boot 分区重打包镜像（`em_emos_update`），复刻需先实现 shell 面与镜像重建流水线，刷写风险高，需要时单独立项。

## 维护约束

- 帧型按**方向**分命名空间（`0x04/0x05` 下行是音乐帧、上行是哨兵），解析只走 `frames::parse_uplink`；`0x06`/`0x08` 是蓝牙代理帧（`ble_proxy` 开启才翻译，`ble.rs` 消费）。
- `/em/data` 认领：显式 `?device_id=` 优先，否则仅当恰好一台未认领时取它；多台设备并存必须带 device_id（协议允许按对端地址收紧，真机多设备联调时再做）。认领取走 `data_rx`，data 任务退出经 `hub_restore_data` 归还（探针校验同会话，防旧认领顶掉新 register 的条目）——链路闪断后设备重拨立即再认领，不必等下次 register；hub 条目存活到会话结束（管理面在线判定/GATT 下发依赖这一点）。
- `GET /em/status` 只读状态查询：`{"enabled":bool,"devices":[{"device_id","connected_secs"}]}`，devices 取 hub 当前 key（= 处于会话中的在线设备）；关闭时也返回 200 + `enabled:false`（排查用，不 404、无鉴权）。
- 帧编解码、能力协商形状、transport 翻译、hub 认领/归还、蓝牙代理状态机与 GATT 请求构造的回归在 `server/src/app/echomuse/tests.rs`、`hub_tests.rs` 与 `ble_tests.rs`（channel 直驱不起真实 WS）；真机联调用 `tools/echomuse_mock_device.py`。注册表 `APP_SECTION_HOT` 已登记 `echomuse: RestartOnly`（护栏测试会抓遗漏）。
