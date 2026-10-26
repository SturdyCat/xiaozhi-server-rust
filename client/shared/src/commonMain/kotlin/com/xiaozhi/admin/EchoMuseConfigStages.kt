package com.xiaozhi.admin

import com.xiaozhi.admin.ui.*

import com.tencent.kuikly.core.base.ViewContainer
import com.tencent.kuikly.core.directives.vif
import com.tencent.kuikly.core.pager.Pager
import com.tencent.kuikly.core.views.Slider
import com.tencent.kuikly.core.views.Text
import com.tencent.kuikly.core.views.View

/**
 * 配置 tab 的 **Stage 01–08** 区块渲染（设计 §6.3 的分派表，逐 Stage 一个函数）。
 *
 * 三条硬约束（都在 §6.3 的表里，代码按表写）：
 * 1. **只渲染白名单内的键**（✅ 列）。官方 dashboard 有、我方 `admin.rs::SETTINGS_KEYS`
 *    没有的键（`streamReply`/`owwSpeexNs`/`ledScene`/`meter*`/`buttonSingleTapEvent`/
 *    `controllerEndpoints`…）**不画控件**——画了只会得到 400 或"存了但不生效"。
 * 2. 每个 Stage 头部都要写清「为什么这里比官方少」：缺白名单键、缺采集、还是官方那部分
 *    根本是**控制器侧渲染**（Stage 04 灯环、Stage 05 的 USB console）。用户看到控件少
 *    不会以为是坏了。
 * 3. 草稿一律走 [EchoMuseConfigDrafts]（同一份 observable），保存只有配置 tab 底部
 *    那一个「下发配置」——部分更新语义（数值留空 = 不发该键）在
 *    [EchoMuseConfigDrafts.buildConfigBody] 里逐键实现。
 *
 * ⚠️ Stage 07 的 `sendspinUnpaired`：设计 §6.3 表里写"→P2 补白名单"，该行 2026-10 起
 * **作废**——`settings` 白名单已是 40 键（含它，见 T02 裁决），所以这里渲染三个键。
 */

// ============================================================
// 灵敏度滑杆的宽度算术（随本文件迁入，扣减项已按新布局改正）
// ============================================================

/**
 * 滑杆之外的全部固定占位：滚动页左右留白（[AdminSpace.xl]×2）+ 卡片内边距
 * （紧凑窗口 sm、常规 cardPadding）+ 右侧阈值读数（48）+ 它左边距（sm）。
 *
 * **与旧版（`EchoMuseDeviceCard` 里的左菜单布局）的差别**：新布局没有卡内左菜单，
 * 故不再扣旧左菜单的定宽 150 + 1px 竖分隔线 + `panelPaddingLeft(AdminSpace.md)`，
 * 合计正好少 167f。少扣会算宽溢出卡片，多扣则滑杆莫名变窄（设计 §6.6 最后一行）。
 */
private fun echoMuseSliderChrome(ctx: Pager): Float =
    AdminSpace.xl * 2 +
        (if (workspaceIsCompact(ctx)) AdminSpace.sm else AdminSpace.cardPadding) * 2 +
        48f + AdminSpace.sm

/** 滑杆最小宽度（窄窗兜底；低于此值宁可窄也不为负）。 */
private const val ECHOMUSE_SLIDER_MIN_WIDTH = 120f

/**
 * 唤醒灵敏度滑杆的显式宽度。
 *
 * ⚠️ 官方 `Slider` 是**组合组件**（track/progress/thumb 三层自绘），渲染时读自身 attr 的
 * 宽高——`flex(1f)` 不参与该判定，读不到就 `throwRuntimeError`（日志「Slider组件需要设置
 * 宽度和高度」）。异常会中断所在面板的构建事务：实测整片 Stage 空白，只剩卡片 header。
 * 因此按内容区宽度算**确定值**，读取放在 attr 闭包内，窗口宽度变化时随 `pageViewWidth` 更新。
 */
