# Echo Dot 2（biscuit）功能对齐清单

对照基准：上游 `EchoMuse@858987c`（固件 `v2.18.1` / controller `v2.18.1`）。
本文件的"官方"一律指该快照的源码与 `docs/device-controller-interface.md`；"现状"指
`server/src/device/echomuse/`。

**判据**：以固件代码为准，不以文档叙述为准。设备实际接受什么，看
`device/internal/client/control.go` 的 dispatch（`switch peek.Type`）与
`device/internal/client/data.go` 的帧型常量；设备实际执行什么，看
`device/cmd/server.go` 注册的回调。

---

## 0. 摘要

| 级别 | 项数 | 含义 |
|---|---|---|
| **P0** | 3 | 直接决定"按了键、说了话，设备有没有反应"。不修，语音轮次不可用 |
| **P1** | 6 | 用户可感知（灯效、打断、配置可调、音效链），但不影响一轮能否走通 |
| **P2** | 8 | 运维/诊断/周边（配网、遥测、私听、音乐面、配对可见性） |
| 无需移植 | 4 | Dot 2 无对应硬件，或不经本服务端 |

最关键的一条结论：**P0 三项全在"控制面时序"上，而且互相叠加**。
现状只做了 `mic_start{lock_mic:true}` 一件事，官方是每轮
`mic_stop → mic_start_turn → … → mic_stop → mic_start`（常开唤醒流）。
缺了后半段，波束永远拿不到基线、mic 流永远不重置。

---

## 1. Dot 2 硬件与能力面（基线事实）

固件 announce 的能力（`control.go:capabilities()`）里，与 Dot 2 有关的：

`mic` `speaker` `leds` `led_anim` `buttons` `button_hold` `audio_mix`
`oww_shadow` `oww_trigger` `oww_local_only` `aec_hw_ref` `output_chain`
`wake_cue` `volume_cue` `wake_mic` `remote_volume_arc` `response_level`
`sendspin` `wake_word_off` `pairing` `ble_connect`

`ambient_light` 只在光感可读时 announce —— 本机 `tsl2540 found but no als_lux attribute`
（驱动未绑定），故**不 announce，不需要移植**。

控制器侧 feature（`ack.features`）只有四个是设备会读的
（`control.go:FeatureBleAdvertsData/FeatureBleConnect/FeatureListenSession/FeatureOutputChain`）：
`ble_adverts_data` `ble_connect` `listen_session` `output_chain`。

---

## 2. P0 — 直接影响"说话没反应"

### 2.1 缺 `mic_stop`（每轮之后 mic 流无法重置）

官方时序（`controller/em_controller.py:4251-4270`）：

```
按键 → mic_stop → mic_start{lock_mic:true} → 一轮 → mic_stop → mic_start{lock_mic:false}
```

我们只有第一步的前半：`turn.rs:113` 发 `{"type":"mic_start","lock_mic":true}`，
**全仓库没有任何 `mic_stop`**（`grep -rn mic_stop server/src/device/echomuse/` 为空）。

后果分两条路径（`device/internal/client/data.go:977-1180`）：

| 本轮结束方式 | 固件行为 | 我们的后果 |
|---|---|---|
| `0x05` 无语音超时 | `sendFrame` 后 `return`，`streamMic` 退出、`micActive=false` | 下一轮 `mic_start` 能生效 → **可自愈** |
| `0x04` 语音结束 | **不 return**，`active=false` 但 goroutine 继续跑 | `micActive` 仍为 true → 之后每次 `mic_start` 命中 `StartMic: already active — ignoring` → **永久卡死**，语音门不再打开 |

也就是说：一旦成功说过一句话，从那以后每一轮都是空转。固件源码注释写得很明白
（`data.go:988`）：

> StopMic→StartMic in quick succession (**the controller sends that pair after every voice turn**)

现状日志里 `[data] Mic streaming started` 只出现一次，之后再也不出现，与此一致。

### 2.2 缺常开唤醒流 → 波束基线永远是"未就绪"

官方在设备接入后立刻发一次 `mic_start`（无 `lock_mic`），跑 ch6 全向常开流，专门喂
语音门/波束/影子评分。我们 `ack` 之后**什么 mic 指令都不发**，只在按键时发
`lock_mic:true` 的 turn 流。

