package com.xiaozhi.admin

import androidx.compose.runtime.Composable
import com.gearui.components.button.Button
import com.gearui.components.button.ButtonType
import com.gearui.foundation.primitives.ScrollView
import com.gearui.foundation.scroll.rememberScrollState
import com.tencent.kuikly.compose.foundation.layout.Column
import com.tencent.kuikly.compose.foundation.layout.fillMaxWidth
import com.tencent.kuikly.compose.ui.Modifier
import com.tencent.kuikly.compose.ui.unit.dp
import com.xiaozhi.admin.contracts.IconToken
import com.xiaozhi.admin.icons.GearIcon
import com.xiaozhi.admin.kit.actionRow

/**
 * Native update view.  The controller currently exposes only read-only device
 * versions and the OWW asset status; it does not expose the official OTA
 * release/deploy endpoints, so this view deliberately does not invent buttons.
 * [EchoMuseState.enterTab] owns the read/30-second polling lifecycle.
 */
@Composable
fun EchoMuseNativeUpdatesTab(
    row: EchoMuseDeviceRow,
    state: EchoMuseState,
    toast: ToastState,
    modifier: Modifier = Modifier,
) {
    val detail = state.detail
    val pager = LocalAdminPagerOrThrow()
    ScrollView(modifier = modifier.fillMaxWidth(), state = rememberScrollState()) {
        Column(modifier = Modifier.fillMaxWidth()) {
        LabeledCard(title = "固件") {
            EmInfoRow("设备版本", row.version.ifEmpty { EM_DASH })
            EmInfoRow("可用版本", "未知（当前服务端未提供 OTA 发布查询）")
            EmNote("此页面只展示设备列表上报的真实版本；当前设备升级接口未提供，固件包托管请前往全局配置 → OTA。")
            if (!row.online) EmWarn("设备当前离线，版本只代表最近一次列表快照。")
        }
        if (row.baseOs == "emos") {
            LabeledCard(title = "emOS") {
                EmInfoRow("运行系统", "emOS")
                EmInfoRow("系统版本", "未知（设备列表未上报 emOS 版本）")
                EmNote("emOS 独立更新包/本地 payload 接口未在当前服务端开放，页面不提供伪可用更新按钮。")
            }
        }
        LabeledCard(title = "OWW 资产") {
            if (!detail.owwLoaded) {
                EmNote(if (detail.owwLoading) "正在读取设备侧资产清单…" else "尚未读取资产状态。")
            } else {
                val vad = detail.owwState?.optJSONObject("vad")?.text("state").orEmpty()
                val ort = detail.owwState?.optJSONObject("ort")?.text("state").orEmpty()
                EmInfoRow("VAD 模型", emAssetTextNative(vad))
                EmInfoRow("推理运行时", emAssetTextNative(ort))
                val free = (detail.owwState?.opt("free_mb") as? Number)?.toLong()
                EmInfoRow("设备剩余空间", free?.let { "$it MB" } ?: EM_DASH)
                if (vad == "missing" || vad == "outdated") {
                    EmWarn("VAD 缺失或版本不一致；请切换到 VAD 资产面板执行资产安装。此处只读取更新状态。")
                }
            }
            actionRow {
                Button(
                    text = "刷新资产状态",
                    onClick = { detail.readOwwState(pager) },
                    type = ButtonType.OUTLINE,
                    loading = detail.owwLoading,
                    disabled = !(detail.canSystem && !detail.owwLoading),
                    iconWidget = { GearIcon(token = IconToken.Refresh, size = 16.dp) },
                )
            }
            if (!detail.canSystem) EmWarn("资产状态读取需要已批准、在线且运行 emOS 的设备。")
            if (detail.error.isNotEmpty()) EmWarn(detail.error, error = true)
        }
        }
    }
}

private fun emAssetTextNative(state: String): String = when (state) {
    "installed" -> "已安装"
    "outdated" -> "版本不一致"
    "blocked" -> "空间不足"
    "missing" -> "缺失"
    else -> "未知"
}

