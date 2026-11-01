package com.xiaozhi.admin

import androidx.compose.runtime.*
import com.gearui.components.button.Button
import com.gearui.components.button.ButtonType
import com.gearui.components.tabs.Tab
import com.gearui.components.tabs.Tabs
import com.gearui.foundation.primitives.ScrollView
import com.gearui.foundation.primitives.Text
import com.gearui.overlay.*
import com.gearui.primitives.composite.Card
import com.gearui.theme.Theme
import com.tencent.kuikly.compose.foundation.layout.*
import com.tencent.kuikly.compose.ui.Alignment
import com.tencent.kuikly.compose.ui.Modifier
import com.tencent.kuikly.compose.ui.zIndex
import com.tencent.kuikly.compose.ui.unit.dp
import com.xiaozhi.admin.kit.LocalAdminSpacing
import com.xiaozhi.admin.kit.LocalAdminViewport
import com.xiaozhi.admin.kit.LocalAdminPager
import com.xiaozhi.admin.kit.LocalFieldHelp

/** 单一 Kuikly 管理工作区；沿用官方分区顺序与布局，不再嵌入英文官方文档。 */
internal val EM_MANAGEMENT_TABS = listOf(
    Tab("status", "状态"), Tab("activity", "活动"), Tab("config", "配置"),
    Tab("shell", "控制台"), Tab("updates", "更新"), Tab("logs", "日志"),
    Tab("music", "DLNA 音乐"), Tab("ble", "GATT 工具"), Tab("assets", "VAD 资产"),
    Tab("models", "模型文件"), Tab("wifi", "WiFi 确认"), Tab("diagnostics", "原始诊断"),
)

/** 管理工作区复用 GearUI modal overlay；显式尺寸不受普通 Dialog 的窄屏宽度上限限制。 */
@Composable
internal fun EchoMuseManagementDialog(
    row: EchoMuseDeviceRow?,
    state: EchoMuseState,
    toast: ToastState,
    apiBase: String,
    token: String,
    onDismiss: () -> Unit,
    modelFiles: @Composable () -> Unit,
) {
    val overlay = LocalOverlayController.current
    val currentRow = rememberUpdatedState(row)
    val currentDismiss = rememberUpdatedState(onDismiss)
    val currentFiles = rememberUpdatedState(modelFiles)
    val pager = LocalAdminPager.current
    val viewport = rememberUpdatedState(LocalAdminViewport.current)
    val spacing = rememberUpdatedState(LocalAdminSpacing.current)
    val fieldHelp = LocalFieldHelp.current
    val id = row?.deviceId
    DisposableEffect(id, apiBase, token) {
        val overlayId = if (id != null) overlay.show(
            options = OverlayOptions(
                placement = OverlayPlacement.Center,
                modal = true,
                maskColor = OverlayDefaults.scrimColor,
                dismissPolicy = OverlayDismissPolicy.Modal.copy(outsideClick = false),
            ),
            onDismiss = { currentDismiss.value() },
        ) {
            // null 时立即卸载文档，不等 overlay 退出动画，防止隐藏控制台占设备 shell。
            currentRow.value?.let { selected ->
                CompositionLocalProvider(
                    LocalAdminPager provides pager,
                    LocalAdminViewport provides viewport.value,
                    LocalAdminSpacing provides spacing.value,
                    LocalFieldHelp provides fieldHelp,
                ) {
                    EchoMuseManagementContent(
                        row = selected, state = state, toast = toast, apiBase = apiBase, token = token,
                        onDismiss = { currentDismiss.value() }, modelFiles = { currentFiles.value() },
                    )
                }
            }
        } else null
        onDispose {
            state.detail.leave()
            overlayId?.let { overlay.close(it) }
        }
    }
}

@Composable
private fun EchoMuseManagementContent(
    row: EchoMuseDeviceRow,
    state: EchoMuseState,
    toast: ToastState,
    apiBase: String,
    token: String,
    onDismiss: () -> Unit,
    modelFiles: @Composable () -> Unit,
) {
    var tab by remember(row.deviceId, apiBase, token) { mutableStateOf("status") }
    var headerInputBlocked by remember(row.deviceId) { mutableStateOf(false) }
    val viewport = LocalAdminViewport.current
    val spacing = LocalAdminSpacing.current
    val pager = LocalAdminPagerOrThrow()
    val label = if (tab == "assets") EM_TAB_UPDATES else EM_MANAGEMENT_TABS.first { it.id == tab }.label
    DisposableEffect(row.deviceId, apiBase, token, tab, row.approved) {
        if (row.approved) state.enterTab(row, label, pager)
        onDispose { state.detail.pauseTab() }
    }
    SideEffect { state.detail.sync(listOf(row)) }
    Card(modifier = Modifier.width((viewport.width.value - 32f).coerceIn(1f, 1200f).dp)
        .height((viewport.height.value - 32f).coerceIn(1f, 900f).dp)) {
        Column(modifier = Modifier.fillMaxSize().zIndex(1f).padding(spacing.md)) {
            EchoMuseDeviceHeader(row = row, state = state, toast = toast, onDismiss = onDismiss,
                onInputBlocked = { headerInputBlocked = it })
            Tabs(
                items = EM_MANAGEMENT_TABS, selectedId = tab, onSelect = { tab = it },
                scrollable = true, modifier = Modifier.fillMaxWidth(),
            )
            when {
                !row.approved -> EchoMuseApprovalTab(row, state, toast, Modifier.weight(1f).fillMaxWidth())
                tab == "shell" -> EchoMuseConsoleTab(row, state, toast, apiBase, token,
                    inputBlocked = headerInputBlocked, modifier = Modifier.weight(1f).fillMaxWidth())
                tab == "config" -> EchoMuseNativeConfigTab(row, state, toast, Modifier.weight(1f).fillMaxWidth())
                tab == "updates" -> EchoMuseNativeUpdatesTab(row, state, toast, Modifier.weight(1f).fillMaxWidth())
                tab == "logs" -> EchoMuseNativeLogsTab(row, state, toast, Modifier.weight(1f).fillMaxWidth())
                tab == "status" || tab == "activity" -> ScrollView(modifier = Modifier.weight(1f).fillMaxWidth()) {
                    Column(Modifier.fillMaxWidth().padding(top = spacing.md)) {
                        if (tab == "status") EchoMuseNativeStatusTab(row, state, toast)
                        else EchoMuseNativeActivityTab(row, state, toast)
                    }
                }
                tab == "models" -> Column(modifier = Modifier.weight(1f).fillMaxWidth()) { modelFiles() }
                else -> ScrollView(modifier = Modifier.weight(1f).fillMaxWidth()) {
                    EchoMuseExtensionContent(row, state, toast, tab)
                }
            }
        }
    }
}