private fun echoMuseSliderWidth(ctx: Pager): Float =
    (adminContentWidth(ctx) - echoMuseSliderChrome(ctx)).coerceAtLeast(ECHOMUSE_SLIDER_MIN_WIDTH)

// ============================================================
// Stage 骨架
// ============================================================

/**
 * Stage 头：分组小标题 + 一句"为什么和官方不一样"。
 *
 * [note] 必填（哪怕是"与官方一致"）——空说明会让人误以为漏了控件；[Stage] 的编号放在
 * 标题里，方便对着官方 dashboard 的 9738-10472 行逐个核对。
 */
private fun ViewContainer<*, *>.stageHead(title: String, note: String) {
    groupLabel(title)
    if (note.isNotEmpty()) textNote(note, micro = true)
}

/** 数值输入的统一占位：留空 = 不改该键（部分更新语义的唯一用户可见口径）。 */
private const val EM_KEEP = "留空不修改"

// ============================================================
// Stage 01 · 播放（官方 playback）
// ============================================================

/**
 * 播放与输出链：EQ（预设 + JSON 文本域）、等响度、播报增益、音乐压低、音量键提示音、
 * 低音保护、限幅、开机音量。
 *
 * 不渲染 `streamReply`：那是**控制器侧**（服务端播报流）的行为，设备 config 里没有这个键。
 */
internal fun ViewContainer<*, *>.stagePlayback(state: EchoMuseState) {
    val drafts = state.drafts
    stageHead(
        "01 · 播放（官方 playback）",
        "ack 已向设备声明 output_chain：EQ / 低音保护 / 限幅由设备在 ALSA 写入处直接执行" +
            "（零重采样、零延迟），这里只下发参数。官方还有 streamReply（控制器侧播报开关），" +
            "设备没有这个键，故不渲染。",
    )
    dropdownField(
        "EQ 预设（写进下面的频段数组）",
        { drafts.eqPresetLabel },
        { drafts.eqPresets }, { "" }, { drafts.eqPresetOpen },
        { drafts.eqPresetOpen = !drafts.eqPresetOpen },
        {
            val value = EM_EQ_PRESET_VALUES[it]
            if (value != null) drafts.eqBandsDraft = value
        },
        help = "预设只是把一段常用增益数组填进文本域，最终生效的仍是那里显示的数字。",
    )
    labeledField(
        "EQ 频段（数字数组 JSON）",
        { drafts.eqBandsDraft }, { drafts.eqBandsDraft = it },
        "[2,0,-1,0,2]",
        help = "按频段顺序的增益 dB 值数组；服务端校验为非空数字数组。" + EM_KEEP + "。",
    )
    switchRow("等响度（EQ Loudness）", drafts.eqLoudness) { drafts.eqLoudness = !drafts.eqLoudness }
    dropdownField(
        "语音相对音乐的提升电平",
        { drafts.responseLevels.firstOrNull { it.first == drafts.responseLevel }?.second ?: "未知" },
        { drafts.responseLevels }, { drafts.responseLevel }, { drafts.responseLevelOpen },
        { drafts.responseLevelOpen = !drafts.responseLevelOpen }, { drafts.responseLevel = it },
        help = "官方 responseLevel：低/中/高 = 0/+6/+12 dB，播报比背景音乐更响。",
    )
    switchRow("音量键提示音", drafts.volumeButtonSound) { drafts.volumeButtonSound = !drafts.volumeButtonSound }
    labeledDecimalField(
        "说话时音乐压低深度（dB，-60～0）",
        { drafts.duckDbDraft }, { drafts.duckDbDraft = it }, EM_KEEP,
        help = "语音轮次开始时把背景音乐压低该深度（duck）、结束恢复；0 = 不压低。",
    )
    switchRow("低音保护", drafts.bassGuardEnabled) { drafts.bassGuardEnabled = !drafts.bassGuardEnabled }
    labeledDecimalField("低音保护阈值（dB，-60～24）", { drafts.bassGuardDbDraft }, { drafts.bassGuardDbDraft = it }, EM_KEEP)
    switchRow("输出限幅", drafts.limiterEnabled) { drafts.limiterEnabled = !drafts.limiterEnabled }
    labeledDecimalField("限幅阈值（dB，-60～0）", { drafts.limiterThresholdDraft }, { drafts.limiterThresholdDraft = it }, EM_KEEP)
    labeledDecimalField("限幅释放时间（毫秒，10～2000）", { drafts.limiterReleaseDraft }, { drafts.limiterReleaseDraft = it }, EM_KEEP)
    labeledIntField(
        "开机音量（0～127）",
        { drafts.startupVolumeDraft }, { drafts.startupVolumeDraft = it }, EM_KEEP,
        help = "设备每次开机自恢复的输出音量（不是本次会话的当前音量，当前音量在「状态」页下发）。",
    )
}

