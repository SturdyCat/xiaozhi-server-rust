package com.xiaozhi.admin

import com.tencent.kuikly.core.base.Color
import com.tencent.kuikly.core.base.ViewContainer
/** Common UI always references the canonical SVG through the dedicated vector view. */
fun IconToken.assetName(): String = when (this) {
    IconToken.Save -> "save"
    IconToken.Delete -> "delete"
    IconToken.Menu -> "menu"
    IconToken.ChevronDown -> "chevron-down"
    IconToken.ChevronUp -> "chevron-up"
    IconToken.ChevronRight -> "chevron-right"
    IconToken.Play -> "play"
    IconToken.Stop -> "stop"
    IconToken.Refresh -> "refresh"
    IconToken.Check -> "check"
    IconToken.Close -> "close"
    IconToken.Settings -> "settings"
    IconToken.Microphone -> "microphone"
    IconToken.Waveform -> "waveform"
    IconToken.Text -> "text"
    IconToken.Intent -> "intent"
    IconToken.Model -> "model"
    IconToken.Speaker -> "speaker"
    IconToken.Info -> "info"
}

fun IconToken.svgSource(): String = "assets://common/icons/source/${assetName()}.svg"

/** Pure vector rendering; parent controls own actions and accessible names. */
fun ViewContainer<*, *>.appIcon(
    token: () -> IconToken,
    size: Float = 20f,
    tint: () -> Color = { AdminColors.textSecondary },
) {
    Svg {
        attr {
            size(size, size)
            src(token().svgSource())
            tintColor(tint())
            touchEnable(false)
        }
    }
}