`streamMic` 开头对 turn 流无条件 `beam.Lock()`（`data.go:1035`），而
`Beamformer.Lock` 要 `baselineReady >= 100`（约 3 秒多通道能量积累）才走
lock-back / onset-ratio 分支（`beamformer.go:211-250`）。流刚起就 Lock，必然落到：

```
default: bestScore = b.energySmooth[...]   →  [beam] locked to ch0 (330°) energy=0.0000 (baseline not ready)
```

而且 `lockedChannel >= 0` 后本轮不再重选 —— 整轮都锁在这个能量测不到的方向。

现场数字对得上：AEC 前 `mic=2732`（0.0834）高于门限 `0.0634`，但门读到的
`rms=0.00836`，中间约 20dB 的落差发生在 AEC 之后的波束输出。

### 2.3 缺 `beam_lock` / `beam_unlock`

官方的波束控制不在 `mic_start` 里做，而是显式消息（`em_controller.py:3994`、`4262`）：

- 唤醒/按键命中时 `beam_lock` —— 锁定说话人所在的外圈麦（"mid-utterance，不动流"）
- 一轮结束回全向 `beam_unlock`，并把 `mic_stop/mic_start` 复位

我们只在 config 里有 `beamformingEnabled` 静态开关，没有运行时锁定/释放。
补了 2.2 之后这一项才有意义；两项一起做才等于官方的采集时序。

---

## 3. P1 — 用户可感知

### 3.1 灯环完全没下发（`leds` / `led_anim`）

Dot 2 有 12 颗 RGB（`is31fl3236`），固件 announce `leds` + `led_anim`，两种都接受：

- `leds`：一整帧 `[{id,r,g,b}...]`（控制器自己算动画时用）
- `led_anim`：`{pattern, colors, periodMs, ttlSec, listening?}`，设备本地自绘（推荐，抖动无关，`ttlSec` 是死信开关）

现状：`ack` 里声明了 `"leds"`，但没有一处下发；`admin_tests.rs` 里那个 `leds` action
只是历史残留断言。**按 action 键无任何灯效**，与第一份设备反馈一致。

最小可用集：聆听（`solid` + 青色，ttl 30s）、思考（`spin`，ttl ~8s）、播报（`meter`，
ttl 按音频时长）、无语音（`pulse` 900ms）、错误（快闪）。

### 3.2 缺 `play_cue`（唤醒音）

设备能自己发声（`wake_cue`），官方在"唤醒赢了仲裁"之后发 `{"type":"play_cue","cue":"wake"}`
（`em_controller.py:1993`）。我们靠 TTS "我在" 覆盖，功能上等价，但少了 `wakeSoundLevel`
独立音量与 0-TTFA 的提示音。

### 3.3 缺 `speaker_flush`（打断）

官方 barge-in 一律先 `speaker_flush` 丢掉设备缓冲（`em_controller.py:1972/2222/3483` …），
我们的打断只靠服务端停送 + `abort` 标志，设备里已缓冲的音频仍会播完。表现是"打断了
还在说"。

### 3.4 `ack.features` 名字用错，且缺两项

```rust
// 现状 mod.rs:408
["mic","speaker","leds","buttons","oww_trigger","oww_local_only"]  // 这是设备能力名
```

设备只读四个 feature 名（见 §1），其余键**进了 map 但永不命中**，等于噪音。
缺 `listen_session`、`output_chain` 是实质缺失（见 3.5、5.1）。

### 3.5 缺 `output_chain`（音效链）

固件能自己在 ALSA 写入处跑 EQ → 低音保护 → 限幅（`cmd/server.go:553`），但要求
**双向声明**：设备 announce `output_chain` 且 `ack` 也带 `output_chain`，表示控制器
已经停手。现状两边都没走这条路，服务端也没有任何 DSP ⇒ **整条音效链目前不存在**。

选边即可：要么在 `ack` 加 `output_chain` 让设备自己跑，要么在服务端做。推荐前者
（零代码路径、零重采样、零延迟）。

### 3.6 `config` 白名单只有 14 项，官方约 50 项