// ============================================================
// Stage 02 · 唤醒词（官方 wakeword）
// ============================================================

/**
 * 唤醒词：开关、检测位置、模型、灵敏度滑杆、提示音与档位、麦克风编号、打断播报。
 *
 * 官方 `owwSpeexNs`/`wakeArbitrationMs`/`wakeClipCapture`/`wakeClipMinScore` 不在白名单
 * （设备 config 没有这些键），因此不渲染——这也是本 Stage 比官方少四行的原因。
 *
 * 灵敏度滑杆移植官方量程/步进/方向：左「Precise（阈值高，少误触易漏唤）」右
 * 「Eager（阈值低，易触发易误触）」，与阈值反向；轨外存储值（旧面板可写 0.05 或 1.0）
 * 把手钉在端点、读数保持存储值，拖动才写入官方量程内的新值。
 */
internal fun ViewContainer<*, *>.stageWakeWord(state: EchoMuseState, ctx: Pager, onOpenModelFiles: () -> Unit) {
    val drafts = state.drafts
    stageHead(
        "02 · 唤醒词（官方 wakeword）",
        "官方这个分组还有 owwSpeexNs / wakeArbitrationMs / wakeClipCapture / wakeClipMinScore，" +
            "设备 config 里没有这些键（服务端白名单也不收），所以本组只有 9 个控件。",
    )
    switchRow("响应唤醒词（关闭后按键仍可用）", drafts.wakeWordEnabled) { drafts.wakeWordEnabled = !drafts.wakeWordEnabled }
    dropdownField(
        "唤醒词检测位置",
        { drafts.owwModes.firstOrNull { it.first == drafts.owwOnDevice }?.second ?: "未知" },
        { drafts.owwModes }, { drafts.owwOnDevice }, { drafts.owwModeOpen },
        { drafts.owwModeOpen = !drafts.owwModeOpen },
        { drafts.owwOnDevice = it; drafts.rebuildModeOptions() },
    )
    Text {
        attr {
            fontSize(AdminType.caption)
            color(AdminColors.textTertiary)
            text(EchoMuseWakeWord.modeNote(drafts.owwOnDevice))
        }
    }
    dropdownField(
        "唤醒词模型",
        { drafts.owwModelOptions.firstOrNull { it.first == drafts.owwModel }?.second ?: drafts.owwModel },
        { drafts.owwModelOptions }, { drafts.owwModel }, { drafts.owwModelOpen },
        { drafts.owwModelOpen = !drafts.owwModelOpen }, { drafts.owwModel = it },
    )
    Text {
        attr {
            fontSize(AdminType.caption)
            color(AdminColors.textTertiary)
            text(
                "当前唤醒词：「${EchoMuseWakeWord.displayName(drafts.owwModel)}」（模型 ${drafts.owwModel}" +
                    "${if (drafts.owwModelStored.isBlank()) "，设备默认" else ""}）。官方内置共 4 款：" +
                    "hey jarvis、alexa、hey mycroft、hey rhasspy。",
            )
        }
    }
    Text {
        attr {
            fontSize(AdminType.caption)
            color(AdminColors.textTertiary)
            text(
                "保存后服务端把 owwModel 随 config 下发，设备立即换用新模型本地评分。设备只认本地已装的" +
                    "模型（开通时已带全部 4 款官方模型）；自定义模型可先上传到服务端 oww-models/ 供这里选择，" +
                    "但上传 ≠ 安装——设备没装过的模型选了也听不到唤醒词，按键仍可唤醒。",
            )
        }
    }
    // 服务端模型文件入口：上传/清理 oww-models/ 下的自定义模型（跳「文件」页对应目录）
    actionRow {
        secondaryButton("管理模型文件（oww-models/）", icon = IconToken.Folder) { onOpenModelFiles() }
    }

    fieldLabel("唤醒灵敏度（官方量程 0.10～0.975，步进 0.025）")
    View {
        attr {
            flexDirectionRow()
            alignItemsCenter()
            marginTop(AdminSpace.xs)
        }
        Slider {
            attr {
                // ⚠️ 必须是显式宽度，不能用 flex(1f)（理由见 [echoMuseSliderWidth]）
                width(echoMuseSliderWidth(ctx))
                height(28f)
                currentProgress(EchoMuseWakeWord.trackProgress(drafts.owwThreshold))
                progressColor(AdminColors.accent)
                trackColor(AdminColors.divider)
                thumbColor(AdminColors.accent)
            }
            event {
                progressDidChanged { p ->
                    drafts.owwThreshold = EchoMuseWakeWord.quantizeTrack(p)
                }
            }
        }
        Text {
            attr {
                marginLeft(AdminSpace.sm)
                width(48f)
                fontSize(AdminType.caption)
                color(AdminColors.textSecondary)
                text((kotlin.math.round(drafts.owwThreshold * 1000) / 1000.0).toString())
            }
        }
    }
    View {
        attr {
            flexDirectionRow()
            marginTop(AdminSpace.xs)
        }
        Text {
            attr {
                fontSize(AdminType.caption)
                color(AdminColors.textTertiary)
                text("Precise（严格）")
            }
        }
        View { attr { flex(1f) } }
        Text {
            attr {
                fontSize(AdminType.caption)
                color(AdminColors.textTertiary)
                text("Eager（积极）")
            }
        }
    }
    textNote(
        "官方语义：唤醒所需置信度——普通对话会误唤醒就向 Precise 调高，很少响应就向 Eager 调低。" +
            "上限 0.975 而非 1.0：openWakeWord 分数低于 1.0 即饱和、判定为 score≥阈值，1.0 意味着" +
            "永不唤醒（官方实测 18,021 帧峰值 0.999）。",
        micro = true,
    )

    switchRow("唤醒提示音", drafts.wakeSound) { drafts.wakeSound = !drafts.wakeSound }
    textNote("听到唤醒词时播放短促上升提示音（官方 wake sound）。", micro = true)
    // 官方行为：提示音档位只在开关打开时显示
    vif({ drafts.wakeSound }) {
        dropdownField(
            "提示音档位",
            { drafts.wakeLevels.firstOrNull { it.first == drafts.wakeSoundLevel }?.second ?: "未知" },
            { drafts.wakeLevels }, { drafts.wakeSoundLevel }, { drafts.wakeLevelOpen },
            { drafts.wakeLevelOpen = !drafts.wakeLevelOpen }, { drafts.wakeSoundLevel = it },
        )
    }
    labeledIntField("麦克风编号（0=中心麦克风，0～6）", { drafts.wakeMic }, { drafts.wakeMic = it }, "0")

    groupLabel("打断播报（官方把 barge-in 放在 wakeword 组）")
    switchRow("说话打断播报（barge-in）", drafts.bargeInEnabled) { drafts.bargeInEnabled = !drafts.bargeInEnabled }
    labeledDecimalField("打断阈值（0～1，越高越难打断）", { drafts.bargeInThresholdDraft }, { drafts.bargeInThresholdDraft = it }, EM_KEEP)
}

