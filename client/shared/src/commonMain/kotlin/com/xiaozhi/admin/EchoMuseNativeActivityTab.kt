package com.xiaozhi.admin

import androidx.compose.runtime.Composable
import com.gearui.components.button.Button
import com.gearui.components.button.ButtonType
import com.tencent.kuikly.compose.foundation.layout.Arrangement
import com.tencent.kuikly.compose.foundation.layout.Column
import com.tencent.kuikly.compose.foundation.layout.fillMaxWidth
import com.tencent.kuikly.compose.ui.Modifier
import com.tencent.kuikly.compose.ui.unit.dp
import com.xiaozhi.admin.kit.LocalAdminSpacing
import com.xiaozhi.admin.kit.actionRow

/**
 * Device telemetry is a latest-value snapshot, NOT a voice-turn archive.
 * The former adapter's /turns route returned a synthetic empty array. Do not present that as zero turns.
 * No timer or subscription is started: modal entry loads runtime, manual refresh reads it again.
 */
@Composable
internal fun EchoMuseNativeActivityTab(
    row: EchoMuseDeviceRow,
    state: EchoMuseState,
    toast: ToastState,
    modifier: Modifier = Modifier,
) {
    val pager = LocalAdminPagerOrThrow()
    val detail = state.detail
    val bound = detail.target == row.deviceId
    val runtime = detail.runtime.takeIf { bound }
    Column(modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(LocalAdminSpacing.current.sm)) {
        EchoMuseNativeSnapshotHeader(row, state)
        actionRow {
            Button(text = "读取活动快照", type = ButtonType.OUTLINE,
                disabled = !bound || !row.approved || !detail.idle, loading = detail.busy == "runtime",
                onClick = { detail.readRuntime(pager) })
            Button(text = "复制活动快照", type = ButtonType.TEXT, disabled = runtime == null,
                onClick = {
                    val output = buildString {
                        append("设备：${row.deviceId}\n")
                        append(if (detail.runtimeCurrent) "当前会话最近上报，非实时\n" else "历史快照，不代表当前状态\n")
                        append("聆听状态：${echoMuseNativeListenText(runtime?.optJSONObject("listen_state")?.text("state").orEmpty())}\n")
                        listOf("listen_end" to "最近聆听结束", "playback_stats" to "最近播放报告", "oww_shadow_cross" to "最近唤醒词影子命中").forEach { (key, title) ->
                            append("$title：${runtime?.optJSONObject(key)?.toString() ?: "尚未上报"}\n")
                        }
                    }
                    toast.show(if (copyToClipboard(output)) "活动快照已复制" else "复制失败：剪贴板不可用")
                })
        }
        EchoMuseNativePanel("语音活动") {
            EmInfoRow("聆听模式", echoMuseNativeListenText(runtime?.optJSONObject("listen_state")?.text("state").orEmpty()))
            EmInfoRow("麦克风静音", when (runtime?.optJSONObject("mute_state")?.opt("muted")) { true -> "已静音"; false -> "未静音"; else -> "未知" })
            EmNote("这里只展示设备最近一次活动上报，不代表持续录音、正在思考或正在播放；聆听模式与语音轮次执行状态不同。")
        }
        EchoMuseNativePanel("轮次记录与质量统计") {
            EmNote("当前服务端不提供官方轮次历史、录音回放、近失唤醒次数、成功率或回复时延统计。无法读取不等于没有活动，不显示虚构的 0 次或成功率。")
        }
        EchoMuseNativeActivityReport("最近聆听结束", runtime?.optJSONObject("listen_end"),
            "设备最新一次聆听结束报告；不是完整事件列表。")
        EchoMuseNativeActivityReport("最近播放报告", runtime?.optJSONObject("playback_stats"),
            "设备侧最新一次播放统计原文；不等于当前回复已播放完成。")
        EchoMuseNativeActivityReport("最近唤醒词影子命中", runtime?.optJSONObject("oww_shadow_cross"),
            "影子评分只上报、不触发动作；不能据此推断一次实际唤醒或语音轮成功。")
        EchoMuseNativeOperationFeedback(state)
    }
}

@Composable
private fun EchoMuseNativeActivityReport(
    title: String,
    data: com.tencent.kuikly.core.nvi.serialization.json.JSONObject?,
    explanation: String,
) {
    EchoMuseNativePanel(title) {
        EmNote(explanation)
        if (data == null || data.length() == 0) EmNote("尚未上报；当前没有可展示的设备观测。")
        else EmOutputBlock(data.toString(), height = 120.dp)
    }
}
