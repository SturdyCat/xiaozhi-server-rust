package com.xiaozhi.admin

import com.xiaozhi.admin.ui.*

import com.tencent.kuikly.core.base.ViewContainer
import com.tencent.kuikly.core.pager.Pager

/**
 * 「音频调优」面板（设备卡菜单 [ECHOMUSE_MENU_AUDIO]，EchoMuseDeviceCard 分发）。
 *
 * 对齐官方 `ConfigMessage`（`device/internal/config/config.go`）的**设备侧采集/输出键**，
 * 分组移植官方音频链口径：语音结束判定 → 采集增益 → 回声消除与降噪 → 波束与打断 →
 * 播报与音量交互 → 输出音效链（ack 声明 `output_chain` 后设备在 ALSA 写入处自跑
 * EQ/低音保护/限幅）→ 背景音乐压低 → JSON 进阶键。
 *
 * 部分更新语义（与固件"指针类型区分未设置/设为零"一致）：**数值与 JSON 字段留空 =
 * 不发该键**（不把设备默认值写死成显式设置）；开关与枚举恒发（有明确视觉状态）。
 * 保存走 `POST …/settings` 的既有白名单校验，服务端仍是权威（202 = 已排队下发）。
 *
 * 草稿填充与保存逻辑见 [EchoMuseState.fillAudioDraft] / [EchoMuseState.saveAudioSettings]。
 */