// ============================================================
// Stage 03 · 麦克风（官方 microphones）
// ============================================================

/**
 * 麦克风与 AEC：波束成形、角度、模拟/数字/处理增益、AEC 四个键、AGC。
 *
 * 官方的「麦克风预设卡 + 7 麦阵列示意图」在 Kuikly 里画不出来（NodeTestPanel 之外
 * 没有 canvas 能力），故改为基础控件——这也是本 Stage 与官方视觉差异最大的一处。
 */
internal fun ViewContainer<*, *>.stageMicrophones(state: EchoMuseState) {
    val drafts = state.drafts
    stageHead(
        "03 · 麦克风（官方 microphones）",
        "官方这里是一张麦克风预设卡 + 7 麦阵列示意图（Kuikly 无 canvas，画不了），所以本组" +
            "用基础控件表达同一组键；官方的 nsAsr / saveUtterances 设备没有对应键，不渲染。",
    )
    switchRow("波束成形（多麦定向）", drafts.beamformingEnabled) { drafts.beamformingEnabled = !drafts.beamformingEnabled }
    labeledDecimalField(
        "波束角度（-1=自动追踪，0～360）",
        { drafts.beamAngleDraft }, { drafts.beamAngleDraft = it }, EM_KEEP,
        help = "排障口径：波束基线校准不出来导致 mic 静音时，关掉波束成形退回单麦可一步验证" +
            "「是不是波束把信号吃掉了」。",
    )
    groupLabel("采集增益（模拟前端 → 数字 → 处理）")
    labeledIntField("模拟前端增益（0～127）", { drafts.adcMicpgaDraft }, { drafts.adcMicpgaDraft = it }, EM_KEEP)
    labeledIntField("数字增益（0～127）", { drafts.adcDigitalGainDraft }, { drafts.adcDigitalGainDraft = it }, EM_KEEP)
    labeledIntField("处理增益（dB，0～42）", { drafts.micGainDbDraft }, { drafts.micGainDbDraft = it }, EM_KEEP)
    groupLabel("回声消除与增益控制")
    switchRow("回声消除（AEC）", drafts.aecEnabled) { drafts.aecEnabled = !drafts.aecEnabled }
    switchRow("自动增益控制（AGC）", drafts.agcEnabled) { drafts.agcEnabled = !drafts.agcEnabled }
    dropdownField(
        "AEC 参考来源",
        { drafts.aecRefSources.firstOrNull { it.first == drafts.aecRefSource }?.second ?: "未知" },
        { drafts.aecRefSources }, { drafts.aecRefSource }, { drafts.aecRefSourceOpen },
        { drafts.aecRefSourceOpen = !drafts.aecRefSourceOpen }, { drafts.aecRefSource = it },
        help = "auto 自动检测硬件环回；AEC 仍残留回声时钉死 hw/sw 做 A/B（硬件参考下延时键可省）。",
    )
    labeledIntField("AEC 参考延时（毫秒，0～1000）", { drafts.aecDelayMsDraft }, { drafts.aecDelayMsDraft = it }, EM_KEEP)
    labeledIntField("AEC 尾长（毫秒，0～500）", { drafts.aecTailMsDraft }, { drafts.aecTailMsDraft = it }, EM_KEEP)
}

