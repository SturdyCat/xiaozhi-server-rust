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
import com.tencent.kuikly.compose.foundation.background
import com.tencent.kuikly.compose.foundation.layout.*
import com.tencent.kuikly.compose.foundation.shape.RoundedCornerShape
import com.tencent.kuikly.compose.ui.Alignment
import com.tencent.kuikly.compose.ui.Modifier
import com.tencent.kuikly.compose.ui.draw.clip
import com.tencent.kuikly.compose.ui.unit.dp
import com.xiaozhi.admin.kit.GearUiBreakpoints
import com.xiaozhi.admin.kit.LocalAdminSpacing
import com.xiaozhi.admin.kit.LocalAdminViewport
import com.xiaozhi.admin.kit.labeledField
import com.xiaozhi.admin.kit.labeledSelect

/**
 * 「涂鸦设备」列表页：凭证状态条 + 立即同步 + 设备卡片栅格 → 点击进详情。
 *
 * 布局参照 `EchoMusePage`（GearUI 浅色主题 + 卡片栅格断点换算），状态全部来自
 * [TuyaState]（stateVar/stateListOf）；凭证表单的 secret 输入在保存成功后由状态层
 * 立即清空（服务端也永不回显），页面上只见"已配置（掩码）"。
 */
@Composable
fun TuyaPage(
    state: TuyaState,
    toast: ToastState,
    isActive: () -> Boolean,
) {
    val pager = LocalAdminPagerOrThrow()
    val spacing = LocalAdminSpacing.current
    val active = isActive()
    var confirmClear by remember { mutableStateOf(false) }

    if (active && state.detailId.isNotEmpty()) {
        // 详情视图（单开一台设备）：返回按钮 + 设备信息 + DP 控制
        TuyaDetailPage(state = state, toast = toast)
        return
    }

    Column(modifier = Modifier.fillMaxSize()) {
        // ---- 头部：摘要 + 立即同步 ----
        Row(
            modifier = Modifier.fillMaxWidth().padding(spacing.pagePadding),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(spacing.sm),
        ) {
            Text(
                text = if (state.loading && !state.loaded) "加载中…" else state.summaryText(),
                modifier = Modifier.weight(1f),
                style = Theme.typography.titleMedium,
                color = Theme.colors.foreground,
            )
            Button(
                text = if (state.syncing) "同步中…" else "立即同步",
                type = ButtonType.OUTLINE,
                loading = state.syncing,
                disabled = state.syncing || !state.enabled,
                onClick = {
                    state.syncNow(pager) { ok, msg -> toast.show(msg, if (ok) "ok" else "error") }
                },
            )
        }

        ScrollView(modifier = Modifier.weight(1f).fillMaxWidth()) {
            Column(
                modifier = Modifier.fillMaxWidth().padding(horizontal = spacing.pagePadding),
                verticalArrangement = Arrangement.spacedBy(spacing.gutter),
            ) {
                if (state.loadError.isNotEmpty()) {
                    Alert(
                        title = "加载失败",
                        description = state.loadError,
                        status = AlertStatus.DANGER,
                        modifier = Modifier.fillMaxWidth(),
                    )
                }
                if (!state.enabled) {
                    TuyaNote("涂鸦设备未启用。请到「全局配置 → Tuya」开启 [tuya].enabled（保存即生效，无需重启）。")
                }
                if (state.enabled) {
                    TuyaCredentialsCard(
                        state = state,
                        confirmClear = confirmClear,
                        onConfirmClearChange = { confirmClear = it },
                        onDone = { ok, msg -> toast.show(msg, if (ok) "ok" else "error") },
                    )
                    if (state.loaded && state.devices.isEmpty()) {
                        EmptyState(
                            message = "还没有涂鸦设备",
                            description = "录入凭证后点「立即同步」拉取账号下的设备目录；设备列表与控制全部走服务端局域网直连。",
                            actionText = "立即同步",
                            onAction = {
                                state.syncNow(pager) { ok, msg -> toast.show(msg, if (ok) "ok" else "error") }
                            },
                            modifier = Modifier.fillMaxWidth(),
                        )
                    }
                    TuyaDeviceGrid(state = state)
                    if (state.storePath.isNotEmpty()) {
                        Text(
                            text = "设备目录存储：${state.storePath}",
                            style = Theme.typography.caption,
                            color = Theme.colors.mutedForeground,
                        )
                    }
                }
            }
        }
    }
}

/** 设备卡片栅格：按可用宽度自适应列数（与 EchoMusePage 同一换算口径）。 */
@Composable
private fun TuyaDeviceGrid(state: TuyaState) {
    val pager = LocalAdminPagerOrThrow()
    val spacing = LocalAdminSpacing.current
    val viewport = LocalAdminViewport.current
    val contentWidth = viewport.width.value -
        (if (viewport.isNarrow) 0f else GearUiBreakpoints.SIDEBAR_WIDTH + 1f) - spacing.pagePadding.value * 2
    val columns = ((contentWidth + spacing.gutter.value) / (260f + spacing.gutter.value)).toInt().coerceAtLeast(1)
    val cardWidth = ((contentWidth - spacing.gutter.value * (columns - 1)) / columns).coerceAtLeast(1f)
    for (group in state.devices.chunked(columns)) {
        Row(
            modifier = Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.spacedBy(spacing.gutter),
        ) {
            for (row in group) {
                TuyaDeviceCard(
                    row = row,
                    modifier = Modifier.width(cardWidth.dp),
                    onOpen = { state.openDetail(pager, row) },
                )
            }
        }
    }
}

