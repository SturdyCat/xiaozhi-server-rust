package com.xiaozhi.admin

import androidx.compose.runtime.Composable
import com.gearui.components.button.Button
import com.gearui.components.button.ButtonType
import com.xiaozhi.admin.contracts.IconToken
import com.xiaozhi.admin.kit.actionRow

/** The one asset panel lives in official Updates. Unknown is not missing. */
@Composable
internal fun EmAssets(state: EchoMuseState, toast: ToastState) {
    val detail = state.detail
    val pager = LocalAdminPagerOrThrow()
    val vad = detail.owwState?.optJSONObject("vad")?.text("state").orEmpty()
    val ort = detail.owwState?.optJSONObject("ort")?.text("state").orEmpty()
    EmInfoRow("VAD 模型", emAssetText(vad))
    EmInfoRow("推理运行时", emAssetText(ort))
    val free = (detail.owwState?.opt("free_mb") as? Number)?.toLong()
    EmInfoRow("剩余空间", free?.let { "$it MB" } ?: EM_DASH)
    if (!detail.owwLoaded) EmNote(if (detail.owwLoading) "正在检查资产…" else "尚未检查资产。")
    actionRow {
        EchoMuseActionButton(icon = IconToken.Refresh, contentDescription = "刷新资产状态",
            loading = detail.owwLoading, disabled = !detail.canSystem || detail.owwLoading,
            onClick = { detail.readOwwState(pager) })
        Button(text = "安装 VAD", type = ButtonType.FILL,
            loading = detail.busy == "oww/install-vad",
            disabled = !detail.canSystem || !detail.idle || vad !in listOf("missing", "outdated"),
            onClick = {
                detail.reinstallVad(pager) { ok, message ->
                    toast.show(message, if (ok) "ok" else "error")
                    if (ok) detail.readOwwState(pager)
                }
            })
    }
    if (vad == "blocked" || ort == "blocked") EmWarn("设备空间不足，不能安装资产。")
    if (!detail.canSystem) EmNote("需要已批准、在线的 emOS 设备。")
    if (detail.error.isNotEmpty()) EmWarn(detail.error, error = true)
}

private fun emAssetText(state: String): String = when (state) {
    "installed" -> "已安装"
    "outdated" -> "可更新"
    "blocked" -> "空间不足"
    "missing" -> "未安装"
    else -> "未知"
}