现状（`admin.rs:SETTINGS_KEYS`）：`wakeWordEnabled` `owwOnDevice` `owwModel`
`owwThreshold` `wakeSound` `wakeSoundLevel` `wakeMic` `bleProxyEnabled`
`bleProxyConnections` `beamformingEnabled` `beamAngle` `aecEnabled` `agcEnabled`
`vadThreshold` `micGainDb`。

缺的（按用途）：

| 组 | 缺的键 | 用途 |
|---|---|---|
| VAD 时序 | `vadSpeechMs` `vadSilenceMs` | 门限之外的时间参数；现场调"多久算说完"必需 |
| AEC 细调 | `aecDelayMs` `aecTailMs` `aecRefSource` | 软件参考路径要补延时；硬件参考下才可省 |
| 增益 | `adcDigitalGain` `adcMicpga` | 模拟/数字前端增益，先于 `micGainDb` |
| 降噪 | `nsAsr` `owwSpeexNs` | ASR 与唤醒各自的降噪 |
| 打断 | `bargeInEnabled` `bargeInThreshold` | 设备侧打断 |
| 音量 | `startupVolume` | 开机音量（`volume_state` 的对端） |
| 音效 | `eqBands` `eqLoudness` `bassGuardEnabled` `bassGuardDb` `limiterEnabled` `limiterThreshold` `limiterRelease` | 配合 3.5 |
| 灯效 | `ledScene` `ledListenColor` `ledThinkColor` `meterAttack` `meterDecay` `meterFloor` `meterGamma` `meterRef` `meterCurve` | 配合 3.1 |
| 音量弧 | `remoteVolumeArc` | 遥控调音量时的青色弧 |
| 语音提升 | `responseLevel` | 低/中/高 = 0/+6/+12 dB，语音相对音乐提升 |
| 音乐 | `duckDb` | 配合 5.1 |
| 按键 | `buttonSingleTapEvent` `buttonMultiTapMs` | 单击语义/多击窗口 |
| 多设备 | `wakeArbitrationMs` | 官方称 controller 侧，我们无仲裁，可不下发 |

注：`saveUtterances`、`streamReply` 是 controller 侧语义，不需移植；`owwOnDevice` 已支持。

---

## 4. P2 — 运维与周边

### 4.1 上行消息未消费

固件会发、我们完全不看（`handle_control_text` 只有 `oww_wake`/`button`/`ble_adverts`）：

| type | 现在 | 影响 |
|---|---|---|
| `playback_stats` | 丢弃 | 官方用它判真实播放质量（`minDepth`/`primeWaitMs`/`underruns`/`barge`）；我们只有服务端自算的"断音"告警，看不到设备侧实况 |
| `listen_end` | 丢弃 | 设备自己关会话的原因（`ack_timeout`/`max_open`/`muted`/`link`/`stopped`） |
| `wifi_scan_result` / `wifi_result` | 丢弃 | 配网流程无法闭环 |
| `pair_request` | 丢弃 | 长按 5 秒申请配对，管理台看不到 |
| `oww_shadow_cross` | 丢弃 | 影子评分命中（只报不动作），调阈值/唤醒词模型的关键数据 |
| `ambient_light` | 丢弃 | 本机无光感，可不做 |
| `sendspin_status` / `sendspin_token` | 丢弃 | 见 §6 |
| `stats` 的 `ble` / `emmc` / `sendspin` 子对象 | 丢弃（只取标量白名单） | BLE 扫描健康、eMMC 寿命 |

### 4.2 `register` 字段没解析

现状只读 `device_id`/`version`/`capabilities`/`board`/`base_os`/`pairing`。
未读：`ip`、`kernel_arch`、`kernel_release`、`boot_id`、`boot_reason`、`emmc`、
`ambient_light_status`。这些是排障与"这块板什么版本"的唯一来源。

### 4.3 拒绝时不告诉设备原因（缺 `refused` / `pending`）

现在 token 错就直接 `bail!` 关连接，设备只看到断链、无限重拨。官方分两种：
`refused`（凭据不对 → 设备显示拒绝环，提示按住按键配对）、`pending`（未审批 → 继续重拨）。
我们已有审批模型（`registry.approved`），但没把结论告诉设备。

### 4.4 `ack` 缺 `time_ms`

设备无 RTC，启动即 2010（日志每行 `2010/01/01`）。官方在 `ack` 里带
`time_ms`，设备据此校正（不加字段也安全，旧固件忽略）。我们没带 ⇒ 设备日志时间全错，
排障只能靠服务端日志对齐。

