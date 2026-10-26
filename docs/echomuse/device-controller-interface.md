# 设备 ↔ 控制器接口（Device ↔ Controller Interface）

> 整理自上游 [wilbowes/EchoMuse docs/device-controller-interface.md](<https://raw.githubusercontent.com/wilbowes/EchoMuse/main/docs/device-controller-interface.md>)（快照 2026-10，main 分支）；协议权威以英文原文与固件源码为准，本文仅供本仓库开发参考。

本文是设备二进制为实现「被 EchoMuse 控制器驱动」而必须遵守的契约。它存在的意义，是让**新板卡**——Echo Show 8（`crown`，继 Echo Dot（`biscuit`）之后的第二块）——能按书面规格开发，而不必去读 Dot 的源码。控制器本身早已板卡无关；新板卡几乎完全就是同一套线上协议之下的一组全新硬件 **Binding**。

每条消息、每个字段的权威出处是原文内联引用的代码；本文与代码不一致时，以代码为准，本文即视为 bug。

## 两条统领其余一切的规则

两者均由 `controller/tests/test_capabilities.py` 守护。遵守它们的板卡可与任意版本的控制器配对，反之亦然。

- **按能力协商，不按版本。** 设备在 `register` 消息中声明自己实现了什么；控制器读取 `Device.capabilities`，按对应字符串逐特性设门。绝不比较版本字符串——那会把发布历史塞进控制器，还会误判 dev 构建。设备缺少某能力的 UI 控件应显示为**禁用并注明原因**，绝不能是一个点了没反应的控件。
- **降级到旧行为，绝不降级到错误答案。** 未知 JSON 字段与消息类型双向忽略。凡记录测量值的新字段，缺失一律存 **NULL 而非 0**——无法上报某项的设备，不得被读成「上报为零」。

## 三个平面

每台设备向控制器打开 **三条** WebSocket 连接；控制器经 mDNS（`_emcontroller._tcp.local`）发现，三条均由设备主动外拨。

| 路径 | 载荷 | 方向 | 用途 |
|------|------|------|------|
| `/control` | JSON 文本 | 双向 | 注册、LED、mic 启停、按键/状态事件、config 推送、WiFi、shell 控制 |
| `/data` | 二进制 | 双向 | 麦克风 PCM 上行；扬声器 + 音乐 PCM 下行 |
| `/shell/{device_id}` | 原始二进制 | 设备按需外拨 | root shell 代理，仅在控制器发出 `shell_open` 命令后才打开 |

三个平面均有明文（`ws://`）与 TLS（`wss://`）两种形态，见下文「链路认证与 TLS」。`/shell` 平面在控制器开口要之前不会被拨。

**保活与丢包。** 控制器在每个平面每 20s 发一次 WebSocket ping，30s 无 pong 即断连（`WS_PING_TIMEOUT_S`）；必须及时应答 ping，即绝不阻塞读 socket 的 goroutine 或线程。家庭 WiFi 会丢包，设备应为其 socket 启用 **`TCP_THIN_LINEAR_TIMEOUTS`**（Linux：在途段不足四个时按线性定时器重传，而非指数翻倍）；控制器自身同样设置。在射频与蓝牙共用天线的板卡上，**链路正承载一轮对话、一个回复或一个 shell 会话期间，必须停掉所有 BLE 扫描**——在 Dot 上，扫描运行期间 AP 重发了 47–150% 的帧（见 `device/CLAUDE.md` 的 "The LE scan costs the WiFi link"）。

## 注册与能力

`/control` socket 一打开，设备立即发送一条 `register` 消息（`device/internal/client/control.go`）：

```json
{
  "type": "register",
  "device_id": "<stable id>",
  "version": "<firmware version, from build ldflags>",
  "capabilities": ["mic", "speaker", ...],
  "ip": "<local ip, omitted if 127.0.0.1 or unresolved>",
  "ambient_light_status": { "...": "..." },
  "base_os": "emos | fireos | unknown",
  "pairing": true,  // only during a pairing window (capability `pairing`)
  "board": "<pkg/board id, or unknown>",
  "kernel_arch": "<uname -m, e.g. aarch64>",
  "kernel_release": "<uname -r, e.g. 3.18.19+>",
  "boot_id": "<the kernel's /proc/sys/kernel/random/boot_id>",
  "boot_reason": "<androidboot.bootreason, e.g. power_key, wdt_by_pass_pwk>",
  "emmc": { "rev": 7, "preEol": 1, "lifeA": 1, "lifeB": 2,
            "name": "FJ25AB", "date": "08/2017", "manfid": "0x000015" }
}
```

`base_os`、`board` 与两个 `kernel_*` 字段描述本次启动，均为信息性：控制器存储并展示它们，按 `base_os` 对 Android 专属载荷设门，并依 `board` 在 Home Assistant 中命名 Echo 机型（`em_boards`；缺失或 `unknown` 时显示 Dot 2 的名字）。`uname` 失败时省略 kernel 两字段。面向新板卡的设备应全部上报。

`boot_id`、`boot_reason` 与 `emmc` 是启动期健康信息，读不到即省略。控制器按 `boot_id` 存一行（设备每次重拨都会重新 register）记录重启原因，按天存一行记录磨损；磨损还会随 `stats` tick 上报、每六小时重读一次——因为设备可能数月不重启。两者都显示在 Status 标签页。`emmc` 携带 eMMC 自身的 EXT_CSD 字节（JEDEC JESD84-B51）：`rev` 是 EXT_CSD_REV [192]，低于 7 时其余三项无意义；`preEol` 是 PRE_EOL_INFO [267]（1 正常、2 警告、3 紧急）；`lifeA`/`lifeB` 是 DEVICE_LIFE_TIME_EST_TYP_A/B [268]/[269]（1–10、按 10% 步进；11 表示超过额定寿命）。按类型（`/sys/bus/mmc/devices/*/type` = `MMC`）定位 eMMC，而非按编号。`boot_reason` 取自内核 cmdline，cmdline 在其之前被截断时缺失（biscuit 的 FireOS 6 内核即如此）。

`capabilities` 是协商信号。Dot 声明以下能力（`control.go` 的 `capabilities()`）：

| 能力 | 条件 | 含义 |
|------|------|------|
| `mic` | 恒有 | 在 `/data` 上行麦克风 PCM |
| `speaker` | 恒有 | 播放来自 `/data` 的扬声器 PCM |
| `leds` | 恒有 | 接受 `leds` 帧 |
| `led_anim` | 恒有 | 每次状态变化按一条 `led_anim` 规格本地渲染动画（而非控制器逐帧下发） |
| `buttons` | 恒有 | 发出 `button` 事件 |
| `oww_shadow` | 恒有 | 可本地评分唤醒词并**上报**（绝不行动） |
| `oww_trigger` | 恒有 | 可**行动**于自身唤醒检测——有意与 `oww_shadow` 分开（见下） |
| `button_hold` | 恒有 | 发出长按（`heldMs`） |
| `audio_mix` | 恒有 | 音乐走自己的帧类型并混音垫在语音之下，而非暂停 |
| `oww_local_only` | 恒有 | 可**私密监听**：本地评分自己的唤醒词，触发前不发送任何数据。是否正在做由 `listen_state` 表达，见上游 `listening.md` |
| `aec_hw_ref` | 恒有 | 可从麦克风采集内部的播放回环取 AEC 远端参考；板卡没有时回退到 ALSA 写入处的软件抽头 |
| `output_chain` | 恒有 | 可在 ALSA 写入处自行运行扬声器输出链（EQ → 低音守卫 → 限幅器），配置键为 `eqBands`、`eqLoudness`、`limiter*`、`bassGuard*`。仅当控制器 `ack` 同样带 `output_chain` 时才运行——那是控制器声明自己已停止处理的信号；任一侧单独开都保留旧路径，音频绝不会被整形两次 |
| `wake_cue` | 恒有 | 可自行生成唤醒音，音量 `wakeSoundLevel`，独立于主音量。`wakeSound` 开且唤醒**赢得仲裁**后播放：私密监听会话在 `listen_ack` 时，否则在 `play_cue` 时——绝不在跨越瞬间，让渡出去的唤醒不发声 |
| `volume_cue` | 恒有 | 物理按键改变音量后，按新选中音量播放 `volumeButtonSound` 预览；最大音量下再按一次 Volume Up 可重放。仅语音与音乐都空闲时 |
| `wake_mic` | 恒有 | 读取 `wakeMic`（0 = 中央麦克风，1–6 = 外围 MK1–MK6）并在该麦克风上监听唤醒词。面向中央麦克风已损坏的机器（#705）。无此能力时控制器把该设置显示为禁用 |
| `remote_volume_arc` | 恒有 | opt-in 的 `remoteVolumeArc` 设置开启时，可为非零的远程音量变更显示青色音量弧。物理按键依旧无条件；静音、开机恢复与重复的状态同步依旧无声 |
| `response_level` | 恒有 | 可用 `responseLevel`（`low`/`medium`/`high` = 0/+6/+12dB）相对设备音量抬升语音平面。增益在语音/音乐混音之前施加，并封顶使「增益 + 设备音量」不超过 unity |
| `stock_sound` | 恒有 | 在运行输出链且 jack 无插入时让自家扬声器发声：自带音量律、驱动校正与压缩器频段，且没有关闭它们的键。期间保持低音守卫开启，无论 `bassGuardEnabled` 说什么；控制器对此类设备隐藏该开关 |
| `ambient_light` | 仅当传感器实际可读（`als.Present()`） | 上报光照读数 |
| `sendspin` | 恒有 | 可作 Sendspin 播放器（#89），`sendspinEnabled` 开启时运行，供 Music Assistant 同步多房间音频。Music Assistant 直连设备（端口 8928，以 `_sendspin._tcp` 广播）；会话内容不经控制器。运行/连接/配对状态见 `sendspin` 状态 |
| `wake_word_off` | 恒有 | 遵守 `wakeWordEnabled: false`（Home Assistant 的 "No wake word"）：唤醒跨越按 shadow cross 上报，不开会话也不开 turn。按钮仍然可用。无此能力时，控制器对 `listen_state` 为 `local` 的 Echo 拒绝 "No wake word" |
| `pairing` | 恒有 | 机主在无控制器链路或仅明文链路时按住 action 键 7s 即请求自行配对（v2.19.0 前为任意链路 5s；wss 上的 Echo 视为已配对，其长按会直达 HA）：在线路上每 5s 发一条 `pair_request`，否则在 2 分钟窗口内每次拨号都带 `"pairing": true`，wss 连不上时回退明文（不带 token）。新凭据落地即提前关闭窗口，由此引发的重拨不再请求。无此能力时，控制器向管理员提供 **Pair** 操作代替，因为设备无法自己请求 |
| `ble_connect` | 恒有 | 可为控制器持有蓝牙 LE 连接并跑 GATT（#656）：最多三条链路，请求与结果走 `/data` 的 `0x08` 帧。仅在 `bleProxyEnabled` 与 `bleProxyConnections` 同时开启时行动，且只对在 `ack` 里回告 `ble_connect` 的控制器 |

**`aec_hw_ref` 是带运行期伴生信号的能力，两者缺一不可。** 能力本身只说明固件*懂得如何*使用硬件回声参考；板卡是否真有一条，由周期性 `stats` 消息上的 `aecRef` 回答——`"hw"`、`"sw"` 或 `"off"`，过老固件**缺失**该字段，缺失绝不能读成 `"sw"`。

之所以拆开，是因为证据只在运行期可得：某声道要确证为播放回环，须在空闲时逐位精确静音**且**扬声器播放时载有音频，而注册时什么都没播过。它影响 AEC 延迟控制——该控制补偿「写入到耳」延迟，对软件抽头有意义，对帧对齐的硬件参考无意义。若按能力设门，当前所有设备（包括回退到抽头、真正需要它的设备）都会被灰掉；按 `aecRef == "hw"` 设门，恰好在它无所作为之处禁用。没有回环的板卡永远上报 `aec_hw_ref` + `aecRef: "sw"`，这是正确的降级行为，控制器无需改动。

**`oww_shadow` 与 `oww_trigger` 是两个能力，不是一个，这一拆分是承重的。** Shadow 先发布，野外存在会评分上报却无行动代码的固件。把「能评分」当「能触发」的控制器会撤下自己的唤醒检测，苦等设备永不发送的触发——一台评分完美却从不应答的设备。本地评分的板卡 SHOULD 同时声明两者；只转发评分的板卡只声明 `oww_shadow`。

控制器经 `Device` 类（`controller/em_controller.py`）上的 `@property` 门读取能力：`led_anim_capable`、`audio_mix_capable`、`button_hold_capable`、`oww_shadow_capable`、`oww_trigger_capable`、`aec_hw_ref_capable`，外加对基础四项（`mic`/`speaker`/`leds`/`buttons`）的直接成员检查。控制器检查的每个字符串必须是设备能发的，设备发的每个字符串必须是控制器认识的——双向都由 `test_capabilities.py` 钉死，任何一侧的拼写错误都会静默失败。

## `/control` — JSON 消息

设备侧按 `control.go` 中的 type switch 分发。未知 type 忽略（向前兼容）。下表列出被实际处理的载荷字段；缺省的可选字段沿用先前/默认行为。

**设备 → 控制器**

| `type` | 载荷 | 含义 |
|--------|------|------|
| `register` | 见上文 | 握手，连接时发送一次 |
| `button` | `clickType`、`down`、`heldMs`、`muted`、`button.type` | 按键按下/释放；仅 `button_hold` 才有 `heldMs` |
| `mute_state` | `muted` | 静音开关切换（静音归设备主权——见 `device/CLAUDE.md`） |
| `volume_state` | `level` | 音量变更；控制器将其持久化为 `startupVolume` |
| `oww_shadow_cross` | score/threshold/age 字段 | shadow 模式唤醒跨越（仅上报） |
| `oww_wake` | `score`、`threshold`、`ageMs`、`capturedMono`；跨越帧仍在缓冲时另有 `level`、`peak`；私密监听下另有 `session`、`floor`、`barge` | 设备端触发已 fire（`owwOnDevice=on`）。带 `session` 即开启了私密监听会话，音频随后以 `0x07` 上行（上游 `listening.md`）；不带则落入 `Device.pending_wake`，由连续流传送音频。`level`/`peak` 是唤醒词在去除 `micGainDb` 后、截至跨越帧的 2.0s 内响度（dBFS）（定义见 `controller/em_wakelevel.py`）；仅记录，不据此行动 |
| `listen_state` | `state`（`local`/`stream`/`degraded`）、`reason?` | 设备对自身唤醒流正在做什么。每次变化及每条 `ack` 后发送 |
| `listen_end` | `session`、`reason` | 设备自行关闭了会话（`ack_timeout`、`max_open`、`muted`、`link`、`stopped`） |
| `ambient_light` | `lux` | 光照读数，真实变化时发送（仅 `ambient_light`）；约 30s 的 `stats` 以 `ambientLux` 携带 |
| `stats` | 硬件与链路遥测（`internal/client/stats.go`，`DeviceStats`） | 约每 30s 一次，连接时再加一次。每个字段皆可选，缺失表示**未测量**，绝不是零。`tcpUpRetrans` 是自上次上报以来设备各平面的 TCP 重传数，`tcpUpSegs` 是内核有计数的已发送段数（FireOS 5 的 3.18 不计）；`ble` 携带扫描器计数，含扫描为链路让路的时间 `yields`/`yieldedMs` |
| `ble_adverts` | `adverts[]` | 被动 BLE 扫描器的批量结果。**遗留路径**——控制器回告 `ble_adverts_data` 时改走 `/data` 的 `0x06`，仅在其未回告时才用本消息（#404） |
| `wifi_scan_result` | `networks[]`（`{ssid, ssid_hex, signal}`），或 `error` | 应答 `wifi_scan` |
| `wifi_result` | `ok`、`ssid`、`error?` | `wifi_change` 的结果，重发直至 `wifi_commit` |
| `pair_request` | — | 机主在已连接设备上按住了 action 键（仅 `pairing`）。控制器显示 **Approve pairing**；批准后经 shell 平面下发轮换的 token 与 CA，并弹开链路 |
| `sendspin_status` | `status`（`sendspin.Status`），播放器停止时为 null | Sendspin 播放器的状态变化：`state`（`listening`、`connected`、`playing`、Home Assistant 占用音乐平面时的 `busy`、`error`）、`server`、`paired`、`pairedWith`、`group`、`synced`、`syncErrUs`、`bufferedMs`、播放器计数。不含机密。`stats` 以 `sendspin` 携带同一对象 |
| `sendspin_token` | `token`、`clientId`，或 `error` | 应答 `sendspin_token_request`。token（`SP:0…`）携带设备的 Sendspin 配对密钥：控制器只交给那一个等待中的请求，绝不记录或存储 |
| `playback_stats` | `periods`、`underruns`、`stats`、`barge?` | 一段完整播完的扬声器流（一次回复或播报），排空后发送一次。`stats` 是投送裕度（`minDepth`、`primeWaitMs`、`recvSpanMs`、`maxGapMs`、`bytesRecv`，出自 `speaker.StreamStats`）；`periods`/`underruns` 同时平铺发送，兼容早于 `stats` 的控制器。`barge` 是流上有打岔时设备端评分器的 `{peak, bar, frames}` |
| `log` | `level`、`message` | 供仪表盘与支持包的设备日志行。控制器持久化并推送 |
| `pong` | `id`；应答带 `id` 的 `ping` 时还有 `mono` | 保活应答。`id` 回显 ping 的；`mono` 是设备单调时钟毫秒值（任意固定原点），控制器映射到自己的时钟以给 `capturedMono` 定日期。主动发的心跳 pong 两者都不带 |

**控制器 → 设备**

| `type` | 载荷 | 含义 |
|--------|------|------|
| `ack` | `device_id`、`features[]` | 注册被接受。`features` 是**控制器**的能力清单——设备清单的镜像，读法相同：缺席的 feature 就是控制器做不到的。2.23.0 之前的控制器整体缺失该字段。当前取值：`ble_adverts_data`、`listen_session`、`output_chain`、`ble_connect` |
| `refused` | — | 不予接入：链路认证拒绝了该设备的凭据（token 错误，或已停用的 token）。在断连之前发送；带 `pairing` 能力的固件显示 refused 灯环，提示机主按住 action 键配对。设备也把非自家 CA 签发的 TLS 证书当作 refused。更老的固件忽略它 |
| `pending` | `pairing?` | 不予接入：设备未获批；或带 `pairing:true` 时，其配对请求已记录、待管理员处理。窗口期内继续重拨；批准后下一次拨号即接入 |
| `leds` | `leds[]`、`listening?` | 单条 LED 帧；`listening:true` 标记聆听环，方向叠加层据此取键 |
| `led_anim` | `{pattern, colors, periodMs, ttlSec}` | 本地动画规格；仅 `led_anim` 时发送 |
| `mic_start` | `lock_mic?` | 开始 mic 流。`lock_mic:false`/缺省 = 常开不限门的唤醒流；`true` = 有界、VAD 设门的 turn |
| `mic_stop` | — | 停止 mic 流。私密监听下它结束一个有界 turn 但**不**结束会话（只有 `listen_close` 按 id 结束会话），也**不**结束本地监听——本地监听正是听见打岔的东西 |
| `listen_ack` | `session` | 私密监听唤醒被受理；停掉设备 3s 的 ack 时钟 |
| `listen_close` | `session`、`reason` | 结束该会话。不是当前打开的那个则忽略 |
| `beam_lock` / `beam_unlock` | — | 将波束锁定到所选外围麦克风一个 turn / 恢复全向 |
| `volume_set` | `level` | 设置绝对音量 |
| `duck` | `on` | turn 期间把音乐压低（duck）在语音之下（turn 开始/结束时发） |
| `config` | `ConfigMessage` 字段 | 推送配置（见下） |
| `wifi_scan` | — | 扫描网络；以 `wifi_scan_result` 应答 |
| `wifi_change` / `wifi_commit` | `ssid`、`ssid_hex?`、`psk` / — | 切换 WiFi 并自动回滚；commit 定案 |
| `shell_open` / `shell_close` | `pty?` | 让设备拨 `/shell`（`pty:true` = 交互式）/ 关闭它 |
| `play_cue` | `cue` | 播放设备自生成的提示音。今日仅 `"wake"`，在 `wakeSound` 开且私密监听会话之外的唤醒赢得仲裁时发送；未知名称忽略 |
| `music_flush` / `speaker_flush` | — | 清空音乐 / 语音缓冲（打岔用 `speaker_flush`） |
| `sendspin_token_request` | — | 索取 Sendspin 配对 token，供仪表盘在管理员请求时展示；以 `sendspin_token` 应答 |

**SSID 是 0–32 个任意字节**，仅凭名称不总能唯一定位一个网络。`ssid` 供显示（非法 UTF-8 显示为 U+FFFD）；`ssid_hex` 是精确字节，每条扫描结果都携带，且网络来自扫描时 `wifi_change` 要把它发回。不带 `ssid_hex` 的 `wifi_change` 表示取 `ssid` 的 UTF-8。`psk`：开放网络为空、8–63 个可打印 ASCII 字符，或 64 位原始 hex PSK。早于 `ssid_hex` 的固件会忽略它。

## `/data` — 二进制帧

首字节是帧型，其余为载荷。**帧型常量按方向划分命名空间**——`0x04`/`0x05` 因发送方不同而含义不同，这是有意为之（`device/internal/client/data.go:25`）。板卡实现者绝不能去查一张全局表。

**控制器 → 设备（播放）**

| 代码 | 名称 | 含义 |
|------|------|------|
| `0x02` | speaker | 语音回复 PCM 块；立即播放 |
| `0x03` | speaker EOS | 语音流结束 |
| `0x04` | music | 音乐 PCM 块，独立平面——**仅 `audio_mix`** |
| `0x05` | music EOS | 音乐流结束——**仅 `audio_mix`** |
| `0x08` | ble gatt | 蓝牙连接桥的一条 JSON 请求——**仅设备声明了 `ble_connect`** |

**设备 → 控制器（采集）**

| 代码 | 名称 | 含义 |
|------|------|------|
| `0x01` | mic | 麦克风 PCM 块（单声道 `S16_LE`，16 kHz） |
| `0x04` | VAD-end | 有界 turn：检测到语音后结束 |
| `0x05` | no-speech-timeout | 有界 turn：超时前始终未检测到语音 |
| `0x06` | ble adverts | 一批扫描到的 BLE 广播——**仅控制器回告了 `ble_adverts_data`** |
| `0x07` | session audio | `[0x07][session u32 BE][seq u16 BE][PCM]`——私密监听音频，**仅控制器回告了 `listen_session`** |
| `0x08` | ble gatt | 蓝牙连接桥的一条 JSON 结果或事件——**仅控制器回告了 `ble_connect`** |

`0x04`/`0x05` 可安全复用，因为播放帧只流向设备、采集帧只来自设备。两个采集哨兵有意区分：「喊了唤醒词然后沉默」（安静放弃）必须与「说了话而后端无话可说」可区分。

麦克风流形态（`device/CLAUDE.md`，Device 音频管线）：

- **常开唤醒流**（`mic_start` 不带 `lock_mic`）：不限门、无 AGC——每个周期都连续发送，唤醒评分器看到不间断的流。
- **有界 turn 流**（`lock_mic:true`）：VAD 设门 + 预录环形缓冲（preroll ring），语音之后关门时以 `0x04` 哨兵收尾；超时无语音则以 `0x05` 收尾。
- **私密监听**（`owwOnDevice=on`、设备声明 `oww_local_only`、控制器声明 `listen_session`）：常开流仍在，但只在设备本地——喂本地评分器，不上发任何数据。唤醒打开会话，音频以 `0x07` 上行，直到控制器在语音结束或设备侧限额时关闭。完整契约见上游 `listening.md`；能本地评分的新板卡应实现它而非直接推流。

### `0x06` — BLE 广播

载荷为 UTF-8 JSON，`{"adverts": [ ... ]}`，每条目同上文 `ble_adverts` 所述。不带序列头：一批自成一体，丢了不值得重传。

**仅在控制器的 `ack` 回告了 `ble_adverts_data` 时发送。** 未回告的控制器会忽略该帧——未知帧型双向忽略——所有广播无声消失，任何一端都无错误。此时应回退用 `ble_adverts` 控制消息。

两条发送侧规则，控制器都无法替你强制：

- **有界 turn 推流期间不要发**（`mic_start` 带 `lock_mic:true`）。数据面是单条 TCP 流，已写入的帧无法被抢占；turn 音频不被遥测排队压后的唯一办法，就是不写遥测。一个 turn 只持续数秒，Home Assistant 对每台设备容忍 195s 的陈旧，下游无人察觉。
  **不要把此规则扩展到常开唤醒流。** 唤醒词在控制器侧评分的设备上该流永远在线；mic 一推流就扣住广播，会让每批广播永远发不出，代理在两端都无错误地死掉。
- **发不出去的批次直接丢弃，不要回退控制面。** 回退会把批量遥测压到存活探测通道上，且恰逢链路本就出问题时——这正是此帧要避免的事。

### `0x08` — 蓝牙连接（GATT）

唯一双向共用的帧码。载荷是单个 UTF-8 JSON 对象。设备是 GATT **客户端**：为 Home Assistant 向外连接外设，最多持三条链路（`limit` 见下）。

**双向协商。** 控制器 `ack` 未带 `ble_connect` 则设备不发；设备未声明则控制器不发。连接还要求 `bleProxyEnabled` 与 `bleProxyConnections` 都开启——链路存活于扫描会话之内。

控制器 → 设备。每个请求带 `req` 编号，结果原样回带。地址格式 `aa:bb:cc:dd:ee:ff`；`addr_type` 0 公有、1 随机。

| `t` | 字段 | 结果额外携带 |
|-----|------|--------------|
| `connect` | `addr`、`addr_type` | `mtu` |
| `disconnect` | `addr` | —（已经不在了也算 `ok`） |
| `services` | `addr` | `services[]`：`uuid`、`start`、`end`；`chars[]`：`uuid`、`handle`（声明句柄）、`value_handle`、`props`；`descs[]`：`uuid`、`handle` |
| `read` | `addr`、`handle` | `value`（base64；长值会跟随到其末尾） |
| `write` | `addr`、`handle`、`value`（base64）、`response` | — |
| `slots` | — | 由携带该 `req` 的 `slots` 事件应答 |

设备 → 控制器。

| `t` | 字段 | 含义 |
|-----|------|------|
| `result` | `req`、`ok`，失败时另有 `error`、`att?`、`detail?` | 对一条请求的应答 |
| `notify` | `addr`、`handle`、`value`（base64）、`ind` | 一个通知，或一条 indication（`ind:true`，已向对端确认） |
| `disconnected` | `addr`、`reason` | 链路结束，无论谁断的。`reason` 是 HCI 码（`0x08` 监督超时、`0x13` 对端、`0x16` 己方） |
| `slots` | `free`、`limit`、`addrs[]` | 变化时即发送。连接开关关闭时 `free` 为 0 |

`error` 取值：`disabled`（设置未开）、`no_slots`、`timeout`（对端 20s 内未应答 connect）、`already_connected`、`not_connected`、`disconnected`（请求中途链路断）、`att`（对端拒绝；`att` 为其 ATT 错误码，如 5 表示认证不足）、`att_timeout`（30s 无应答，随后按 ATT 规范要求断链）、`too_long`（写入超 MTU−3 字节）、`not_running`（扫描会话未运行）、`busy`、`bad_request`、`failed`（`detail` 说明详情，供日志）。

设备必须遵守的规则：

- **同一对端的请求按到达顺序执行。** Home Assistant 先写命令后读结果。不同对端不得互相等待：一个 connect 可能耗 20s。
- **UUID 一律完整 128 位形式**，小写带连字符，无论对端用过多短的形式。
- **UUID、句柄与值属对端所有，按原样透传。** 设备不替控制器订阅：开启通知就是对 CCCD 描述符做一次普通 `write`，到达的每个通知都转发。
- **没有配对。** 对端的 Security Request 以 Pairing Not Supported 回应；需要配对的属性以 `att` 5 失败。
- **控制连接一断，释放所有链路。** 结果已无处可去；外设的连接槽位释放优于占用。
- **`0x08` 帧不因 turn 而扣住**（与 `0x06` 不同）。结果欠着一条请求。
- 链路以 30ms 连接间隔建立，5s 无请求后改到 500ms——间隔决定扫描能剩下多少（实测 biscuit：30ms 时广播剩 24%，500ms 约 56%）。射频独立的板卡无需如此。

## Config 推送 — `ConfigMessage`

控制器在连接建立及任一每设备配置变更时发送 `config`。字段为 camelCase（`device/internal/config/config.go`）。**部分更新：非零/非 nil 字段被应用；零/nil 字段被忽略**，因此在必须区分 `false`/`0` 与「未设置」之处使用指针类型（`*bool`、`*int`）。变更立即生效，无需重启。

字段全集（repo 根 `CLAUDE.md`，"Device config push"）：

```
vadThreshold, vadSpeechMs, vadSilenceMs,
owwThreshold, owwModel, owwSpeexNs, owwOnDevice,
adcDigitalGain, adcMicpga, micGainDb,
startupVolume,
beamAngle, beamformingEnabled,
aecEnabled, aecDelayMs, aecTailMs, agcEnabled, nsAsr,
bargeInEnabled, bargeInThreshold,
bleProxyEnabled, bleProxyConnections,
sendspinEnabled, sendspinUnpaired, sendspinName,
eqBands, eqLoudness, limiterEnabled, limiterThreshold, limiterRelease,
bassGuardEnabled, bassGuardDb,
ledScene, ledListenColor, ledThinkColor,
meterAttack, meterDecay, meterFloor, meterGamma, meterRef, meterCurve,
wakeArbitrationMs, duckDb, responseLevel,
buttonSingleTapEvent, buttonMultiTapMs,
owwOnDevice, saveUtterances, streamReply,
wakeSound, wakeSoundLevel, volumeButtonSound
```

`wakeWordEnabled` 单独下发、不入存储配置：它是 Home Assistant 的每 Echo 选择器状态，在连接建立与变更时推给声明 `wake_word_off` 的固件。用指针，因为 `false` 才是关键值。

并非每个字段设备都会处理。输出链键（`limiter*`、`bassGuard*`）、`eq*`、`saveUtterances`、`streamReply`、`wakeArbitrationMs` 及 `button*` 计时键是**控制器侧**的——处理发生在音频上线之前，或仅用于配置作用域。`owwOnDevice` 双吃：控制器消费（作用域）+ 设备执行。`sendspinName` 不是存储设置：它是设备标签，注册时并入推送、改名时单发，因为 Music Assistant 按它列出播放器。新板卡只需实现自己实际拥有的硬件相关的键；未知键忽略，即正确的降级。

## 链路认证与 TLS

- 三个平面都携带 `X-EM-Token` 头，**每次拨号**都从设备凭据文件重读（`device/internal/client/tlscreds.go`），推送的凭据下次重连即生效、无需重启。持有 CA 的设备**绝不**在明文拨号时发送 token。
- 磁盘上有 CA 的设备**只**拨 `wss://`，端口取 mDNS TXT 的 `tls_port` 或其 endpoint 文件；无 `tls_port` 则干脆不拨。唯一例外是配对窗口（`pairing`）：先 wss，再不带 token 的明文，两次都以 `"pairing": true` 注册。无 CA 的设备拨明文。服务器身份是固定 DNS SAN `echomuse-controller`，绝不是 IP。
- 证书回拨/长寿**且**设备把验证时钟钳制到固件构建时间，因为 Echo 在 NTP 之前带着假时钟启动，而连不上的设备没法修自己的时钟。新板卡照搬——两半都不要「规范化」。
- 强制逻辑在 `em_linkauth.decide`（`controller/em_linkauth.py`）：token 错一律拒；存有 token 而设备未出示的放行，仅到设备出示过一次为止（凭据推送本身走明文面），此后拒绝；设备无记录的 token 被忽略而非拒绝。`REQUIRE_DEVICE_TLS=1` 使 TLS+token 成为强制。**设备二进制必须在三个平面都发 token**，否则控制器见过它之后，缺 token 的平面会被拒。
- `/data` 与 `/shell` 只从设备存活 `/control` 连接的地址、且同 scheme 放行（`em_linkauth.follows_control`），所以真设备必须从同一地址拨全部三条。

## 设备二进制负责什么

对任何板卡，设备二进制是语音卫星的本地硬件代理。职责：

- **语音助手采集**——mic 推流（唤醒流 + 有界 turn）、设备端采集管线（按硬件支持做 beamform/增益/AEC/AGC/VAD）、可选的设备端唤醒评分/触发。
- **播放**——扬声器输出；声明了 `audio_mix` 时，音乐走独立平面混音垫在语音之下（ducking）。
- **按键**——按下/长按/静音事件，按设备**名称**解析，不按索引（`event2` 在一块板上是按钮、另一块上是触摸屏）。
- **播报**——在 turn 之外播放控制器推送的音频。
- **状态输出**——Dot 上是 LED；无屏或借屏的板卡则是该板自定义的最小状态面（或没有）。

后端能合理驱动的一切——intents、TTS、HA 实体——都留在控制器 / Home Assistant，经 ESPHome Voice Assistant API。设备不重新实现它，也不依赖 Amazon HAL 或库来完成自己的工作（见 repo 根 `CLAUDE.md` 的方向注记）。

## 板卡档案 — `crown`（Echo Show 8 一代）

`crown` 的 binding 藏在与 `server` 镜像的 Go build tag 之后（ADR-0001）。硬件清单——ALSA 卡/设备、格式、输入设备路径、自启——在 [PR #358](https://github.com/wilbowes/EchoMuse/pull/358) 分支的 `docs/echo-show-8-hardware-map.md`（尚未合并）；本节只记录*接口*为 MVP 承诺的部分。

**MVP 能力**（Dot 的子集）：

| 能力 | crown MVP | 备注 |
|------|-----------|------|
| `speaker` | 是 | `card0,device0` → RT5616（issue #5）。推流干净，但 `Ext_Speaker_Amp_Switch` 反相（`On`=无声，`Off`=出声）——与 checkers 同一陷阱，2026-08-26 人耳确认。binding 初始化必须驱为 `Off`；开机默认是 `On` |
| `mic` | **已证明** | `card0,device22`，6ch/16kHz/`S24_3LE`，2026-08-26 真机采集确认——真实信号而非数字零。HW_REFINE 与 checkers 的驱动常数完全一致。待办：安静房间/跨房间 SNR，而非原始采集——见下 |
| `buttons` | 是 | 按名称解析（`gpio-keys` 的音量键、action 键、相机快门） |
| `leds` / `led_anim` | **无** | 无 LED 环；屏幕上的「语音 turn」状态叠加层推迟到 MVP 之后，且即便如此也不碍用户 |
| `audio_mix` | 以后 | MVP 语音环不需要 |
| `ambient_light` | 待定 | 仅当找到可读传感器 |
| `oww_shadow` / `oww_trigger` | 无（MVP） | MVP 用**控制器侧**唤醒词 |

**音频归属**（ADR-0002）：`crown` 在 EchoMuse 运行期间独占 mic 与扬声器，与 Dot 完全一致。mic **持续持有**——MVP 唤醒词在控制器侧，设备必须全程推 mic PCM，否则对下一个唤醒词失聪。**扬声器**按 turn 抓取（回复 + turn 请求的媒体）并释放回空闲——Alexa 模型。屏幕全程完全属于用户自己的软件。

**mic 曾是 MVP 阻塞项，现在不是了。** 麦克风在外置 TLV320AIC3101 芯片上（TDM，`card0,device22`，6ch/16 kHz/`S24_3LE`）——与 Dot 已在采集的格式相同（`device/internal/bindings/mic/pcm_microphone.go`，9ch/16000 Hz/`tinyalsa.PCM_FORMAT_S24_3LE`），只是通道更少、同样的 `Channels * 3` 步进、同样 `DeviceConfig` 驱动的下游路径。2026-08-26 真机验证：`device/tools/capture_mics` 干净打开，4 个 mic 通道全有真实信号、无数字零。`device/tools/hw_refine_probe` 确认驱动自身量程与 checkers 的常数完全一致。完整历史（为什么「数字零」是最初的误读、增益修复调查、实际采集数据）在 PR #358 分支的同一文件里。

**仍未关闭：安静房间/跨房间 SNR**，而非原始采集——今天的测试是音乐播放中的大房间健全性检查，不是唤醒可靠性测量。扬声器（#5）与 mic（#6）均已证明；SNR 是 MVP 里程碑（启动 → 连接 → HA → 唤醒 → Assist → 语音回复）剩余的 go/no-go。

适用于所有 crown binding，不限 mic：输入设备与 i2c 地址**按名称**解析，绝不按编号——`eventN` 编号跨板不稳（在 `biscuit` 是音量键，在 `checkers` 是触摸屏），错号打开会静默失败而非报错。另见 issue #36（Echo Show 5 / `checkers`）——同形的 bring-up 问题，写 crown 专属 binding 前值得一读。

## 不变量（测试钉死了什么）

`controller/tests/test_capabilities.py` 是本契约的护栏：

- 设备声明预期的基线能力。
- 控制器检查的每个能力字符串都是设备能发的，反之亦然（双向拼写守卫）。
- `oww_shadow` 与 `oww_trigger` 保持分离。
- ESPHome 服务器存在前上报的 pending 能力不丢失。
- 能力变更会弹掉 HA 连接，让实体列表刷新。
- 能力缺失时，禁用的 UI 控件不会静默写入。
- 私密监听双向协商：设备 `oww_local_only` + ack 上 `listen_session`，两侧字符串一致。

会话规则本身——设备侧期限、环形缓冲、一帧可抵达哪个会话——由 `device/internal/listen` 与 `controller/tests/test_listen.py` 钉死。

破坏以上任一条的板卡或协议改动，必须连同理由更新测试，不得绕行。