internal fun ViewContainer<*, *>.echoMuseAudioPanel(
    row: EchoMuseDeviceRow,
    state: EchoMuseState,
    toast: ToastState,
    ctx: Pager,
) {
    // ---- 语音结束判定 ----
    groupLabel("语音结束判定（多久算说完）")
    labeledDecimalField(
        "VAD 门限（0～1，越小越灵敏）",
        { state.vadThresholdDraft }, { state.vadThresholdDraft = it },
        "留空不修改",
        help = "语音能量超过该门限才算开口；现场对话误触发就调高，说话不响应就调低（设备默认 0.004）。",
    )
    labeledIntField("开口判定时长（毫秒，0～60000）", { state.vadSpeechMsDraft }, { state.vadSpeechMsDraft = it }, "留空不修改")
    labeledIntField("收尾静音时长（毫秒，0～60000）", { state.vadSilenceMsDraft }, { state.vadSilenceMsDraft = it }, "留空不修改")
    textNote(
        "官方 VAD 时序：开口持续超过「开口判定时长」才记为说话；静音持续超过「收尾静音时长」" +
            "就结束一轮。调太短会截断句尾，太长则回复迟钝。",
        micro = true,
    )

    // ---- 采集增益 ----
    groupLabel("采集增益（模拟前端 → 数字 → 处理）")
    labeledIntField("模拟前端增益（0～127）", { state.adcMicpgaDraft }, { state.adcMicpgaDraft = it }, "留空不修改")
    labeledIntField("数字增益（0～127）", { state.adcDigitalGainDraft }, { state.adcDigitalGainDraft = it }, "留空不修改")
    labeledIntField("处理增益（dB，0～42）", { state.micGainDbDraft }, { state.micGainDbDraft = it }, "留空不修改")
    labeledIntField("开机音量（0～127）", { state.startupVolumeDraft }, { state.startupVolumeDraft = it }, "留空不修改")

    // ---- 回声消除与降噪 ----
    groupLabel("回声消除与增益控制")
    switchRow("回声消除（AEC）", state.aecEnabled) { state.aecEnabled = !state.aecEnabled }
    switchRow("自动增益控制（AGC）", state.agcEnabled) { state.agcEnabled = !state.agcEnabled }
    dropdownField(
        "AEC 参考来源",
        { state.aecRefSources.firstOrNull { it.first == state.aecRefSource }?.second ?: "未知" },
        { state.aecRefSources }, { state.aecRefSource }, { state.aecRefSourceOpen },
        { state.aecRefSourceOpen = !state.aecRefSourceOpen }, { state.aecRefSource = it },
        help = "auto 自动检测硬件环回；AEC 仍残留回声时钉死 hw/sw 做 A/B（硬件参考下延时键可省）。",
    )
    labeledIntField("AEC 参考延时（毫秒，0～1000）", { state.aecDelayMsDraft }, { state.aecDelayMsDraft = it }, "留空不修改")
    labeledIntField("AEC 尾长（毫秒，0～500）", { state.aecTailMsDraft }, { state.aecTailMsDraft = it }, "留空不修改")

    // ---- 波束与打断 ----
    groupLabel("波束与打断")
    switchRow("波束成形（多麦定向）", state.beamformingEnabled) { state.beamformingEnabled = !state.beamformingEnabled }
    labeledDecimalField("波束角度（-1=自动追踪，0～360）", { state.beamAngleDraft }, { state.beamAngleDraft = it }, "留空不修改")
    textNote(
        "排障口径：波束基线校准不出来导致 mic 静音时，关掉波束成形退回单麦可一步验证" +
            "「是不是波束把信号吃掉了」。",
        micro = true,
    )
    switchRow("说话打断播报（barge-in）", state.bargeInEnabled) { state.bargeInEnabled = !state.bargeInEnabled }
    labeledDecimalField("打断阈值（0～1，越高越难打断）", { state.bargeInThresholdDraft }, { state.bargeInThresholdDraft = it }, "留空不修改")

    // ---- 播报与音量交互 ----
    groupLabel("播报与音量交互")
    dropdownField(
        "语音相对音乐的提升电平",
        { state.responseLevels.firstOrNull { it.first == state.responseLevel }?.second ?: "未知" },
        { state.responseLevels }, { state.responseLevel }, { state.responseLevelOpen },
        { state.responseLevelOpen = !state.responseLevelOpen }, { state.responseLevel = it },
        help = "官方 responseLevel：低/中/高 = 0/+6/+12 dB，播报比背景音乐更响。",
    )
    switchRow("音量键提示音", state.volumeButtonSound) { state.volumeButtonSound = !state.volumeButtonSound }
    switchRow("遥控调音量时灯环青色弧", state.remoteVolumeArc) { state.remoteVolumeArc = !state.remoteVolumeArc }

    // ---- 输出音效链 ----
    groupLabel("输出音效链（设备本地 DSP）")
    textNote(
        "ack 已向设备声明 output_chain：EQ/低音保护/限幅由设备在 ALSA 写入处直接执行" +
            "（零重采样、零延迟），这里的开关与阈值随 config 下发。",
        micro = true,
    )
    switchRow("等响度（EQ Loudness）", state.eqLoudness) { state.eqLoudness = !state.eqLoudness }
    switchRow("低音保护", state.bassGuardEnabled) { state.bassGuardEnabled = !state.bassGuardEnabled }
    labeledDecimalField("低音保护阈值（dB，-60～24）", { state.bassGuardDbDraft }, { state.bassGuardDbDraft = it }, "留空不修改")
    switchRow("输出限幅", state.limiterEnabled) { state.limiterEnabled = !state.limiterEnabled }
    labeledDecimalField("限幅阈值（dB，-60～0）", { state.limiterThresholdDraft }, { state.limiterThresholdDraft = it }, "留空不修改")
    labeledDecimalField("限幅释放时间（毫秒，10～2000）", { state.limiterReleaseDraft }, { state.limiterReleaseDraft = it }, "留空不修改")

    // ---- 背景音乐 ----
    groupLabel("背景音乐")
    labeledDecimalField("说话时音乐压低深度（dB，-60～0）", { state.duckDbDraft }, { state.duckDbDraft = it }, "留空不修改")
    textNote(
        "语音轮次开始时设备把背景音乐压低该深度（duck）、结束恢复；0 = 不压低。",
        micro = true,
    )

    // ---- JSON 进阶键 ----
    groupLabel("进阶（JSON，留空不修改）")
    labeledField(
        "EQ 频段（数字数组）",
        { state.eqBandsDraft }, { state.eqBandsDraft = it },
        "[2,0,-1,0,2]",
        help = "按频段顺序的增益 dB 值数组；服务端校验为非空数字数组。",
    )
    labeledTextArea(
        "聆听灯环动画 spec",
        { state.listeningAnimDraft }, { state.listeningAnimDraft = it },
        "{\"pattern\":\"spin\",\"colors\":[[0,180,0]],\"ttlSec\":30}",
        help = "私听态默认动画（设备本地渲染）：pattern/colors/periodMs/ttlSec/listening；ttlSec 是死信开关。",
    )

    actionRow {
        primaryButton(
            "保存音频设置",
            loading = { state.actionBusy },
            loadingText = "保存中…",
            icon = IconToken.Save,
        ) {
            state.saveAudioSettings(ctx, row.deviceId) { ok, msg ->
                toast.show(msg, if (ok) "ok" else "error")
            }
        }
    }
    textNote(
        "设备在线时设置即时下发；离线时保存到服务端，设备下次接入自动生效（结果看设备灯环/听感）。",
        micro = true,
    )
}
