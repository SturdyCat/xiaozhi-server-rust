# Dot 2 LED 灯环状态模型（中文整理版）

> 整理自上游 [wilbowes/EchoMuse docs/led-ring-states.md](https://raw.githubusercontent.com/wilbowes/EchoMuse/main/docs/led-ring-states.md)（快照 2026-10，main 分支）；以英文原文与固件源码为准，本文仅供本仓库开发参考。

12 颗 LED 组成的灯环（外加静音键上那颗独立 LED）是 EchoMuse 的全部用户界面：没有屏幕、没有其他指示，设备正在做什么全靠灯环表达。

**核心要求**：灯环必须不惊扰（unjarring）、前后一致（consistent）、可被理解（understandable）。亮了却没有下文的灯环，比从不亮更糟。

本文是权威行为规范：每一个灯环所有者、每一个事件及其结局。条目标注：

- **[today]** — 成文时已上线的行为（本文写于 v2.9.7；更晚的条目会自注版本）
- **[proposed]** — 尚未实现、待批准的设计

---

## 1. 设计原则

1. **绝不展示系统兑现不了的状态。** 乐观反馈只允许在「可界定且可了结」处使用——能被控制器确认，或能被可见地撤回。
2. **反馈在本地，决策在远端。** 设备拥有它第一手知道的事（按键被按下、自身扬声器缓冲排空、被静音）；控制器拥有需要设备缺失信息才能判定的事（是否说了唤醒词、HA 是否应答、哪台设备该回应）。
3. **一个信号只承载一个含义。** 橙色已经表示「控制器不可达」，新的失败模式要归并进既有词汇，而不是发明用户必须重新学习的信号。
4. **每个临时状态都必须了结。** 任何状态不得凭假设无限期保持：临时上灯带截止时间（deadline），动画带 TTL 死人开关（dead-man）。
5. **设备主权状态永不被网络覆盖。** 静音是范本：控制器缺席、卡死甚至撒谎时它都必须正确。

---

## 2. 灯环所有者——优先级阶梯

活跃的最高层赢得实体上灯；低层仍然把状态记录进 `baseLEDs`，以便到期后正确交还。

| # | 所有者 | 视觉 | 生命周期 | 主权归属 | 代码 |
|---|---|---|---|---|---|
| 1 | **Volume arc**（音量弧） | 青色，12 颗中按比例亮 N | 2s 窗口（`volumeLEDSecs`） | 设备本地；物理按键，以及选择加入后的远端音量变化 | `volume.go:145` |
| 2 | **Mute ring**（静音环） | 实心红 `(180,0,0)` + 按键 LED（gpio444，高电平有效） | 直到解除静音；跨重启与 OTA | **设备主权**，持久化于 `/data/local/etc/echomuse/state.json` | `mute.go:132` |
| 3 | **Link state**（链路状态） | 橙色正弦脉冲（断连）/ 白色慢脉冲（待批准） | 直到链路解决 | 设备本地 | `cmd/server.go:161,174` |
| 4 | **Turn / media animation**（回合/媒体动画） | `solid`·`spin`·`rotate`·`pulse`·`meter`·`off`，按场景着色 | 直到被替换或 `ttlSec` 到期（listening 30s / spinner 135s / meter 按每次响应） | 控制器指定、设备渲染 | `animator.go:53` |
| 5 | **Direction overlay**（方向叠加） | 基础环色朝白提亮于波束角 | listening 环存在期间 | 设备本地，需要 `listeningLEDs` | `server.go:269` |
| 6 | **Idle**（空闲） | 全灭 | — | — | — |

### 抑制规则 [today]

层 1 与层 2 都会对层 3–5 抑制硬件上灯，同时照常写入 `baseLEDs`：

```go
if s.volume.DisplayActive() || s.mute.IsMuted() {
    return          // server.go:381 (SetLEDs), server.go:275 (SetDirectionLEDs)
}
```

层 1、层 2 之间：音量弧在其窗口内获胜——两者都直接刷硬件，而弧最后刷。它的到期定时器是静音感知的，静音时恢复红环而不是交还控制器：

```go
if vc.isMuted != nil && vc.isMuted() {
    lc.SetLEDs(redRing...)      // volume.go:177
} else if expire != nil {
    expire()                    // → paintBaseLEDs()
}
```

层 4 动画跑在设备自己的 ticker 上，每一帧都经 `SetLEDs` 路由，因此继承上述抑制并保持 `baseLEDs` 最新。这使音量弧在动画进行中交还也无缝：动画器在被抑制的窗口里一直在更新 `baseLEDs`，`paintBaseLEDs()` 画的是*最新*一帧，下一次 tick（≤80ms）即恢复正常。替换是原子的（generation 计数器）——过期的动画 goroutine 永远画不到它的继任者之上（`animator.go:45-48`）。

---

## 3. 链路可用性——三态，不是两态

| 状态 | 含义 | 允许本地反馈？ |
|---|---|---|
| **LINKED** | 控制 WS 已注册*且*有近期入站证据 | 是 |
| **SUSPECT** | 套接字 believed up 但无近期证据，或某次交互未获确认 | 否——按 DOWN 处理 |
| **DOWN** | 套接字已关闭，或等待批准 | 否——展示链路状态（层 3） |

### 现状的问题 [today]

```go
func (c *ControlClient) IsConnected() bool {
    return c.conn != nil        // control.go:131
}
```

`c.conn` 仅在读循环出错时置空，读循环受 `wsPongWait = 45 * time.Second`（`data.go:61`）管辖。于是对一条悄悄死掉的链路（控制器停机、WiFi 黑洞、控制器事件循环卡死），设备会在控制器消失后**继续报告 connected 长达 45 秒**，橙色脉冲也不启动。用 `IsConnected()` 门控乐观上灯，就有 45 秒窗口：灯环自信地亮着却什么都不会发生——而这恰恰是用户最可能去按键的时刻（正因为没反应才按）。字面满足要求，实质完全违背。第二个缺口：套接字活性看不见「连着但无法服务」的控制器——HA 挂了、pipeline 报错、事件循环停摆。

### 被动检测有下限 [proposed]

空闲期的入站证据来自控制器每 **30 秒**一次的 ping 循环（`em_controller.py:1674`），所以被动新鲜度阈值小于 ~35s 必然误报。**被动活性单独给不出快速检测**——这是下述设计改建立在「逐交互确认」上的结构性原因。

### 逐交互确认 [proposed]

不问「控制器在吗」，问「控制器确认了*这次*交互吗」：

- 本地交互发生：立即上灯，并武装**确认截止时间**（confirmation deadline）。
- deadline 前任何拥有灯环的控制器消息（`led_anim` / `leds`）都构成确认并替换临时上灯。**该机制已存在**——generation 计数器干的就是这件事，协议零改动。
- deadline 到期仍无消息 → 交互未被受理：转入 DOWN，显示橙色脉冲（层 3）。

量化依据：控制器在按钮事件到达后**亚毫秒**内发出响应，deadline 只需覆盖两个网络跳。79 次唤醒回合实测的上行时延（检测 → 第一帧麦克风）：

| 设备 | 中位 | p90 | 最大 | RSSI |
|---|---|---|---|---|
| Office | 264ms | 272ms | 278ms | −25 |
| Lounge | 260ms | 294ms | 952ms | −52 |
| Retreat | 258ms | 1046ms | 1225ms | −66 |

尾部随 RSSI 恶化，所以 deadline 必须按 Retreat 的最坏情况而非 Office 的典型值设定。见 §6 Q3——deadline 在顺利路径上零成本，因此应从宽设定，让 RTT 埋点保持「确认性」而非「阻断性」。

附带收益：一次未确认的交互是比 45s pong 超时快得多的链路故障探测器。按钮成为主动探针，本地反馈与连接感知互相强化而非互相冲突。

---

## 4. 事件 → 结局表

下文使用的状态名：`IDLE`、`LISTENING`、`THINKING`、`PLAYING`、`MUTED`、`VOL-DISPLAY`（2s 弧窗口）、`DISCONNECTED`、`PENDING`。

### 4.1 Action（dot）按钮——clickType 138

| # | 状态 | 链路 | 设备动作 | 灯环结局 | 控制器 | 状态 |
|---|---|---|---|---|---|---|
| A1 | IDLE | LINKED | 发送按钮事件 | **控制器回复前什么都不亮**——迟一个 RTT | 开始回合，发送 `led_anim` listening | [today] |
| A2 | IDLE | LINKED | 发送按钮事件**并本地画 listening 环**，武装确认 deadline | listening 环**立即**出现 | 以 `led_anim` listening 确认、替换临时上灯 | [proposed] |
| A3 | IDLE | LINKED，deadline 前未确认 | 撤回临时上灯，标记链路 DOWN | **硬切**橙色脉冲——不渐变（§6 Q4） | — | [proposed] |
| A4 | IDLE | DOWN / SUSPECT | 不发送、不画回合状态 | 橙色脉冲继续（不变） | — | [proposed] |
| A5 | LISTENING / THINKING / PLAYING | LINKED | 发送按钮事件 | 控制器的清理到达时环熄灭 | 取消回合（`cancel_event` + `speaker_flush`）；**仅本地——HA 的 pipeline 跑到完成，结果丢弃**（`em_esphome.py:1158`） | [today] |
| A6 | LISTENING / THINKING / PLAYING | LINKED | 发送按钮事件**并本地清环**（回合状态中的按压无歧义=取消） | 环**立即**清空 | 同上取消 | [proposed] |
| A7 | **MUTED** | tap | 携 `muted: true` 发送；**控制器拒绝该回合**（`em_button.decide` → `BLOCKED`） | 红环不变——**刻意沉默**（见 §6 Q1） | 无。麦克风不开：静音时设备侧拒绝 `mic_start` | [today] |
| A8 | **MUTED** | hold | 携 `muted: true` 发送 | 红环不变 | **向 HA 发 `long`**。hold 不是语音，静音对它没有意见 | [today] |
| A9 | VOL-DISPLAY | LINKED | 发送按钮事件**并取消弧的持有**（`CancelVolumeDisplay`） | 弧停止主权；回合的 listening 帧一到即画 | 正常开始回合 | [today] |

### 4.2 静音按钮——clickType 113（设备本地，永不出设备）

| # | 状态 | 设备动作 | 灯环结局 | 控制器 | 状态 |
|---|---|---|---|---|---|
| M1 | IDLE | ADC 静音全部 4 对 codec，持久化 state.json，按键 LED 亮 | 实心红；**抑制全部控制器上灯** | 经 `mute_state` 获通知 | [today] |
| M2 | LISTENING / THINKING / PLAYING | 同 M1 | 立即实心红；被取消回合的 LED 清理后到、被正确忽略 | 取消回合 + `speaker_flush` | [today] |
| M3 | MUTED（解除静音） | ADC 解除、清环、按键 LED 灭、持久化 | 环变黑；下一控制器帧重画 | 经 `mute_state` 获通知 | [today] |
| M4 | VOL-DISPLAY | 同 M1——红被直接画 | 红获胜（后画）；弧到期是静音感知的、恢复红 | 获通知 | [today] |
| M5 | DISCONNECTED | 同 M1——**完全没有控制器也工作** | 实心红取代橙色脉冲 | 无 | [today] |
| M6 | 重启/OTA 后开机且静音中 | `RestoreMuted()`——连接前恢复 ADC + 标志；LED 初始化完成即亮环 + 按键 LED | 红，先于任何控制器接触 | 无 | [today] |

静音是原则 5 的参考实现，其行为**不变**。

### 4.3 音量按钮——clickType 115 / 114（设备本地）

| # | 状态 | 设备动作 | 灯环结局 | 状态 |
|---|---|---|---|---|
| V1 | IDLE | `Set(level, showRing=true)` | 青弧 2s → 黑 | [today] |
| V2 | LISTENING / THINKING / PLAYING | 同 V1 | 青弧 2s → 帧中途交还给进行中的动画 | [today] |
| V3 | MUTED | 同 V1 | 青弧 2s → **恢复红环**（到期静音感知） | [today] |
| V4 | DISCONNECTED | 同 V1 | 青弧 2s → 橙色脉冲恢复 | [today] |
| V5 | 远端设音量（控制器 / HA / Sendspin），该设置关闭 | `Set(level, showRing=false)` | 无弧——默认行为 | [today] |
| V6 | 远端设音量，`remoteVolumeArc` 开，有效音量变为非零 | `Set(level, showRing=true)` | 青弧 2s，之后正常交还 | [today] |
| V7 | 远端设 0、重复当前音量、开机 `SeedVolume` | `Set(level, showRing=false)` | 无弧——静音、状态同步与恢复保持沉默 | [today] |

### 4.4 控制器来源的灯环消息

| # | 消息 | 状态 | 灯环结局 | 状态 |
|---|---|---|---|---|
| C1 | `led_anim` listening（`solid`，`listening:true`） | 任意未抑制 | 场景 listening 色；启用方向叠加 | [today] |
| C2 | `led_anim` `spin`/`rotate`（thinking） | 任意未抑制 | 设备 ticker 上的转轮，80ms | [today] |
| C3 | `led_anim` `meter`（playing） | 任意未抑制 | 随 ALSA 写入处的实时扬声器 RMS 起伏——**仅语音平面**，在音乐混音之前测量（v2.10.0）。AEC 远端抽头刻意取混音输出（那正是要从麦克风里消掉的东西）；meter 绝不能取，否则会对没人要它可视化的歌曲起伏 | [today] |
| C4 | `led_anim` `off` | 任意未抑制 | 环变黑 | [today] |
| C5 | 任意 `led_anim` / `leds` | MUTED 或 VOL-DISPLAY | **只记录进 `baseLEDs`，不上灯** | [today] |
| C6 | 旧式 `leds` 帧 | 任意未抑制 | 原子替换进行中的动画（generation 计数器） | [today] |
| C7 | `ttlSec` 内无替换 | 动画运行中 | dead-man 清环——防控制器回合中途死亡 | [today] |

### 4.5 音频 / 链路生命周期

| # | 事件 | 状态 | 灯环结局 | 状态 |
|---|---|---|---|---|
| L1 | 控制 WS 注册 | PENDING / DISCONNECTED | 脉冲停；静音中恢复红环，否则交还 | [today] |
| L2 | 控制 WS 关闭（读循环出错） | 任意 | `StopAnim()` 后橙色脉冲 | [today] |
| L3 | 等待管理员批准 | 开机 | 白色慢脉冲 | [today] |
| L4 | 链路悄悄死亡 | 任意 | **最长 45s 毫无反应**——环保持最后状态 | [today] |
| L5 | 链路悄悄死亡 | 任意 | 由下一次交互（§3）或入站新鲜度超时检出 | [proposed] |
| L6 | **扬声器流结束**（收到 EOS *且*音频通道排空） | PLAYING | 控制器等 Echo 的 `playback_stats` 上报后再清环。曾按墙钟估算、慢链路上最多提前 6.1s 清环（§5） | [today] |
| L7 | 扬声器流结束 | PLAYING | 设备凭已在记录的信号（`pcm_speaker.go:309`）自行清环/交还 | [proposed] |

---

## 5. L6 为何曾是错的（已修复）

> **本文成文后已修复。** 控制器不再估算播放结束：它等待 Echo 的 `playback_stats` 上报（Echo 在 EOS 后音频通道排空时发送），再清环。以下分析保留，作为该设计的缘由。

`_run_post_turn_playback` 无从得知播放真正结束的时刻（`em_controller.py:746`）：

```python
audio_duration = len(speaker_pcm) / (SPEAKER_RATE * 2) + SPEAKER_PRIME_SECONDS
remaining      = max(0.0, audio_duration - elapsed)
```

`elapsed` 是*套接字写*时间且被**减去**——估算恰恰在需要最久的那些流上缩水。对 `delivery_ms`（设备 `playback_stats` 到达——真正的音频结束信号）比对最近 40 个埋点回合，有 4 个提前清环逾 0.5s：

| 回合 | tts | send_ms | delivery_ms | recv_span | 清环时刻 |
|---|---|---|---|---|---|
| 07-24 23:53 Retreat | 2.6s | 1154 | 9764 | 8193 | **早 6.1s** |
| 07-24 22:31 Lounge | 5.0s | 4296 | 9353 | 2039 | **早 3.2s** |
| 07-24 05:26 Lounge | 9.6s | 5364 | 13833 | 8275 | **早 3.1s** |
| 07-22 22:14 Lounge | 4.6s | 1 | 7417 | 4112 | **早 1.7s** |

每一个都与膨胀的 `recv_span_ms` / `max_gap_ms` 相关。健康回合的估算约富余 1s，所以这个问题只是偶发暴露。设备是唯一知道自己缓冲何时排空的一方，且已经在检测并记录它。按原则 2 这归属设备，且**无需确认**——没有预测任何东西。

---

## 5b. deadline 到期的了结 [proposed]

确认 deadline 到期，交互即告结束。四件事必须发生，第三件最有牙齿。

**1. 灯环——硬切橙色**（§6 Q4）。设备此刻处于链路态 DOWN。

**2. 本地还原麦克风。** 到期时设备可能已收到 `mic_start_turn`（控制器发送顺序中它在 `led_anim` 之前），或已有受限回合流在跑：必须停止并回到连续唤醒流——`StopMic()` 然后 `StartMic(false)`。

**3. 通知控制器放弃该回合。** 尽力而为的 `turn_abort` 控制消息。**这是强制的，不是可选的，理由是隐私而非整洁**：没有它，控制器会跑完整语音回合——把麦克风音频流给 HA——而设备亮着橙色。「正在收音却没有灯」正是绝不许存在的状态。控制器释放 `voice_lock`、取消回合、停掉麦克风流。常见情况下 abort 能到达：迟到回复的常见原因是控制器*慢而未死*。链路真断时，重连已重置灯环（配置推送带 `leds_off`，`em_controller.py:1638`），控制器自己的回合也随套接字一起失败。

**4. 拒绝被弃交互的迟到者。** 针对 abort 与迟到 `led_anim` 在途交叉的竞态保险：设备为每次本地交互盖一个单调递增的**交互 epoch**，随按钮事件发送，控制器在该回合的灯环消息上回显。携带早于设备当前 epoch 的 `led_anim` 一律丢弃。这与动画器已有的 generation 计数器（`animator.go:45`）是同一模式、延伸到线上——过期的响应永远盖不住它的继任者。一刀切忽略 `led_anim` 一段时间是错的：会连带压掉第二次按压或唤醒词触发的合法*新*回合。

### 与既有按钮竞态的相互作用

`handle_button_event` 有一个潜在竞态，了结语义会把它从无害变成用户可见。start/cancel 判定读 `device.voice_lock.locked()`，但派生的任务要到数次 await 之后才拿锁：

```python
if device.voice_lock.locked():   # em_controller.py:1433
    ...cancel...
else:
    _btn_task = asyncio.create_task(_button_voice_turn())   # 稍后才拿锁
```

窗口内的两次按压都看到锁空闲、都排队一个回合；第二回合的 `led_anim` 被第一回合的耗时推迟——轻松超过任何合理 deadline。了结语义下，设备会变橙、abort，控制器却仍会跑排队的回合。**所以这个修复必须进同一变更**：让 start 路径在 handler 内同步占住回合（spawn 前设 `turn_pending` 标志，或在 handler 内拿锁），使 cancel/start 判定原子化。2026-07-25 实测未观察到（按压间隔 ~700ms 未相撞），但快速双击会。

---

## 6. 决策

### Q1 — 静音中被拒的按压：**保持沉默**（已决）

红环加上红色静音键 LED 足以表明设备听不见；Alexa 也是这样做的，大量焦点组工作已经定过的行为无须重造。无重申闪烁、无拒绝信号。A7 行维持原样。

**2026-08-08 修订。** 上述「沉默」决定不变，但被拒的范围收窄了。设备过去丢弃静音期间的每一次 dot 按压——在按钮只意味着「开始语音回合」时是对的，hold 也会发 HA 事件之后就错了：绑定到与语音无关动作的 hold，在麦克风关闭时失灵，且灯环上毫无能联系二者的线索。现在按压携带静音状态，控制器只拒**回合**（A7 行）；hold 照发（A8 行）。静音在设备上仍然主权——主权从来不是按钮过滤器，它是 ADC 静音加设备拒绝静音期间的一切 `mic_start`。

### Q2 — 设备如何知道自己在回合中：**给 `led_anim` 加语义字段**（推荐）

只影响 A6 行（取消按压立即清环）。没有它，设备得在*每一次* dot 按压时画 listening——若该按压本是取消，就为一个错误状态闪一个 RTT。

- **方案 A——从当前动画推断。** 动画器只存 `{mu, gen}`（`animator.go:45`），无 pattern：需要把当前 pattern 记在 generation 旁，约三行。*这一等价关系自 2026-10-05 起不再成立。*曾经精确：`em_player` 与公告路径都不画环，所以「环显示回合状态」⟺「有语音回合活跃」。#780 起，公告、以及 Home Assistant 发起的对话的开场消息，会在没有语音回合时显示 playback meter——方案 A 会把公告期间的按压读成取消。*风险是未来的脚枪，不是现在的 bug。*等价是隐式契约：哪天音乐播放也上了 meter 环（非常自然的特性请求），dot 按钮就在音乐期间悄悄变义，且没有任何东西大声失败——环只是开始在本该开始回合的按压上清环。
- **方案 B——从设备音频/麦克风状态推导。** 不可行。唤醒触发的回合刻意保持连续无门限流、从不发 `mic_start_turn`（P0-1），所以最常见的回合类型在 listening 阶段 `micActive && lockMic` 为**假**。而 `streamActive` 对音乐和公告也为真——A 的问题反过来。两个信号单独或组合都盖不住状态空间。
- **方案 C——控制器显式语义。** 「我的按压会被读成取消吗」的权威就是 `device.voice_lock.locked()`；任何设备侧推断都是复制品。控制器在**每个**回合状态变化时都发 `led_anim`，给这条消息加语义字段（`state: listening | thinking | playing | idle`）**零额外流量**、每回合两次跃迁。

**推荐方案 C。** 它保留 A 的零成本性质，同时去掉 A 的隐式契约——设备读一个声明出来的状态，而不是从像素颜色猜意图，未来音乐 meter 特性无法悄悄改变按钮语义。乐观上灯也因此自描述：设备乐观上灯时自设语义状态，控制器的下一条 `led_anim` 要么确认要么覆盖。陈旧性（一个 RTT）A、C 完全相同且不可再减，不构成区分；两个漂移方向都在一个 RTT 内自纠。

### Q3 — 确认 deadline：**固定 3–3.5s**（推荐）

因为 deadline **会终止回合**（Q4），误撤回如今让用户损失一个真实回合，而非一次闪烁。deadline 因此*不再免费*，其下限是最坏合法的首条入站消息时间。最坏情况是一个整往返：控制器在按钮事件到达后亚毫秒内发首响应（实测「Dot button → voice turn」与「Voice turn starting」落在同一毫秒），唯一变量是线路。Retreat 的上行腿 p90 1046ms / max 1225ms，往返最坏 ~2.5s。**3–3.5s** 留裕量覆盖它，仍是今天 45s 盲区约 13 倍的改善。偏长仍然正确：多等只是错误指示变慢，少等会杀死用户想要的回合。

**固定，而非自适应。** 逐设备自适应 deadline 为一个罕见故障引入状态和调参面。仅当实数据表明 Retreat 级设备误 abort 时再议。

**确认是任意入站控制消息，不特指 `led_anim`。** 这一点重要且不显然：按钮回合控制器依次发 `mic_stop` → `mic_start_turn` → `led_anim`，所以 `mic_start_turn` 是比灯环消息*更早*的生命证明。对任意入站流量武装 deadline 是严格更弱的条件、更早满足，因此误 abort 更少。`led_anim` 仍是灯环*状态*的唯一权威——活性与灯环状态权威是两个关注点。

*RTT 埋点*保持确认性而非阻断性——在实地验证没有合法首响应超过 deadline。

### Q4 — 临时上灯的撤回：**硬切橙色，并终止回合**（已决）

硬切让人注意到错误状态；渐变会软化本该被注意的东西。

**deadline 终止交互——不只是重画。** 设备已变橙、已告知用户「控制器不应答」之后，一个迟到的到达者复活回合只会添乱。回合在 deadline 结束，其后该交互的任何到达都被拒。

需要做什么见 §5b。注意对 Q3 的连带影响：deadline 如今终结一个真实回合而非只变色，它不再免费，下限由最坏合法往返决定。

---

## 7. 不变量——不得破坏

- **静音是设备主权。** 控制器缺席、卡死、撒谎都必须正确。本地持久化；跨 OTA 槽位切换存活。
- **音量弧在其 2s 窗口内对*动画*拥有灯环。** 回合动画每 ~80ms 重画，否则一帧内就把它踩掉。它**不**高于刻意的 action button 按压——按压取消弧的持有；弧防的是重画噪声，不是用户（2026-07-25：调完音量立刻按键，窗口期满前毫无「在听」迹象）。
- **弧到期是静音感知的。** 静音时必须恢复红，绝不交还控制器状态。
- **`ttlSec` dead-man 保留。** 它是防控制器动画中途死亡的唯一保护。
- **动画替换保持原子**（generation 计数器）。过期的 goroutine 永远盖不住它的继任者。
- **`baseLEDs` 在被抑制时照常记录。** 这是无缝交还的根基。
- **方向叠加需要 `listeningLEDs`** 且只提亮基础色——绝不画写死的绿色，在非标准场景上那读作毛刺。

---

## 8. 风险备注

LED 优先级系统是本代码库返修最多的子系统——§2 的两条上灯抑制各自都是惨胜，注释里记录着为什么。加入临时上灯层意味着新增**第四**条仲裁规则。本文提案刻意分级：风险最低、证据完全的 L7 可独立先行；携带新仲裁规则与 `led_anim` 语义字段的临时上灯层（A2/A3/A6）随后再上。
