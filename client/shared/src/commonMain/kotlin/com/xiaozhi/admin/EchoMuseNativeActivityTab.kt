package com.xiaozhi.admin

import androidx.compose.runtime.Composable
import com.gearui.components.empty.EmptyState
import com.tencent.kuikly.compose.foundation.layout.Column
import com.tencent.kuikly.compose.foundation.layout.fillMaxWidth
import com.tencent.kuikly.compose.ui.Modifier

/** Official voice-activity panel only. Raw telemetry belongs to the separate diagnostics tools. */
@Composable
internal fun EchoMuseNativeActivityTab(
    row: EchoMuseDeviceRow,
    state: EchoMuseState,
    toast: ToastState,
    modifier: Modifier = Modifier,
) {
    val model = row.settings?.text("owwModel").orEmpty().ifEmpty { "默认模型" }
    val threshold = row.settings?.opt("owwThreshold")?.toString().orEmpty()
    Column(modifier.fillMaxWidth()) {
        EchoMuseNativePanel("语音活动 · $model${if (threshold.isNotEmpty()) " @ $threshold" else ""}") {
            EmptyState(message = "语音轮次记录暂不可用",
                description = "当前服务端尚未提供轮次历史、录音回放与质量统计接口。",
                modifier = Modifier.fillMaxWidth())
        }
    }
}
