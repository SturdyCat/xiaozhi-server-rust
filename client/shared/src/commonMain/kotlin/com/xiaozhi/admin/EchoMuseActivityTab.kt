package com.xiaozhi.admin

import androidx.compose.runtime.Composable
import com.xiaozhi.admin.kit.GearUiSpace
import com.xiaozhi.admin.kit.splitRow

/**
 * 活动 tab：六个统计位（**2 个真实 / 4 个 `—`**，见设计 §6.5 的"修订过的诚实版"）。
 *
 * 为什么只有 2 个是真的：`listen_state`（聆听状态）与 `playback_stats.underruns`
 * （断音次数）是设备**确实上行**的两项；而轮次数、成功率、回复中位耗时、近失手
 * （near-miss）四项需要**服务端轮次打点**——那部分还没做（二期），所以一期如实显示
 * `—` 加副标，而不是编一个 0 或画一条假曲线。
 *
 * 文本口径与 PRD 一致：宁可承认"未采集"，也不让运维拿假数据下判断
 * （"成功率高"与"没测过"长得一样是这类页面的头号误读来源）。
 */

/** 「未采集」的统一副标（四处复用，改一处即全改）。 */
private const val EM_ACTIVITY_NOT_COLLECTED = "未采集（需服务端轮次打点，二期）"

@Composable
internal fun EchoMuseActivityTab(state: EchoMuseState) {
    val detail = state.detail

    EmGroupLabel("聆听与播放（设备上行实况）", topGap = GearUiSpace.sm)
    if (detail.runtime == null) {
        EmNote("尚未读取运行状态：进本 tab 会自动读一次，也可以到「状态」tab 手动刷新。")
    } else {
        splitRow(divider = false, spacing = GearUiSpace.md) {
            grow {
                EmStatTile(
                    label = "State（聆听状态）",
                    value = when (detail.runtime?.optJSONObject("listen_state")?.text("state")) {
                        "local" -> "local（设备本地唤醒词判定）"
                        "stream" -> "stream（上行流式判定）"
                        "degraded" -> "degraded（退化：走 RMS 门限）"
                        else -> EM_DASH
                    },
                    hint = listenReasonHint(detail),
                )
            }
            grow {
                EmStatTile(
                    label = "Underruns（断音）",
                    value = emNumber(detail.runtime?.optJSONObject("playback_stats"), "underruns")
                        ?.toLong()?.toString() ?: EM_DASH,
                    hint = "最近一次播报流的设备侧断音计数",
                )
            }
        }
    }

    EmGroupLabel("轮次统计")
    splitRow(divider = false, spacing = GearUiSpace.md) {
        grow { EmStatTile("Turns（最近 50 轮）", EM_DASH, EM_ACTIVITY_NOT_COLLECTED) }
        grow { EmStatTile("成功率", EM_DASH, EM_ACTIVITY_NOT_COLLECTED) }
        grow { EmStatTile("回复中位耗时", EM_DASH, EM_ACTIVITY_NOT_COLLECTED) }
    }
    splitRow(divider = false, spacing = GearUiSpace.md) {
        grow { EmStatTile("Near-misses（近失手）", EM_DASH, EM_ACTIVITY_NOT_COLLECTED) }
        grow { EmStatTile("—", EM_DASH, "本格无对应指标（保持与上一行同宽）") }
        grow { EmStatTile("—", EM_DASH, "本格无对应指标（保持与上一行同宽）") }
    }
    EmNote(
        "还没有轮次记录——轮次统计需服务端打点，二期提供。" +
            "现在能在「状态」tab 看到的是设备上报的即时状态（音量/静音/资源/链路），不是轮次历史。",
    )
}

/** State 块的副标：`listen_state.reason` 是设备给的降级/关闭原因，有就照原文透出。 */
private fun listenReasonHint(detail: EchoMuseDetailState): String {
    val reason = detail.runtime?.optJSONObject("listen_state")?.text("reason").orEmpty()
    return when {
        reason.isNotEmpty() -> "原因：$reason"
        detail.runtime?.optJSONObject("listen_state") == null -> "设备未上报"
        else -> "设备上报的当前聆听模式"
    }
}