### 4.5 配网链路不完整（缺 `wifi_scan` / `wifi_commit`）

只有 `wifi_change`，没有 `wifi_scan`（列网）、`wifi_commit`（确认），也不处理
`wifi_result`。官方是"切换 → 设备重连 → 回滚窗口内等 `commit`"，我们缺两端，
**切错网就失联**。

### 4.6 `shell` 只有 plain pipe，缺 pty

`shell_open` 支持 `pty:true`（交互式 mksh，带 prompt/行编辑/resize 帧 0x01），
我们发的是不带 `pty` 的编程式会话。只影响"像终端一样用"，不影响 OTA/脚本。

### 4.7 明文 `ws://`，缺 wss + 三平面 token

设备当前拨 `ws://192.168.99.249:8767`。官方三平面（control/data/shell）都走
`X-EM-Token` + wss（自签 CA）。局域网可接受，跨网必须补。

### 4.8 `0x07` 私听音频语义错位

`frames.rs:83` 把 `0x07` 剥掉 6 字节头后当普通 mic PCM 用。这在未协商
`listen_session` 时不会出现（设备走 stream），一旦补了私听会话就是错误实现。

---

## 5. P1/P2 里两块"整块功能"

### 5.1 音乐面（`0x04`/`0x05` 下行 + `duck` + `music_flush`）

Dot 2 announce `audio_mix`：音乐在自己独立平面上与语音在 ALSA 写入处混音，所以 voice
turn 只 `duck`（压低）而不是暂停。现状 `frames.rs` 明确"其余帧型不解析、静默忽略"，
`duck`/`music_flush` 也未实现 ⇒ **无法播放音乐，也无从打断音乐**。

这是"整块功能"，不是补丁：需要音乐源（HA / Music Assistant / 本地文件）、独立
`DataOut` 通道、音量与 `duckDb` 联动、`music_flush` vs `speaker_flush` 的分别处理。

### 5.2 私听会话（`listen_session`）

官方在"设备本地评分"模式下：常开流只在设备内部跑、不发送；本地唤醒命中后开 session，
音频以 `0x07`（带 session/seq 头）上行，控制器用 `listen_ack`/`listen_close` 按 id 收尾。

现状 `ack` 不声明 `listen_session` ⇒ 设备保持 stream 模式（音频持续上行、服务端评分）。
**这是"降级但正确"的选择**，也是我们唯一能走通唤醒词的模式。补齐它才谈得上
"设备端唤醒零上行带宽"。

---

## 6. 明确不需要移植

| 项 | 原因 |
|---|---|
| `ambient_light` | 本机 `tsl2540` 驱动未绑定，固件不 announce |
| `sendspin`（Music Assistant 多房间） | Music Assistant 直连设备 8928 端口，会话不过 controller；只需 `sendspinEnabled`/`sendspinName` 两个 config 键与状态展示 |
| `crown` 板型（Echo Show 8） | 非 Dot 2 |
| ESPHome/HA 实体映射（`em_esphome.py`） | 属"控制器 → HA"接口，本项目有独立 `esp_home` 桥 |

---

## 7. 建议实施顺序

1. **采集时序**（P0 三项一起）：连接后 `mic_start{lock_mic:false}` 起常开流；
   开轮 `mic_stop → beam_lock → mic_start{lock_mic:true}`；收口
   `mic_stop → beam_unlock → mic_start{lock_mic:false}`。
   验收：`[beam] locked ... burst_ratio=`（不再是 `baseline not ready`）、
   `Mic streaming started` 每轮出现、`gate=true`。
2. **灯效**（`led_anim` 五态）：用户第一眼就能判断"听没听见"。
3. **`ack` 修正**：features 改四个真名 + 加 `time_ms` + `output_chain`。
4. **`speaker_flush`**：打断立即生效。
5. **config 白名单扩容**（先 §3.6 的 VAD 时序、增益、AEC 三组）。
6. 上行遥测（`playback_stats` / `listen_end` / `wifi_result`）。
7. 配网闭环、`refused`/`pending`、wss。
8. 音乐面、私听会话（整块功能，单独立项）。

---

## 8. 实施状态（2026-10-24）

