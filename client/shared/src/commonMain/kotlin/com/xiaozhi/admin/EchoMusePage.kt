package com.xiaozhi.admin

import androidx.compose.runtime.*
import com.gearui.components.alert.Alert
import com.gearui.components.alert.AlertStatus
import com.gearui.components.button.Button
import com.gearui.components.button.ButtonType
import com.gearui.components.empty.EmptyState
import com.gearui.foundation.primitives.ScrollView
import com.gearui.foundation.primitives.Text
import com.gearui.theme.Theme
import com.tencent.kuikly.compose.foundation.layout.*
import com.tencent.kuikly.compose.ui.Alignment
import com.tencent.kuikly.compose.ui.Modifier
import com.xiaozhi.admin.kit.LocalAdminSpacing

/** 列表只显示 Kuikly 设备摘要；单设备管理统一进入双 Tab 弹窗。 */
@Composable
fun EchoMusePage(
    state: EchoMuseState,
    toast: ToastState,
    isActive: () -> Boolean,
    modelFiles: @Composable () -> Unit = {},
    apiBase: String = "",
    token: String = "",
) {
    val pager = LocalAdminPagerOrThrow()
    val active = isActive()
    var selectedId by remember(apiBase, token) { mutableStateOf<String?>(null) }
    val selected = state.devices.firstOrNull { it.deviceId == selectedId }
    DisposableEffect(state, active, apiBase, token) {
        state.ensureLiveUpdates(pager) { active }
        onDispose {
            state.ensureLiveUpdates(pager) { false }
            state.detail.leave()
        }
    }
    LaunchedEffect(selectedId, selected?.deviceId, state.loaded) {
        if (state.loaded && selectedId != null && selected == null) selectedId = null
    }

    Column(modifier = Modifier.fillMaxSize()) {
        Row(
            modifier = Modifier.fillMaxWidth().padding(LocalAdminSpacing.current.pagePadding),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(LocalAdminSpacing.current.sm),
        ) {
            Text(
                text = if (state.loading && !state.loaded) "加载中…" else state.summaryText(),
                modifier = Modifier.weight(1f), style = Theme.typography.titleMedium,
                color = Theme.colors.foreground,
            )
            Button(text = "刷新", onClick = { state.refresh(pager) }, type = ButtonType.OUTLINE,
                loading = state.loading, disabled = state.loading)
        }
        ScrollView(modifier = Modifier.weight(1f).fillMaxWidth()) {
            Column(modifier = Modifier.fillMaxWidth().padding(horizontal = LocalAdminSpacing.current.pagePadding)) {
                if (state.loadError.isNotEmpty()) {
                    Alert(title = "设备列表加载失败", description = state.loadError,
                        status = AlertStatus.DANGER, modifier = Modifier.fillMaxWidth())
                }
                if (state.loading && !state.loaded) EmNote("正在读取 EchoMuse 设备列表…")
                if (state.loaded && !state.enabled) {
                    EmNote("EchoMuse 未启用。请到「全局配置 → EchoMuse 接入」开启设备入口并重启服务端。")
                } else if (state.enabled) {
                    if (state.loaded && state.devices.isEmpty()) {
                        EmptyState(message = "还没有 EchoMuse 设备登记",
                            description = "设备首次连上服务端后会出现在这里，经批准后即可管理。",
                            actionText = "刷新", onAction = { state.refresh(pager) }, modifier = Modifier.fillMaxWidth())
                    }
                    for (row in state.devices) {
                        EchoMuseDeviceCard(row = row, onOpen = { selectedId = row.deviceId })
                    }
                }
            }
        }
    }
    EchoMuseManagementDialog(
        row = selected.takeIf { active && state.enabled }, state = state, toast = toast,
        apiBase = apiBase, token = token, onDismiss = { selectedId = null },
        modelFiles = modelFiles,
    )
}
