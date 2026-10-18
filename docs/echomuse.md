# EchoMuse 设备入口（Echo Dot 2 / emOS）

> 当前功能说明；协议权威来源是 EchoMuse 仓库 `docs/device-controller-interface.md`（以代码为准），实现细节以 `server/src/app/echomuse/` 源码与测试为准。

## 是什么

把一台刷了 emOS 的 Echo Dot 2（EchoMuse 固件）作为语音卫星**直接接入本服务端**：不经 Home Assistant、不跑 EchoMuse 官方 controller。设备经 mDNS 发现本服务器，走 `/em/control` + `/em/data` 双 WebSocket 接入，与 xiaozhi ESP 设备**共用同一条** SenseVoice ASR → LLM/技能级联 → TTS 流水线。

## 配置（`[echomuse]`，改动需重启）

| 字段 | 默认 | 说明 |
|---|---|---|
| `enabled` | `false` | 挂载 `/em/control`、`/em/data` 路由并广播 mDNS `_emcontroller._tcp`；关闭时零行为变化 |
| `token` | 空 | 校验设备 `X-Em-Token` 头；留空不校验。生产建议设置（同网段任何设备都能按此协议连进来） |

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

能力协商：`ack.features` 只声明 `mic/speaker/leds/buttons/oww_trigger/oww_local_only`。**不声明** `listen_session`（不走 `0x07` 私听会话）、`audio_mix`（不做音乐面）、`ble_connect`（不做 BLE 代理）——按协议"未声明即降级"静默关闭；未知帧型/未知 JSON 双向忽略。

会话入口无 xiaozhi hello 交换：`session_params()` 固定构造参数后直接进 `run_session`（`app::echomuse::routes::control_session`）。

## 刻意不做的范围

`/shell` 面、TLS/wss、音乐面（`0x04/0x05` 下行）、BLE、Sendspin、WiFi 配置推送、设备音频链路参数（EQ/AEC/波束）推送均不实现。TLS 是二期项（协议文档给了完整 CA/时钟钳制规范）；无 CA 的设备拨 `ws://` 明文，首版即够用。

## 部署前置（一次性，服务端之外）

1. Dot 用 [firebreak](https://github.com/bboe/firebreak) 解锁（风险自负，可变砖）；
2. 用 EchoMuse 官方 controller 向导刷入 emOS 并配网（设备凭据文件持久化 endpoint/token）；
3. 服务端开 `[echomuse].enabled = true`、设置 `token`，与 Dot 同网段重启——mDNS 广播被设备发现后自动接管；
4. 联调：`python3 tools/echomuse_mock_device.py --host <server> --port 8000 --token <token>`（模拟 register → 唤醒 → 上行 PCM → 收 `0x02…0x03`）。

## 管理面 API（`/api/echomuse/*`，一期）

经 `auth_ok` 鉴权（`Authorization: Bearer <token>` 或 `?token=`）；`[echomuse].enabled=false` 时除鉴权外一律 404（`{"error":"echomuse disabled"}`）。设备登记持久化在 `<memory.db_path 父目录>/echomuse_devices.json`（与配置库同卷，默认 `/data`），register 时 upsert，`online`/`connected_secs` 不落盘、由 hub 实时合并。

- `GET /api/echomuse/devices` → `{"enabled":true,"devices":[{"device_id","label","approved","version","board","base_os","first_seen","last_seen","online","connected_secs"}]}`（registry 全表，按 device_id 排序；`connected_secs` 离线为 `null`）。
- `GET /api/echomuse/devices/{id}` → 单台（同上形状）；不存在 404 `{"error":"device not found"}`。
- `POST /api/echomuse/devices/{id}/approve`，body `{"label":"客厅音箱"}`（label 可省）→ 200 + 该设备 JSON（`approved=true`，label trim 后非空才改名）；不存在 404。
- `POST /api/echomuse/devices/{id}/command`，body `{"action":"reboot"}` 或 `{"action":"wifi","ssid":"...","psk":"..."}`：在线（hub 有 control 通道）返回 202 `{"queued":true}` 并经 `ControlOut::Json` 下发 `{"type":"reboot"}` / `{"type":"wifi_change","ssid","psk"}`；不在线 409 `{"error":"device offline"}`；action 非法 / wifi 缺 ssid 400。
- `POST /api/echomuse/devices/{id}/settings`，body 为**部分更新**的每设备配置，白名单：`wakeWordEnabled`/`owwOnDevice`（bool）、`owwModel`（string）、`owwThreshold`/`wakeSoundLevel`/`wakeMic`（number），白名单外或类型不符 400。先持久化到 registry `settings` 字段；设备在线 202 `{"persisted":true,"delivered":true}` 并即时下发 `config`（`wakeWordEnabled` 按协议**单独一条**），离线 200 `{"persisted":true,"delivered":false}`（设备下次接入时 control_session 自动推送）。`GET devices`/`{id}` 的设备对象新增 `settings` 字段。

遗留限制：`reboot`/`wifi_change` 的设备侧支持需真机验证——命名按 EchoMuse `device-controller-interface.md`，设备不认识的 type 按协议忽略（静默无效果，不会报错）。emOS OTA 管理刻意未实现：官方控制器经 `/shell` root shell 读 boot 分区重打包镜像（`em_emos_update`），复刻需先实现 shell 面与镜像重建流水线，刷写风险高，需要时单独立项。

## 维护约束

- 帧型按**方向**分命名空间（`0x04/0x05` 下行是音乐帧、上行是哨兵），解析只走 `frames::parse_uplink`。
- `/em/data` 认领：显式 `?device_id=` 优先，否则仅当恰好一台待认领时取它；多台设备并存必须带 device_id（协议允许按对端地址收紧，真机多设备联调时再做）。
- `GET /em/status` 只读状态查询：`{"enabled":bool,"devices":[{"device_id","connected_secs"}]}`，devices 取 hub 当前 key（= 处于会话中的在线设备）；关闭时也返回 200 + `enabled:false`（排查用，不 404、无鉴权）。
- 帧编解码、能力协商形状、transport 翻译的回归在 `server/src/app/echomuse/tests.rs`；注册表 `APP_SECTION_HOT` 已登记 `echomuse: RestartOnly`（护栏测试会抓遗漏）。