P0/P1/P2 与音乐面已全部落地（全量测试 851 passed）。逐项对应：

| 清单项 | 状态 | 落点 |
|---|---|---|
| 2.1/2.2/2.3 采集时序（P0） | ✅ | `mic.rs`（`wake_stream`/`turn_stream`/`turn_ended` 三连）；开轮 `speaker_flush → duck → mic_stop → beam_lock → mic_start{lock_mic:true} → listening → play_cue`（`turn.rs:start_turn`），收口 `turn_finished` |
| 3.1 灯环 `led_anim` | ✅ | `fx.rs` 五态（listening/thinking/speaking/nospeech/off），SPEAKING_TTL=120s 死信开关 |
| 3.2 `play_cue` | ✅ | `turn.rs`（唤醒仲裁后 0-TTFA 提示音，替代 TTS 覆盖） |
| 3.3 `speaker_flush` | ✅ | `turn.rs` 开轮 + `fx.rs:speaker_flush` |
| 3.4 `ack.features` | ✅ | `mod.rs:ack_message` 改 `["output_chain"]`（+ble 两键，条件性） |
| 3.5 `output_chain` | ✅ | ack 声明，设备侧自跑 EQ/低音保护/限幅（config 键全部透传） |
| 3.6 config 白名单 | ✅ | `admin.rs:SETTINGS_KEYS` 扩至官方 ConfigMessage 全量设备键（VAD 时序/AEC/增益/降噪/打断/音效/灯效/音量弧/responseLevel/duckDb/sendspin/按键） |
| 4.1 上行消费 | ✅ | `runtime.rs:LiveControl::update`：`playback_stats`/`listen_end`/`wifi_result`/`wifi_scan_result`/`oww_shadow_cross`/`pair_request` 全登记 |
| 4.2 `register` 字段 | ✅ | `mod.rs:DeviceRegister` 加 ip/kernel_arch/kernel_release/boot_id/boot_reason |
| 4.3 `refused`/`pending` | ✅ | `routes.rs` 准入裁决下发 + registry 登记 |
| 4.4 `ack.time_ms` | ✅ | `mod.rs:ack_message` |
| 4.5 配网闭环 | ✅ | `admin.rs:command_out` 加 `wifi_scan`/`wifi_commit`，上行 `wifi_result`/`wifi_scan_result` 入 runtime |
| 4.6 shell pty | ⏸ 维持原设计 | plain pipe 已覆盖 OTA/脚本场景；交互式 mksh 单独立项 |
| 4.7 wss + 三平面 token | ⏸ 后续整块工程 | 局域网可接受；跨网部署时做（自签 CA + token 分发） |
| 4.8 `0x07` 私听 | ⏸ 未动 | `ack` 不声明 `listen_session`（见 5.2），设备保持 stream 模式，现状正确 |
| 5.1 音乐面 | ✅ | `music.rs`（WAV 解码 + 48k 重采样 + 逐帧下发任务）、`frames.rs` `0x04/0x05`、`media.rs` `music_epoch`（共享 output 预算）、`turn.rs` duck 起止、`admin.rs` `POST /api/echomuse/devices/{id}/music`（play/stop，路径越界校验）、config 键 `[echomuse].music_dir`（默认 `/data/music`） |
| 5.2 私听会话 | ⏸ 未做 | 依赖设备端唤醒资产同步；当前 stream 模式是唯一能走通唤醒词的路径 |
| **Dot2 专属适配层（按键识别 + 静音主权）** | ✅ | `dot2.rs`（`is_action_button` 语义字段 `button.type=="action"` 优先 + `clickType==138` 兜底、`Dot2Button` 映射、四键常量 113/114/115/138）；`routes.rs` button 分支仅 action 键且仅 `down=true` 开轮、静音双保险拒绝；`runtime.rs::LiveControl::is_muted()`；`dot2_tests.rs` + `routes.rs` 端到端共 11 用例 |

真机验收（部署后观察设备日志）：
`[beam] locked ... burst_ratio=`（不再是 `baseline not ready`）、
`Mic streaming started` 每轮出现、`gate=true`、每轮灯效（聆听青环 → 思考 spin →
播报 meter）、按键 0-TTFA 提示音。

---

## 9. Dot2 专属适配层（按键识别 + 静音主权）