/** 单张设备卡片：名称 / 型号 / 在线状态点 / IP（点击进详情）。 */
@Composable
private fun TuyaDeviceCard(row: TuyaDeviceRow, modifier: Modifier, onOpen: () -> Unit) {
    Column(
        modifier = modifier
            .clip(RoundedCornerShape(12.dp))
            .background(Theme.colors.surface)
            .padding(16.dp),
        verticalArrangement = Arrangement.spacedBy(4.dp),
    ) {
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(6.dp)) {
            Box(
                modifier = Modifier
                    .size(8.dp)
                    .background(
                        if (row.online) Theme.colors.primary else Theme.colors.mutedForeground,
                        RoundedCornerShape(4.dp),
                    ),
            )
            Text(
                text = row.name.ifEmpty { row.deviceId },
                style = Theme.typography.titleSmall,
                color = Theme.colors.foreground,
                modifier = Modifier.weight(1f),
            )
        }
        Text(
            text = row.displayModel,
            style = Theme.typography.caption,
            color = Theme.colors.mutedForeground,
        )
        if (row.ip.isNotEmpty()) {
            Text(
                text = "${row.ip} · ${if (row.online) "在线" else "离线"}",
                style = Theme.typography.caption,
                color = Theme.colors.mutedForeground,
            )
        }
        Button(text = "查看详情", type = ButtonType.OUTLINE, onClick = onOpen)
    }
}

/**
 * 凭证卡：状态行（已配置 + 掩码 / 未配置）+ 区域/ID/secret 表单 + 保存/清除。
 * secret 保存成功后由状态层清空草稿；这里只渲染"已配置"与掩码，不出现原值。
 */
@Composable
private fun TuyaCredentialsCard(
    state: TuyaState,
    confirmClear: Boolean,
    onConfirmClearChange: (Boolean) -> Unit,
    onDone: (ok: Boolean, msg: String) -> Unit,
) {
    val pager = LocalAdminPagerOrThrow()
    val spacing = LocalAdminSpacing.current
    Column(
        modifier = Modifier
            .fillMaxWidth()
            .clip(RoundedCornerShape(12.dp))
            .background(Theme.colors.surface)
            .padding(16.dp),
        verticalArrangement = Arrangement.spacedBy(spacing.sm),
    ) {
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(spacing.sm)) {
            Text(
                text = "涂鸦云凭证",
                style = Theme.typography.titleSmall,
                color = Theme.colors.foreground,
                modifier = Modifier.weight(1f),
            )
            Text(
                text = if (state.configured) "已配置 · ${state.accessIdMasked} · ${state.region}" else "未配置",
                style = Theme.typography.caption,
                color = if (state.configured) Theme.colors.primary else Theme.colors.warning,
            )
        }
        Text(
            text = state.lastSyncText,
            style = Theme.typography.caption,
            color = Theme.colors.mutedForeground,
        )
        labeledSelect(
            label = "区域",
            selectedId = state.formRegion,
            options = listOf(
                "cn" to "中国（cn）",
                "us" to "美洲（us）",
                "eu" to "欧洲（eu）",
                "in" to "印度（in）",
            ),
            onSelect = { state.formRegion = it },
        )
        labeledField(
            label = "access_id",
            value = state.formAccessId,
            onValueChange = { state.formAccessId = it },
            placeholder = "涂鸦 IoT 平台云开发项目的 access_id",
        )
        if (state.configured) {
            Text(
                text = "access_secret 已配置（不再回显）。留空保存 = 保留旧值；更换请输入新值。",
                style = Theme.typography.caption,
                color = Theme.colors.mutedForeground,
            )
        }
        labeledField(
            label = if (state.configured) "access_secret（留空保留旧值）" else "access_secret",
            value = state.formAccessSecret,
            onValueChange = { state.formAccessSecret = it },
            placeholder = "涂鸦 IoT 平台云开发项目的 access_secret",
        )
        Row(horizontalArrangement = Arrangement.spacedBy(spacing.sm)) {
            Button(
                text = if (state.credentialsBusy) "保存中…" else "保存凭证",
                type = ButtonType.FILL,
                loading = state.credentialsBusy,
                disabled = state.credentialsBusy,
                onClick = { state.saveCredentials(pager, onDone) },
            )
            if (state.configured) {
                if (confirmClear) {
                    Button(
                        text = "确认清除？",
                        type = ButtonType.OUTLINE,
                        onClick = {
                            onConfirmClearChange(false)
                            state.clearCredentials(pager, onDone)
                        },
                    )
                    Button(text = "取消", type = ButtonType.OUTLINE, onClick = { onConfirmClearChange(false) })
                } else {
                    Button(text = "清除凭证", type = ButtonType.OUTLINE, onClick = { onConfirmClearChange(true) })
                }
            }
        }
    }
}

/** 只读说明行（浅色主题 caption；列表页与详情页共用）。 */
@Composable
internal fun TuyaNote(text: String) {
    Text(
        text = text,
        style = Theme.typography.caption,
        color = Theme.colors.mutedForeground,
    )
}
