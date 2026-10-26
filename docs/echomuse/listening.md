# 聆听（Listening）：谁做决策，音频何时离开 Echo

> 整理自上游 [wilbowes/EchoMuse docs/listening.md](https://raw.githubusercontent.com/wilbowes/EchoMuse/main/docs/listening.md)（快照 2026-10，main 分支）；以英文原文与固件源码为准，本文仅供本仓库开发参考。

本文是「Echo 如何监听唤醒词、麦克风音频何时离开设备」的规范，是固件（`device/internal/listen/`）、控制器（`controller/em_listen.py`）、线上协议（`docs/device-controller-interface.md`）以及一切面向用户隐私声明的参考依据。如果代码与本文不一致，二者必有一处是 bug。

## 两种模式，按每台 Echo 分别选择

| 模式 | 存储值（`owwOnDevice`） | 唤醒词在哪运行 | 音频是否离开 Echo |
|------|--------------------------|----------------|-------------------|
| **On this Echo**（默认） | `on` | Echo 本机 | 仅在你与它对话时 |
| **On the controller** | `off` | 控制器 | **全程持续**，流向局域网内的控制器 |

没有第三种面向用户的模式。`shadow`（两个检测器对同一条连续音频流打分、用于对比）仍作为**开发者诊断模式**存在：不在管理台提供，只能通过 config API 设置。已处于该模式的 Echo 会如实显示为 streaming——因为它确实在推流。

每台 Echo 恰好处于一种模式。整组设备可以混用两种模式，且仲裁（见 *Arbitration*）能跨混合部署工作。管理台显示每台 Echo 的模式，明确标示推流状态，并汇总整个编队（"1 of 3 Echoes streams audio continuously"）。

存储值沿用三模式设计（`off`/`shadow`/`on`）的取值，使既有配置与固件含义不变；变的只是展示层。

## "On this Echo" 的确切含义

**在本机唤醒词检测触发之前，没有任何音频离开 Echo。** 触发后，Echo 推流唤醒词之后的音频，直到以下任一条件终止：

1. 控制器宣告语句结束（Home Assistant 的 end-of-speech；若 HA 的判定从不触发，则用控制器自身的 endpoint）；
2. 控制器拒绝这次唤醒（另一台 Echo 赢得仲裁、没有 Home Assistant 连接、已有 turn 在跑且 barge-in 关闭）；
3. 控制器在 `ackTimeout`（3s）内没有确认（ack）这次唤醒；
4. 会话已开启 `maxOpen`（30s）；
5. Echo 被静音，或数据连接断开。

规则 3–5 在 **Echo 端**强制执行，因此消息丢失、控制器崩溃或网络分区都无法让它无限推流。

回复播放期间，Echo 仍在**本地**继续聆听。在回复播放中说出的唤醒词（barge-in）由 Echo 本机检测，随后按上述规则原样开一个新会话。只要没听到唤醒词，回复播放期间不会有任何音频外发。

只有两种情况会在没有本地唤醒时打开麦克风，且都是用户主动要求或被要求说话：**action button（实体按键）**，以及 Home Assistant 的**追问**（follow-up question，即 "continue conversation"，或 `ask_question` / `start_conversation` action）。两者都走 bounded turn stream（`mic_start` 且 `lock_mic:true`），由 Echo 按语音门控、静默 5s 后自行结束；控制器在 end-of-speech 时关闭它（与关闭会话同一方式），Echo 回到本地聆听。

### 什么可以诚实宣称，什么不可以

- 可以说：*只有听到唤醒词后音频才会离开 Echo，且只持续到你说完为止。*
- 可以说：*一次误唤醒会送出几秒你并不想发的音频。* 这是所有唤醒词系统的共性（Amazon 也一样），我们主动公开而不是被人抓出来。
- 不可以说 *音频从不离开设备* 或 *完全本地*：指令会送到控制器，再送到 Home Assistant 的 speech-to-text——运行在哪取决于用户的配置。

## 从 Home Assistant 关闭唤醒词

Home Assistant 在 Echo 设备页上的 **Wake word** 选择器可关闭唤醒词（"No wake word"）再打开。关闭后，唤醒词不再启动 turn；麦克风保持活跃，因此按键和 HA 主动提问（`ask_question`、`start_conversation`）仍然可用。

对处于私密聆听（On this Echo）的 Echo，关闭指令直达 Echo 本身（`wakeWordEnabled`，capability `wake_word_off`），它在"越线"处停下，因此什么都不会外发。没有该 capability 的旧固件会先开会话并送出音频、之后才被控制器关掉，所以对这类固件关闭操作会被**拒绝**，选择器弹回原状。向控制器推流的 Echo 则由控制器直接停止。

无论唤醒词在哪一侧检测，off 行为一致（#778）。关闭后唤醒词既不能启动 turn、也不能打断回复；由 HA 或按键启动的 turn 仍会聆听，追问也一样。关闭期间不推任何流：若一台唤醒词已关闭的 Echo 仍发来音频，控制器会停掉这条流。该场景是在真机上发现的：一台从私密聆听切到 "On the controller" 的 Echo 在 "No wake word" 下让一条流持续跑了 31 秒。

## Echo 可能处于的状态

控制器为每台 Echo 解析出以下状态之一（`em_listen.resolve`）并在管理台显示。配置说的是"要求了什么"，这里说的是"实际是什么"。

| 状态 | 含义 | 是否持续推流？ |
|------|------|----------------|
| `local` | Echo 本机唤醒词，本地聆听 | 否 |
| `controller` | 按配置由控制器检测唤醒词 | **是** |
| `diagnostic` | `shadow`，两个检测器都在跑 | **是** |
| `legacy` | 要求 On this Echo，但固件早于 `oww_local_only` | **是** —— 提示 "update firmware for private listening" |
| `degraded` | 要求 On this Echo，但 Echo 无法评分（运行时或模型缺失、加载失败） | 否 —— **仅按键可用**，并说明原因 |
| `unknown` | Echo 尚未上报 | 显示为 unknown，绝不显示为 private |

**`degraded` 绝不回退为推流。** 无法运行本机唤醒词的设备要么坏了、要么在等安装，它仍能响应按键。悄悄改成推流会让管理台的隐私声明在无人选择的情况下变成假话。

`legacy` 确实推流，也如实显示推流。这是当前固件的行为；解决办法是升级固件，管理台会如此提示。

状态来自 Echo 本身（`/control` 上的 `listen_state`），绝不来自控制器对配置的解读：只有 Echo 自己知道它的评分器是否加载成功。`capabilities` 里的 `oww_local_only` 说明固件**能否**；`listen_state` 说明**是否正在**。

## 会话协议（session protocol）

这一切都是协商出来的：Echo 宣告 `oww_local_only`，控制器在其 `ack` 中宣告 `listen_session`。**两者都在场时本地聆听才启用。** 面对旧控制器，Echo 保持现有行为（持续推流、用本机唤醒触发），因为旧控制器只在有帧到达时才响应设备侧唤醒。

### 消息

Echo → 控制器，`/control`：

| `type` | 字段 | 含义 |
|--------|------|------|
| `listen_state` | `state`（`local`/`stream`/`degraded`）、`reason?` | 每次变化时以及每次 `ack` 之后发送 |
| `oww_wake` | `score`、`threshold`、`ageMs`、**`session`**、**`floor`**、**`barge`** | 本地唤醒已打开会话 `session`。`ageMs` 是唤醒词最后一帧被**捕获**距今的时长。`floor` 是 Echo 的噪声底（RMS）。`barge` 为 true 表示说话时扬声器正在播放 |
| `listen_end` | `session`、`reason` | Echo 自行关闭了会话（`ack_timeout`、`max_open`、`muted`、`link`） |

控制器 → Echo，`/control`：

| `type` | 字段 | 含义 |
|--------|------|------|
| `listen_ack` | `session` | 唤醒已被接管；停掉 `ackTimeout` 计时 |
| `listen_close` | `session`、`reason` | 结束会话。**`session` 不是当前打开的那个时忽略** |

`mic_stop` 在私密聆听下含义略有变化：它结束 bounded turn 流，但从不结束会话（会话只能按 id 结束），也从不打断 Echo 的本地聆听——正是本地聆听在听回复上的 barge-in。`mic_start` 且 `lock_mic:true` 会在该 turn 期间取代本地唤醒流。

Echo → 控制器，`/data`：

| 码 | 布局 | 含义 |
|----|------|------|
| `0x07` | `[0x07][session u32 BE][seq u16 BE][PCM]` | 会话音频，单声道 `S16_LE` 16 kHz |

### 为什么会话要编号

控制与数据走不同 socket，所以一个会话的首段音频可能先于或晚于它的 `oww_wake` 到达，某个会话的迟到帧也可能在控制器已经转移之后才来。给每帧打上 session 标签后，这两者都无害：控制器为尚未听说过的会话暂存帧（有界，`PENDING_MAX_S`），唤醒消息一到就交付；已关闭会话的帧直接丢弃。过去的无标签音频靠一个 flag 路由，flag 翻转瞬间落错侧的一帧，会成为*下一位*用户指令的前半秒。

### 会话的首段音频

Echo 维护一个近期已处理音频的环形缓冲（`ringMs`，2s），记录每 80ms 帧的捕获时间。唤醒上报越线帧的捕获时间；会话从该时刻**前 260ms** 开始（`listen.WakeLead`），然后接续实时音频。评分器通常在"唤醒词之后接着说出的命令"的前几帧才越线，从越线处起播会吃掉第一个词：9 次 VVV 捕获中有 5 次把 "what time is it" 说成 "time is it?"（2026-10-23）。提前量取 240–280ms 时，九次转写全部完整且不含 "Jarvis"；该扫描在单个说话人上的裕量是 ±20ms。提前量按"从越线帧往回数帧数"计算，因为麦克风按 160ms 成对交付帧、时间戳抖动达数十毫秒。barge-in 的越线以其两帧中的第一帧计日；若按第二帧计，会话会晚一帧开始，"stop" 到达 speech to text 时就成了 "Nope"。在这些捕获里，这段提前量实际补上的正是唤醒词的结尾。控制器把这段全部交给 speech to text：`VOICE_PREROLL_DISCARD`（240ms）只作用于控制器自己评分的流，因为对会话流它会吃掉紧跟唤醒词说出的命令首词（`em_listen.tail_discard`）。在回复播放上说出唤醒词同样按此开流、同样处理。Echo 在越线处切断自己的回复（`listen.CutReplyOnBarge`），但声音还会再响最多约 210ms（ALSA ring 加一个 period），这段音频也会被发送。曾尝试丢弃它（2026-10-23）后又移除：它修好了"唤醒词后停顿再说 'Oh, never mind'"的场景，但一旦会话提前开始且 Echo 自切回复，该场景本来就干净；而加上丢弃后，紧跟唤醒词的命令会丢开头（"what time is it" 只剩 "it."）。

时间戳来自把帧同时交给评分器与环形缓冲时的同一个 `time.Now()`，因此评分器的排队延迟（忙时可达 640ms）绝不会移动会话起点，`ageMs` 也从捕获时刻起算而非推理完成时刻。

## Echo 上的唤醒阈值

Echo 评分阈值取：

- 常规为 `owwThreshold`；
- 扬声器在播回复或闹钟、**或**（barge-in 已启用时）在播音乐，取 `bargeInThreshold`——与控制器侧同一规则；
- 且在该低门槛生效期间，需要**连续两帧**高于它（`em_barge.decide` 的规则），因为低于唤醒阈值十倍的单帧门槛，曾因助手自己的语音而触发。

barge-in 关闭时，回复播放中听到的唤醒仍会上报、由控制器拒绝（`listen_close`），因此规则只留在一处。

## 仲裁（Arbitration）

先**听到**者赢，不是先到达者赢。每条 claim 携带其音频被**捕获**的时间，换算到控制器时钟，且换算方式保证在途时间无法移动它。家庭 WiFi 会丢包，TCP 会重传，消息可能晚到数秒；到达时间恰恰是那个会说谎的数字。

- **控制器评分的唤醒**以越线帧计日。持续流每 80ms 发一帧（含静音），所以第 *n* 帧在流开始后 *n* × 80ms 被捕获；控制器从延迟最小的帧学习那个起点（`em_listen.CaptureClock`，一个跟随 Echo 时钟相对漂移的滑动最小值）。被重传扣住一秒的帧仍按捕获时刻计日，在控制器自己队列里等待评分的帧也一样。
- **Echo 检测的唤醒**携带 `capturedMono`，即 Echo 单调钟上的捕获时刻。控制器用每 5s 已交换的 ping 应答把该钟映射到自己的钟，每个应答都带 Echo 的 `mono`：最近两分钟内往返最短的那个应答，把映射钉在该往返时长一半的精度内（`em_listen.DeviceClock`，NTP 同款规则）。重传只会拉长往返，所以永远不会被选中。不发 `capturedMono` 的固件回退到 到达 − `ageMs` − 平滑 RTT 的一半，仅当消息未被延迟时才正确。

仍未修正的是任何帧或 ping 曾有的最小延迟（几毫秒量级），以及 Echo 的发送合批（至多一个 80ms 帧）；两者都远小于 `wakeArbitrationMs`。两个估计值都不会晚于到达时刻、也不会比它早 3s 以上。

若一条 claim 在当前赢家被听到时刻的 `wakeArbitrationMs` 邻域内被听到——无论它何时到达——则它让位。已授予的 claim **绝不撤销**——那会掐断已在聆听的 turn——所以赢家是"窗口内被听到"的 claim 中第一个到达的，且赢家被持有 `wakeArbitrationMs` 加 3s，让迟到的 claim 仍能找到它。持有无成本：另一个房间的独立唤醒靠"被听到的时刻"区分，而非"到达的时刻"。3s 正是 Echo 的 ack 超时；更晚的私密唤醒早已自行关闭会话，控制器也会忽略针对 Echo 已关会话的唤醒。

**每场竞赛都会举行，最响的 Echo 获胜**（#747）。只要两台以上 Echo 可能主张，第一条 claim 被持有到它被听到后 250ms（`ARB_HOLD_S`，扣除它已在途的时间），届时收集窗口内听到的所有 claim。不撤销任何东西：持有结束前没人握有 turn。

然后选出一名：**最响者**。响度是 Echo 实测的唤醒平均电平，已过回声消除、并除去了 mic gain 设置的影响。没有裕量。必须做对的是"房间"：跨房间的领先很大；同一房间内的电平只差一两分贝，而其中哪台应答无关紧要。响度只在可信时使用，以下情况竞赛回退到最早听到者：

- 任一 Echo 没送电平（旧固件）；
- 任一 Echo 是在自己扬声器播放中听到的唤醒——此时读数里含回复或音乐；barge-in 必属此类；
- 各 Echo 的 MICPGA 或数字增益设置不同——这些仍在读数之内。

为什么不用时间：评分器按 80ms 帧工作，而帧网格起点在各 Echo 上不同，"听到顺序"多半是帧对齐的产物。三台并排的 Echo 对同一个词的报告相差 89 至 224ms（2026-10-06）；而声音传一米只需 3ms。某宅十四次有争议的唤醒中，最响的 Echo 每次至少领先 9.7 dB，而 first-to-hear 只在其中八次选中它。

未处理：麦克风偏弱的 Echo 读数偏小，可能把 turn 输给更远但更响的一台（#731）；v2.18.0 之前的固件上，emOS Echo 的读数约比 FireOS 5 低 6dB（#806）。单 Echo 家庭从不等待。2026-10-05 之前只有混合编队（部分 Echo 本机检测、部分由控制器评分）才举行持有，且总是最早听到者赢。播放中的 barge-in 在两帧中的第二帧触发、按第一帧计日。

唤醒日志行会为控制器评分的唤醒报告：到达后多久才被评分、其帧在途多久。

## 各模式的成本与损失

| | On this Echo | On the controller |
|--|--------------|-------------------|
| Echo CPU | 常驻约 0.4 核 | 唤醒词零开销 |
| WiFi | 空闲时为零 | 每台约 32 KB/s，常驻 |
| 管理台 near-miss 计数器 | `—`：Echo 自己的窗口统计带有其峰值分数 | 在控制器上计数 |
| 控制器对比分数（`ctrl_wake_score`） | NULL——无从比较，也不记为 miss | 有记录 |
| 控制器重启后存活 | 唤醒仍被检测，但无人应答 | 否 |

缺失的测量存为 NULL，绝不存 0：没推流的 Echo 没有控制器分数，这不等于"控制器打了零分"。