// ============================================================
// Stage 04 · 灯环（官方 ring）
// ============================================================

/**
 * 灯环：只渲染 `remoteVolumeArc`；`listeningAnim` 一期**只读展示**（P2 才出独立控件，
 * 见设计 §6.3），要改走 Stage 05 末尾的「进阶 JSON」手填入口。
 *
 * 官方的 `ledScene`/`ledListenColor`/`ledThinkColor`/`meter*` 全部是**控制器侧渲染**的
 * 参数（浏览器里画灯环用），设备固件不接受，本服务端也不下发——Stage 头必须写明，
 * 否则用户会以为"灯环怎么只有两个开关"。
 */
internal fun ViewContainer<*, *>.stageRing(state: EchoMuseState) {
    val drafts = state.drafts
    stageHead(
        "04 · 灯环（官方 ring）",
        "设备固件只接受 listeningAnim（JSON），其余灯环参数（ledScene / ledListenColor / " +
            "ledThinkColor / meter*）是官方控制器侧的渲染参数，本服务端不下发。故本组只有两个开关" +
            "位：遥控音量弧 + 灯环动画（一期只读）。",
    )
    switchRow("遥控调音量时灯环青色弧", drafts.remoteVolumeArc) { drafts.remoteVolumeArc = !drafts.remoteVolumeArc }
    infoRow(
        { "灯环动画（只读）" },
        { drafts.listeningAnimStored.ifEmpty { "设备未上报（用固件默认动画）" } },
    )
    textNote(
        "聆听灯环动画一期只读展示：要改请在「05 · 进阶」的 JSON 里手填 listeningAnim " +
            "（{\"pattern\":\"spin\",\"colors\":[[0,180,0]],\"ttlSec\":30}），同样过白名单与形状校验。",
        micro = true,
    )
}

