package com.xiaozhi.admin

import androidx.compose.runtime.*
import com.gearui.foundation.primitives.Text
import com.gearui.overlay.*
import com.gearui.primitives.composite.Card
import com.gearui.theme.Theme
import com.tencent.kuikly.compose.foundation.layout.*
import com.tencent.kuikly.compose.ui.Alignment
import com.tencent.kuikly.compose.ui.Modifier
import com.tencent.kuikly.compose.ui.unit.dp
import com.xiaozhi.admin.contracts.IconToken
import com.xiaozhi.admin.kit.LocalAdminPager
import com.xiaozhi.admin.kit.LocalAdminSpacing
import com.xiaozhi.admin.kit.LocalAdminViewport
import com.xiaozhi.admin.kit.LocalFieldHelp

/**
 * 涂鸦设备详情弹窗：GearUI 居中 modal overlay + 头部（名称 + 关闭），内容区复用
 * [TuyaDeviceDetailContent]（设备信息 / 本地别名 / DP 控制）。
 *
 * 与 EchoMuse 管理弹窗同一套宿主模式：遮罩外点击不关闭；Pager/viewport/spacing/fieldHelp
 * 显式穿过 overlay 门户。进入弹窗即拉 DP 快照（[TuyaState.openDetail]），关闭清理
 * （[TuyaState.closeDetail]）。日志/测试/调试等官方 EchoMuse 才有的面板这里**不伪造**
 * ——涂鸦侧未提供的能力不出现。
 */
@Composable
internal fun TuyaDeviceDialog(
    row: TuyaDeviceRow?,
    state: TuyaState,
    toast: ToastState,
    onDismiss: () -> Unit,
) {
    val overlay = LocalOverlayController.current
    val currentRow = rememberUpdatedState(row)
    val currentDismiss = rememberUpdatedState(onDismiss)
    val pager = LocalAdminPagerOrThrow()
    val viewport = rememberUpdatedState(LocalAdminViewport.current)
    val spacing = rememberUpdatedState(LocalAdminSpacing.current)
    val fieldHelp = LocalFieldHelp.current
    val id = row?.deviceId
    DisposableEffect(id) {
        // null 时立即卸载内容，不等 overlay 退出动画，避免迟到的 DP 回包写进已关状态
        val overlayId = if (id != null) overlay.show(
            options = OverlayOptions(
                placement = OverlayPlacement.Center,
                modal = true,
                maskColor = OverlayDefaults.scrimColor,
                dismissPolicy = OverlayDismissPolicy.Modal.copy(outsideClick = false),
            ),
            onDismiss = { currentDismiss.value() },
        ) {
            currentRow.value?.let { selected ->
                CompositionLocalProvider(
                    LocalAdminPager provides pager,
                    LocalAdminViewport provides viewport.value,
                    LocalAdminSpacing provides spacing.value,
                    LocalFieldHelp provides fieldHelp,
                ) {
                    TuyaDeviceDialogContent(selected, state, toast, onDismiss = { currentDismiss.value() })
                }
            }
        } else null
        // 进入弹窗即拉详情 + DP 快照（行信息先垫底，回包对号后覆盖）
        row?.let { state.openDetail(pager, it) }
        onDispose {
            state.closeDetail()
            overlayId?.let { overlay.close(it) }
        }
    }
}

@Composable
private fun TuyaDeviceDialogContent(
    row: TuyaDeviceRow,
    state: TuyaState,
    toast: ToastState,
    onDismiss: () -> Unit,
) {
    val viewport = LocalAdminViewport.current
    val spacing = LocalAdminSpacing.current
    Card(
        modifier = Modifier
            .width((viewport.width.value * 0.95f).coerceIn(1f, 700f).dp)
            .height((viewport.height.value * 0.90f).coerceIn(1f, 620f).dp),
    ) {
        Column(modifier = Modifier.fillMaxSize().padding(spacing.md), verticalArrangement = Arrangement.spacedBy(spacing.sm)) {
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(spacing.sm)) {
                Column(modifier = Modifier.weight(1f)) {
                    Text(
                        text = row.name.ifEmpty { row.deviceId },
                        style = Theme.typography.titleMedium,
                        color = Theme.colors.foreground,
                        maxLines = 1,
                    )
                    Text(
                        text = row.displayModel,
                        style = Theme.typography.caption,
                        color = Theme.colors.mutedForeground,
                        maxLines = 1,
                    )
                }
                EchoMuseActionButton(icon = IconToken.Close, contentDescription = "关闭设备详情", onClick = onDismiss)
            }
            TuyaDeviceDetailContent(state, toast)
        }
    }
}
