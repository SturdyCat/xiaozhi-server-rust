# 音频状态模型（Audio State Model）

> 整理自上游 [wilbowes/EchoMuse docs/audio-states.md](https://raw.githubusercontent.com/wilbowes/EchoMuse/main/docs/audio-states.md)（快照 2026-10，main 分支）；以英文原文与固件源码为准，本文仅供本仓库开发参考。

谁拥有扬声器、线上是什么、以及两件事同时想要它时会发生什么。

本文存在的理由与 `led-ring-states.md` 相同。现在有四类东西能把音频送上设备——语音回复、音乐、HA announcement、以及（自 #167 起）timer alarm——各自独立加入，单独看都正确；它们之间的交互正是 bug 的藏身之处。

其中两个已经修掉，修法同构：**owner 是计数，不是标志。** #261 在回复播到一半把 duck 提了起来，因为 `ducked` 是单个布尔值，两个重叠 owner 中先结束的那个把它弹掉了；#314 出于同样原因释放了 turn 的扬声器所有权。`duck_depth` 与 `owner_depth` 特意是两个独立计数器——duck 只在混音路径上递增，因此以暂停代替 duck 的设备会有重叠 owner 但 duck 深度为零；把两者合并，除了恰好这类设备外处处"正确"。

仍然未决：#262（音乐被推迟到 turn 结束）、以及欠着 alarm 这个第四 owner 的状态图。Sendspin（第 6 节）为音乐 plane 加入了第二个生产者，但不加 owner。

状态标记与 LED 文档一致：**[today]** 为已上线且经代码验证的行为；**[proposed]** 为已设计或在评审、未合入；**[built]** 为已编写、已测试、已合入，但尚未上真机验证。

---

## 1. 设计原则

以下已定案，其余皆由其推出。

- **设备混音；控制器决策。** 两路独立 PCM 流到达设备，在 ALSA 写入处求和（`speaker/pcm_speaker.go:277`）。控制器绝不混音。
- **语音永不被衰减。** ducking 压低的是语音底下的"垫底"音频，绝不是回复本身。
- **离开控制器的音频收不回来。** 唤醒词触发时，已有 `LEAD_S` = 4.0s 的音乐在设备缓冲里，而设备深度为 `audioChanDepth` = 128 × 42.7ms ≈ **5.46s**（`pcm_speaker.go:37`）。控制器想改变已在途音频的任何部分，只能靠控制消息，不能靠换着发。
- **语音 turn 对音乐是 duck，不是 pause。** 暂停后恢复需要 seek，而 Music Assistant 的 flow 流不可 seek，一次 28s 的 turn 会吃掉歌的 28s。
- **音频的结束以设备的说法为准。** 播放完成要等设备的 `playback_stats`，绝不等 socket 写入的估计——无论链路多慢，写入都近乎瞬时完成（2026-07-24 实测：一台设备的 ring 提前 6.1s 清空，另一台 3.2s）。

## 2. 扬声器 owner —— 优先级阶梯

从高到低。任一时刻只有一个 owner 驱动 **voice plane**；音乐在自己的 plane 上垫底运行，被 duck 而非被挤走。

| # | Owner | Plane | 取得所有权 | 释放条件 |
|---|-------|-------|------------|----------|
| 1 | Voice turn（唤醒词或按键） | voice | `em_player.interrupt()` —— **无条件**，即使什么都没在播 | turn 结束时 `resume_interrupted()` |
| 2 | HA announcement | voice | 同一 `interrupt()` 路径 | announcement 播放完成 |
| 3 | Timer alarm ring | voice | `start_timer_alarm()`，burst 受 `speaker_busy` 门控 | 解除（按键 / 语音 / CANCELLED）或 `MAX_RING_S` = 15 min（Voice PE 的）；**[today]** |
| 4 | Media / music | music | `em_player.play()` | `stop()` / `pause()` / 设备消失 |

**所有权无条件取得，这是有意为之**——不是可以删掉的优化。"Play some jazz" 在 HA 生成语音回复**之前**就跑 intent，所以 `play_media` 可能在 TTS 还在路上时到达。如果所有权取决于"有什么正在播"，音乐会落在与回复相同的 plane 上并盖过它。这也是 #262 的直接成因；修法是让音乐在自己的 plane 上起步，而不是削弱这条规则。

## 3. 两个 plane

| Byte | 方向 | 含义 | 常量 |
|------|------|------|------|
| `0x01` | device → controller | 麦克风 PCM | — |
| `0x02` | controller → device | 语音 PCM（48kHz mono S16_LE） | `frameTypeSpeaker` |
| `0x03` | controller → device | 语音 end-of-stream | `frameTypeEOS` |
| `0x04` | controller → device | **音乐 PCM** | `frameTypeMusic` |
| `0x05` | controller → device | **音乐 end-of-stream** | `frameTypeMusicEOS` |
| `0x04` | device → controller | **VAD end-of-speech** | `frameTypeVADEnd` |
| `0x05` | device → controller | **无语音超时** | `frameTypeNoSpeechTimeout` |

**`0x04` 与 `0x05` 在两个方向上含义不同**，仅靠帧的行进方向消歧（`device/internal/client/data.go:27-45`）。除此之外没有任何机制强制这一点，只因为读端各在一头。新增帧类型前必须知道这一点。

设备为每个 plane 各持一个 `audioStream`，深度均为 `audioChanDepth`，在写入处混音、且当前 duck gain 只作用于音乐（`pcm_speaker.go:123-125`、`:277`）。求和**饱和而非回绕**——回绕会把响亮峰值变成满幅反极性峰值，比削波糟糕得多。

## 4. 能力降级 —— `audio_mix`

`em_player._frame_types()` 依据设备宣告的 capability 选 plane（`em_player.py:70`）：

| 固件 | 音乐 plane | 语音 turn 行为 | 后果 | 状态 |
|------|------------|----------------|------|------|
| 宣告 `audio_mix` | `0x04`/`0x05` | **duck**（`duck on`） | 音乐在回复下方小声继续 | **[today]** |
| 未宣告 | `0x02`/`0x03` | **pause**，置 `resume_after` | 旧行为，恢复需 seek | **[today]** |

降级到旧路径、而非降级到错误答案，是 `CLAUDE.md` 的规则：不能混音的设备根本不会播 `0x04`——那是静音，不是降级行为。

## 5. 状态转换

### 5.1 语音 turn 压住音乐

| # | 前提 | 动作 | 音乐 | 语音 | 状态 |
|---|------|------|------|------|------|
| V1 | 音乐播放中，有 `audio_mix` | `interrupt()` 置 `ducked`，发 `duck on`，feed 提前量从 `LEAD_S` 4.0s 降到 `TURN_LEAD_S` 1.0s，让出共享数据 plane | 继续，被 `duckDb`（默认 −18dB）衰减 | 回复走 `0x02` | [today] |
| V2 | 音乐播放中，无 `audio_mix` | `interrupt()` → `pause()`，置 `resume_after = True` | 停止，记书签 | 回复走 `0x02` | [today] |
| V3 | turn 结束 | `resume_interrupted()` 释放所有权，`duck off` | 回到 unity | — | [today] |
| V4 | turn 期间用户指令 | 记为 `pending`；**覆盖**我们的 auto-resume | last write wins | — | [today] |
| V5 | duck 在回复结束前被释放 | — | **提前抬起，与回复尾部打架** | — | **bug，#261** |

V1 的混音之前，固件只把 `responseLevel` 用于 voice plane：Low / Medium / High 请求在设备音量之上 +0 / +6 / +12dB。有效提升相对实时设备音量爬坡封顶，合成增益不超过 unity，音量最大时收敛为零提升。

**V5 就是 #261，且原因未明。** `em_player` 只记失败路径的日志（`duck failed` / `unduck failed`），一次"发出去了、生效了、然后被提前释放"的 duck 在日志里完全静默。先加日志再做理论："duck 根本没发出去"与"duck 发出去了、被某处释放了"需要完全相反的调查方向。

### 5.2 Stop 与 flush

| # | 情形 | 消息 | 理由 | 状态 |
|---|------|------|------|------|
| F1 | 用户停止/暂停音乐，有 `audio_mix` | `music_flush` | 只丢弃缓冲的**音乐** | [today] |
| F2 | 用户停止/暂停音乐，无 `audio_mix` | `speaker_flush` | 音乐在 voice plane 上 | [today] |
| F3 | 回复播放中 barge-in | `speaker_flush` | 掐断缓冲的回复；其余通常还在 TCP 里，设备会丢弃直到看到该流的 `0x03` | [today] |
| F4 | 语音 turn 压上音乐 | **都不发** | flush 会丢掉让 duck 即时生效的缓冲音频，且在不可 seek 的流上永久丢失 | [today] |
| F5 | 闹钟解除 | `speaker_flush` | 否则 ring 会把约 5.5s 的设备缓冲播完 | **[today]** |

F1/F2 的判定在 `em_player.py:481`。**语音 turn 绝不发 `music_flush`**——设备自己的 handler 就这么说（`control.go:520`），这正是第二个 plane 存在的全部理由。

### 5.3 Timer alarm **[today —— #167]**

| # | 前提 | 动作 | 状态 |
|---|------|------|------|
| T1 | HA 发来 `TIMER_FINISHED` | ring 开始：循环 burst +（若 `led_anim_capable`）琥珀色 LED 脉冲 | [today] |
| T2 | turn 或 announcement 在播 | `device.speaker_busy` 非零期间 burst 暂缓 | [today] |
| T3 | 任一 Echo ring 期间听到唤醒词 | 控制器聆听语音期间，所有 ring 静音（flush）至多 `DISMISS_LISTEN_S` + 2s；不启动 turn | [today] |
| T4 | 解除（按键、唤醒词后的语音、或 `CANCELLED`） | ring 停止，`speaker_flush`；无语音则释放 hold、ring 恢复 | [today] |
| T5 | 无人应答 | 在 `MAX_RING_S` = 15 min 停止 | [today] |

`speaker_busy` 是计数器不是标志，因为 announcement 可以与 turn 的播放重叠；它在 `try/finally` 中持有，因为一次被取消的 turn 若泄漏了它，会在进程余生里挡住之后每一次 ring。

## 6. Sendspin —— 音乐 plane 的第二个生产者 **[built —— Early Access 固件，自 2026-10-01 上真机]**

经 Open Home Foundation 的 Sendspin 协议实现多房间同步播放，Music Assistant 原生说该协议。部署决策 2026-08-22（Wil）：**播放器跑在设备上，Music Assistant 直连设备**，不经过控制器。设计于 PR #271，2026-09-30 以 `device/internal/sendspin` 建成，按 Echo 经 `sendspinEnabled` 开关，先以 Early Access 固件发布。

### 6.1 它是第二个生产者，不是第三个 plane

Sendspin 承载的正是音乐 plane 已承载的东西——Music Assistant 音频——只是换了条路：

```
0x04       MA → HA → controller (em_player) → 0x04 → device music plane
sendspin   MA ─────────────────────────────────────→ device music plane
```

所以没有新帧类型、没有新混音输入、所有权阶梯不加新行。duck、饱和混音、输出链与软件音量对它原样生效，因为它从与 `0x04` 相同的 `Mixer.Mix` 输入进入。

唯一的差别是时序，而正是它决定了形态。`0x04` 是 push-and-play-in-order：设备播完一个 period 再播下一个，plane 上不带任何时间戳。Sendspin 若不能把每个采样放到服务器选定的时刻就毫无用处。所以播放器是一个**拉取源**（`PcmSpeaker.SetMusicSource`）：写循环每个 period 读一次子流的 ALSA `delay`，算出正在构建的 period 何时到达 DAC，向播放器索取届时该响的音频。只有这个循环知道答案；往现有缓冲里 push 会丢掉它。

替代方案——控制器跑 client 再经 `0x04` 转推——被否决，理由相同：时间戳活不过那一跳。

### 6.2 设备实现了什么

| 部件 | 实现 | 验证手段 |
|------|------|----------|
| 连接 | 服务器发起：设备在 8928 端口 `/sendspin` 监听并宣告 `_sendspin._tcp`（zeroconf）。它从不出网拨号，正如 spec 对 advertising client 的要求。在 emOS 上（其 init 会丢弃入站连接），固件在宣告前于防火墙打开 8928、播放器停止时关闭（`internal/firewall`） | aiosendspin 9.1.1 `connect_to_client`；15LE |
| 加密 | Noise `KKpsk2`、`25519_ChaChaPoly_SHA256`，设备为 responder（`flynn/noise`）。PSK 在两条握手消息之间选定，由 message 1 点名 | 9.1.1，首次握手与 in-band 重握手均测 |
| 身份 | X25519 keypair 存于 `/data/local/etc/echomuse/sendspin.json`；其公钥即 `client_id`。文件丢失 = 设备与所有服务器解绑 | spec vectors |
| 配对 | 仅 Pairing PSK 方法（client 必须提供的那一种）：管理台显示 `SP:0…` 令牌，用户粘进 Music Assistant。未配对访问是设置项，默认关 | 9.1.1 `initiate_pairing`；spec token vector |
| 时钟 | spec 的 2-D Kalman 时间滤波器，逐行移植 | aiosendspin 自带滤波器，300 步内到微秒级一致 |
| 编解码 | 仅 FLAC，48kHz 16-bit **单声道**——只有一个格式，服务器负责重采样与下混，设备永不在流中途切换输出 | aiosendspin 自带 FLAC 编码器，bit-exact |
| DAC 位置 | `OutputClock`：对每 period 的 ALSA delay 读数做 alpha-beta 跟踪——先最小二乘、再收窄到固定增益 | host test，±2ms 噪声 |
| 校正 | spec 建议策略：整帧丢弃/重复，每 2048 帧 period ≤8 次（0.39%，低于 0.5% 上限）；超过 5ms 一次性 resync | host test，DAC ±600ppm |

**互通测试，2026-09-30**（`device/tools/sendspin_interop`，对 Music Assistant 自带的库）：令牌配对及换钥到长期 PSK；最差同步误差 **225µs**、DAC 偏 560ppm 时（spec 底线 1ms、目标 0.5ms），以服务器自身时钟测量而非自报；音量走 spec 曲线；一次 seek；HA 抢占 plane；以存储 PSK 重连；未配对访问关与开。

**真机，2026-09-30**（15LE、emOS 32-bit、Music Assistant 2.10.4）：令牌配对；播放、语音回复下 duck、seek、两端调音量、pause/resume、以及不填地址的发现。第一次尝试撞上 emOS 的入站过滤：播放器宣告并打出 "listening" 日志，但所有连接超时，Music Assistant 列表里从未出现。

**真机，2026-10-01**（15LE emOS 32-bit 与 C95 emOS 64-bit，在 Music Assistant 里编组，两台均开 on-device wake word）：一耳一台 Echo，听感同步；自报同步误差 0.1–0.4ms，无 underrun、无迟到丢弃，缓冲约 9s。校正稳定在每分钟 2–7 次；15LE 重启后头两分钟约每分钟 300 次，随后收敛。停掉控制器，两台播放器同一秒内停、端口关闭，约 20s 后随控制器回来而恢复，Music Assistant 自己重启了播放。固件每分钟打一行 `[sendspin] playing:` 带上述计数。

立体声配对在单声道播放器上可用：在 Music Assistant 里把一台 Echo 设为左声道、另一台设为右声道，它在编码前就选好每台播放器的声道，因此每台 Echo 以单声道收到自己那一侧（用左右声道测试音轨听感确认）。

CPU：FLAC 解码 + ChaCha20-Poly1305 占单核 3.68%（PR #271 在一台 Dot 上的基准）。真机上播放与暂停在噪声内测得相同（/proc ticks ×10s），播放器成本低于该手段的分辨率。

### 6.3 所有权与仲裁

| # | 前提 | 动作 | 状态 |
|---|------|------|------|
| S1 | Sendspin 会话开始 | 播放器排队音频；`0x04` 为空时写循环拉取 | [built] |
| S2 | Sendspin 播放中来了语音 turn | 不变——唤醒越线处本地 duck，然后是控制器的 `duck on`（对 `audio_mix` 设备每个 turn 都发，无论它是否认为在放音乐） | [built] |
| S3 | Sendspin 会话播放中控制器发 `0x04` | **HA 赢**（Wil，2026-08-22）。`0x04` plane 优先被 consult，HA 音频立即上扬声器；100ms 内播放器上报 `available: false`、清空缓冲，服务器把它移出 group | [built] |
| S4 | Sendspin 流结束或清除 | 缓冲清空；`stream/clear` 原地 resync | [built] |

**HA 路由的音乐结束后不重新入组**（Wil，2026-08-22），协议也同意：server 不得 rejoin 一个已转为 unavailable 的 client。由人来重启 group。做错的代价是点一下。

**只有一个音量**（Wil，2026-09-30）。server 的音量命令设置 Echo 自身音量——即 HA 与按键所动的那一个——按与 HA 百分比相同的比例把 0-100 映射到 0..127；任何路由的每次变更都回报，因此 Music Assistant 的滑条会跟随。同步的音乐本身以 unity 播放，电平只施加一次，由扬声器软件音量完成。server 的 mute 只 mute 同步音乐：Echo 的 mute 按键是麦克风。

**它跟随控制器链路**（Wil，2026-09-30）：EchoMuse 是一个系统，ring 显示断连的 Echo 不该在放歌。ring 显示链路 down——disconnected、pending 或 refused——播放器即停（goodbye `restart`），在下一次 config push 时再启动。互通测试确认其后 Music Assistant 会自行重拨。

未做：告诉 Home Assistant 的 media player entity 这台 Echo 正在播同步音乐（S1 的 "controller told"）。届时它显示 idle。

### 6.4 aiosendspin 9.1.1 与 spec 不一致之处

设备必须与 Music Assistant 所跑的库兼容，有几处与 spec（main，2026-09-17）不同。凡能区分两式的，设备发送 9.1.1 的形式、两种都接受：

- **Fragments**：9.1.1 用 type 字节 2（"more"）与 3（"end"）；spec 用带 flags 字节的 type 1。两者都接受；设备从不发送。
- **Audio chunk header**：9.1.1 为 9 字节，spec 为 13（多 `send_ahead`）。靠 FLAC frame sync 的落点区分——这也是设备只宣告 FLAC 的原因之一。
- **`supported_pair_methods`**：9.1.1 是 list，spec 是 object。9.1.1 拒绝 object，所以发 list。
- **Pairing PSK 流程**：9.1.1 只有 `client/pair-finalize`；spec 先来 `client/pair-init`，9.1.1 会当作乱序拒绝。
- **重握手之后**，9.1.1 会重发 `server/hello` 并等新的 `client/hello`；spec 说两者都不重发。设备来了就应答一条，且从交换开始直到收尾的 `server/activate` 之外什么都不说（不发 `client/time`）——中途多出的消息会让配对失败。
- **`available: false`** 在 9.1.1 里意为"外部源占用扬声器"，会 ungroup 该 client。spec 还要求时钟收敛前绝不报 `true`。两者都靠把首条 `client/state` 推迟到收敛（约 0.4s，在 9.1.1 的 5s 允许内）来满足。
- **Delay 字段**：9.1.1 为 `static_delay_ms` / `set_static_delay`，spec 为 `output_delay_ms` / `set_output_delay`。两种命令都接受。

Music Assistant 升级到更新的 aiosendspin 时，先对它重跑互通 harness：list 形状的 `supported_pair_methods` 与配对顺序这两处无法两边都发。

### 6.5 尚未弄清的事

- **爆音不是校正造成的（2026-10-02 定案，#707）。** 是 tinyalsa 的 `pcm_write` 在被信号打断时丢弃写入的剩余部分——每台 Echo 约 15 次/小时，覆盖所有播放路径——已用写入期间屏蔽信号修复（GoTinyAlsa fork，#711）。同一被丢音频正是 tracker "过早读数"与校正爆发的来源；修复并给 tracker 加门控（#710）后，校正稳定在每分钟 2–3 次，过夜最差的一分钟为 9 次。校正仍是硬拼接；只有真的听得到才值得做 crossfade。
- **每小时 3–10 次 tracker reset 仍在**：一次超过 20ms 的提前跳变、且每次写入完整，FireOS 6 内核上频率约 3 倍。该频率下无害；原因未明。
- **与其他品牌播放器的同步。** 所有 Echo 共享同一固定 DAC latency，因此彼此一致；别的播放器需要靠耳朵设 `static_delay_ms`。

## 7. 未决问题

- **Q1 —— 应否允许音乐在 turn 期间在自己的 plane 上起步？** 如今 `play/resume/pause/stop` 只记录意图、在 turn 持有扬声器时不碰线上数据，所以从手机起的流会静默到回答结束（#262）。音乐有了自己的 plane 之后，这条一刀切规则的理由比写下时弱了。未决。
- ~~**Q2 —— 输出链属于谁？**~~ **已答：属于设备**，在 ALSA 写入处，只要两端都宣告 `output_chain`（#243）。它作用于混音结果，所以同步音乐也经过它；在无 `output_chain` 的控制器之后，Sendspin 音乐完全不被整形，因为它从不经过控制器。
- **Q4 —— 2026-09-07 已定：alarm 从不等待，announcement 等（#373）。** 今天两者都写 `0x02`，只有 `_ring_timer_alarm` 会先询问——恰好反了：timer 必须在结束的那一刻响起，所以时序即全部意义的写方反而在让路。2026-08-28 实测：落在 chime burst 之间的 announcement 会播出，落在 burst 之中的听不见。

  规则是 **音乐为 alarm 让路（静音），alarm 为 response 让路（duck）**。三方场景——音乐在放、turn 进行、timer 触发——无需新固件即可解决：`Mixer.Mix(voice, music, target)` 恰好两个输入且只衰减音乐侧，所以 alarm 骑音乐 plane、响铃期间音乐挂起、设备现有 duck 收尾。以 `audio_mix` 门控，因为没有它的固件从不播 `0x04`、静音的 timer 是最坏失败；这类设备保持 `0x02`，alarm 直接抢 plane。

  announcement 等回复播完，并排在其他 announcement 之后，设有上限。整个 ring 期间都挡着它仍然不对——HA 持有 `_is_announcing`，15 分钟的 `MAX_RING_S` 会废掉所有其他 announcement——但那是 UNBOUNDED 等待。等在飞的那个 burst 不到两秒；把警告读成两者都禁，正是这个问题一直悬着的原因。

  未决：alarm 的 duck 深度想要独立取值而非 `duckDb`（后者是按"语音下的音乐垫"调的）；解除后音乐恢复在不可 seek 的流上是回到直播边缘，30s 闹钟吃掉 30s 曲目。

  **共享的 `playback_done` Event 已修**（#481）：现在是每播放一条等待者的 FIFO 队列，一次设备报告不再喂饱两个等待者。

- **Q3 —— 耳机孔被占用时谁拥有扬声器？** 孔里有插头会拖垮整个音频子系统（#117/#141），音乐会话活跃时能把包括语音在内的一切静音。这是硬件/HAL 故障而非所有权问题，但它以所有权 bug 的面目出现，应在此记名，以免再被误诊为所有权问题。

## 8. 不变量 —— 不得破坏

1. **语音永不被 duck 衰减。** 只有音乐 plane 携带 `duckTarget`。
2. **语音 turn 绝不 flush 音乐 plane**（上文 F4）。
3. **混音器饱和、绝不回绕**（`mix_test.go:149` 锁定此行为）。
4. **播放完成来自设备**，不来自时长估计。
5. **`speaker_busy` 在 `finally` 中释放。** [today]
6. **帧类型按方向限定作用域。** `0x04`/`0x05` 不可随意复用。
7. **音乐 plane 任一时刻恰有一个生产者，且 HA 赢。** 写循环只在 `0x04` 为空时拉取 Sendspin 播放器，播放器在 `0x04` 有内容时上报自己被占用。两者求和 = 两条不相干的流同时响。不变量 2 在此多一条理由：为语音 turn flush 音乐，会丢掉同步 group 指望的音频并强制 resync。