// ============================================================
// Stage 05 · 进阶（官方 advanced）
// ============================================================

/**
 * 进阶：语音结束判定三键 + 进阶 JSON 手填入口。
 *
 * 「为什么比官方少」要写清楚三件事：
 * - `buttonSingleTapEvent`/`buttonMultiTapMs`：设备没有这两个键；
 * - `controllerEndpoints`：我方由 mDNS 自动下发、只读，不给改（改错会失联）；
 * - `consolePassword`/`consoleTimeoutMin`：P2 才补白名单，且指的是**USB 串口 console**，
 *   不是本页的网页控制台（设备端 `console` 是 tty，不是登录密码）。
 *
 * AGC 已在本表 Stage 03 渲染（同一键画两个开关会出现视觉不同步），这里只在说明里点名。
 */
internal fun ViewContainer<*, *>.stageAdvanced(state: EchoMuseState) {
    val drafts = state.drafts
    stageHead(
        "05 · 进阶（官方 advanced）",
        "比官方少四行：buttonSingleTapEvent / buttonMultiTapMs（设备无此键）、controllerEndpoints" +
            "（我方由 mDNS 自动下发、只读）、consolePassword / consoleTimeoutMin（P2 才补白名单，" +
            "且指设备 USB 串口 console，不是本页的网页控制台）。AGC 已在「03 · 麦克风」渲染，不重复。",
    )
    labeledDecimalField(
        "VAD 门限（0～1，越小越灵敏）",
        { drafts.vadThresholdDraft }, { drafts.vadThresholdDraft = it }, EM_KEEP,
        help = "语音能量超过该门限才算开口；现场对话误触发就调高，说话不响应就调低（设备默认 0.004）。",
    )
    labeledIntField("开口判定时长（毫秒，0～60000）", { drafts.vadSpeechMsDraft }, { drafts.vadSpeechMsDraft = it }, EM_KEEP)
    labeledIntField("收尾静音时长（毫秒，0～60000）", { drafts.vadSilenceMsDraft }, { drafts.vadSilenceMsDraft = it }, EM_KEEP)
    textNote(
        "官方 VAD 时序：开口持续超过「开口判定时长」才记为说话；静音持续超过「收尾静音时长」" +
            "就结束一轮。调太短会截断句尾，太长则回复迟钝。",
        micro = true,
    )
    labeledTextArea(
        "进阶 JSON（整对象手填）",
        { drafts.extraJsonDraft }, { drafts.extraJsonDraft = it },
        "{\"listeningAnim\":{\"pattern\":\"spin\"}}",
        height = 96f,
        help = "这里填的键与上面的控件合并后一起下发；键名仍由服务端白名单裁定（不在 40 键里的会被 400 拒绝）。" +
            EM_KEEP + "。",
    )
}