把 Dot2（biscuit 板 / emOS）的控制器侧按键处理从通用 echomuse 流水线里**收口成一个独立模块**，
集中承载"只有 action 键该开语音轮"这一专属语义——通用代码不再关心具体 clickType 数字。

### 9.1 按键线格式（固件事实）

设备对 action 键、音量±、禁麦**都发 `type:"button"`**，只靠 `clickType`（数字）或
`button.type`（语义字符串，新固件恒发）区分。实测 Dot2 v2.18.1：

| `clickType` | `button.type` | 键 | 设备本地行为 | 控制器应否开轮 |
|---|---|---|---|---|
| `138` | `action` | 动作键（按下说话 / 唤醒兜底） | 无 | **是**（仅 `down==true`） |
| `115` | `volume_up` | 音量 + | 本地调音量 + 上报 `volume_state` | 否 |
| `114` | `volume_down` | 音量 − | 本地调音量 + 上报 `volume_state` | 否 |
| `113` | `mute` | 禁麦 | 本地 ADC 静音 + 上报 `mute_state` | 否（静音双保险拒绝） |

### 9.2 实现落点

- **`dot2.rs`（新建，专属适配层）**：
  - 四键常量 `ACTION_CLICK_TYPE=138` / `VOLUME_UP=115` / `VOLUME_DOWN=114` / `MUTE=113`；
  - `is_action_button(v)`：优先认 `button.type=="action"`（不依赖数字取值），退认
    `clickType==138`，其余一律否；
  - `enum Dot2Button { Action, VolumeUp, VolumeDown, Mute, Other }` + `from_click_type()`
    （非 action 键的 debug 日志用，便于排障时看出"这条是按了什么键"）。
- **`routes.rs` button 分支重写**：非 action 键只打 `debug` 日志（含 `Dot2Button` 分类与
  `clickType`）；action 键仅 `down==true` 才 `start_turn(live, up_tx, "action 键")`；
  设备已静音（`live.is_muted()`）时双保险拒绝并打 `debug`。
- **`runtime.rs::LiveControl::is_muted()`（新增）**：读 `mute_state.muted` 上报值；设备主权
  静音（ADC mute + 拒绝 `mic_start`）是本保证，控制器侧再兜一层。
- **测试**：`dot2_tests.rs`（5 用例：138/语义/音量±/禁麦/未知/缺字段映射）+ `routes.rs`
  内联 `mod tests`（6 用例：action 按下开轮 / 松开不开 / 音量±不开 / 禁麦不开 / 静音下不开
  / 语义字段开轮），共 11 用例。

### 9.3 已修的 bug（与"官方对齐"同源）

此前 `handle_control_text` 的 `button` 分支**不看 `clickType`/`button.type`**，对音量±、
禁麦、action 一律 `start_turn`——用户实测表现即"按音量 + −、按禁麦，都被当成 action 键开了
语音轮"。修复后语义与上游 `em_button.decide` 对齐：控制器只拒绝语音轮，不拦设备本地音量 /
静音行为（那些由设备 ADC / 调音量自处理并上报状态）。

### 9.4 下一步可继续收口的 Dot2 专属项

当前层只承载"按键识别 + 静音主权"两件最影响可用性的事；其它 Dot2 专属面（biscuit 板特定
codec / ALSA 路径、LED ring `is31fl3236` 灯效语义、12 颗灯布局等）已通过本清单 §3.1 / §4.x
在通用层落地，未再单列——若后续要支持 Dot2 私有键（如长按配对 `pair_request` 的控制器侧
应答），应优先加进 `dot2.rs` 的 `Dot2Button` 与对应处理，而非散落到通用 `routes.rs`。

---

## 附：核对方法

```bash
# 固件接受的全部控制消息（权威）
grep -n 'case "' /tmp/em_up/EchoMuse-858987c/device/internal/client/control.go

# 固件接受的四个 controller feature
grep -n 'Feature[A-Za-z]* =' /tmp/em_up/EchoMuse-858987c/device/internal/client/control.go

# config 全字段
grep -n 'json:"' /tmp/em_up/EchoMuse-858987c/device/internal/config/config.go

# 帧型（按方向分命名空间）
sed -n '20,80p' /tmp/em_up/EchoMuse-858987c/device/internal/client/data.go
```
