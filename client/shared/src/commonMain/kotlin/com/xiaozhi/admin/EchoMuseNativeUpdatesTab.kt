package com.xiaozhi.admin

import androidx.compose.runtime.Composable
import com.gearui.foundation.primitives.ScrollView
import com.tencent.kuikly.compose.foundation.layout.Column
import com.tencent.kuikly.compose.foundation.layout.fillMaxWidth
import com.tencent.kuikly.compose.ui.Modifier

/** Official panel order: firmware, emOS, wake-word assets. No duplicate assets tab. */
@Composable
fun EchoMuseNativeUpdatesTab(
    row: EchoMuseDeviceRow,
    state: EchoMuseState,
    toast: ToastState,
    modifier: Modifier = Modifier,
) {
    ScrollView(modifier = modifier.fillMaxWidth()) {
        Column(modifier = Modifier.fillMaxWidth()) {
            LabeledCard(title = "固件") {
                EmInfoRow("当前版本", row.version.ifEmpty { EM_DASH })
                EmInfoRow("可用版本", EM_DASH)
                EmNote("当前服务端未提供设备固件发布与升级接口。")
                if (!row.online) EmWarn("设备离线，显示最近上报版本。")
            }
            if (row.baseOs == "emos") LabeledCard(title = "emOS") {
                EmInfoRow("当前版本", EM_DASH)
                EmNote("当前服务端未提供 emOS 升级接口。")
            }
            LabeledCard(title = "唤醒词与语音资产") { EmAssets(state, toast) }
        }
    }
}