// ============================================================
// Stage 06 · 蓝牙（官方 bluetooth）
// ============================================================

/**
 * 蓝牙：`bleProxyEnabled`/`bleProxyConnections` 两个开关 + 整套通用 GATT 工具。
 *
 * 两个开关由 [echoMuseBlePanel] 里的 `bleSettings` 渲染（同一份 `[EchoMuseBleState]`，
 * 不在这里再画一遍，否则同一键有两个开关会视觉不同步）。官方还有 `BleProxyKey`（配对密钥），
 * 我方没有配对能力，不渲染。
 */
internal fun ViewContainer<*, *>.stageBluetooth(row: EchoMuseDeviceRow, state: EchoMuseState, ctx: Pager) {
    stageHead(
        "06 · 蓝牙（官方 bluetooth）",
        "只渲染 bleProxyEnabled / bleProxyConnections 两个键（在下面的代理设置里），官方还有" +
            "BleProxyKey（配对密钥），我方没有配对能力，不渲染。",
    )
    echoMuseBlePanel(row, state, ctx)
}

// ============================================================
// Stage 07 · Sendspin（官方 sendspin）
// ============================================================

/**
 * Sendspin（多房间音频同步）：`sendspinEnabled` + `sendspinName` + `sendspinUnpaired`。
 *
 * 设计 §6.3 表里 `sendspinUnpaired` 原写"→ P2 补白名单"，该行 **2026-10 起作废**：
 * 白名单已 40 键含它（T02 裁决）。三键都要显式渲染——少画一个，用户就没有解除配对的入口。
 */
internal fun ViewContainer<*, *>.stageSendspin(state: EchoMuseState) {
    val drafts = state.drafts
    stageHead(
        "07 · Sendspin（官方 sendspin）",
        "三个键全部在 40 键白名单内（含 sendspinUnpaired，2026-10 起）。配对本身要在设备上发起，" +
            "这里只能改名与解除既有配对。",
    )
    switchRow("启用 Sendspin（多房间同步）", drafts.sendspinEnabled) { drafts.sendspinEnabled = !drafts.sendspinEnabled }
    labeledField(
        "房间/设备名（广播给其它 Sendspin 节点）",
        { drafts.sendspinName }, { drafts.sendspinName = it },
        "如 客厅",
        help = "留空 = 用设备号。同名两个节点在客户端里无法区分。",
    )
    switchRow("解除既有配对", drafts.sendspinUnpaired) { drafts.sendspinUnpaired = !drafts.sendspinUnpaired }
    textNote(
        "打开「解除既有配对」并下发后，设备会忘掉已配对的同步关系；再同步需要在设备上重新发起配对。",
        micro = true,
    )
}

// ============================================================
// Stage 08 · 音乐（本服务端扩展，官方无此分组）
// ============================================================

/** Stage 08 头 + 内容：内容全在 `EchoMuseMusicPanel.kt`（本服务端扩展的 DLNA 播放面）。 */
internal fun ViewContainer<*, *>.stageMusic(row: EchoMuseDeviceRow, state: EchoMuseState, toast: ToastState, ctx: Pager) {
    stageHead(
        "08 · 音乐（本服务端扩展，官方 dashboard 无此分组）",
        "官方 dashboard 没有这一组：音乐面是本服务端的扩展（DLNA 拉流 + 与语音同平面混音）。" +
            "它不写任何 config 键，因此不受「下发配置」影响。",
    )
    echoMuseMusicStage(row, state, toast, ctx)
}
