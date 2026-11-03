package com.xiaozhi.admin

import androidx.compose.runtime.*
import com.gearui.components.button.Button
import com.gearui.components.button.ButtonType
import com.gearui.foundation.primitives.Text
import com.gearui.primitives.composite.Card
import com.gearui.theme.Theme
import com.tencent.kuikly.compose.foundation.background
import com.tencent.kuikly.compose.foundation.clickable
import com.tencent.kuikly.compose.foundation.layout.*
import com.tencent.kuikly.compose.ui.Alignment
import com.tencent.kuikly.compose.ui.Modifier
import com.tencent.kuikly.compose.ui.unit.dp
import com.xiaozhi.admin.contracts.IconToken
import com.xiaozhi.admin.icons.GearIcon
import com.xiaozhi.admin.kit.LocalAdminSpacing
import com.xiaozhi.admin.kit.labeledField
import com.xiaozhi.admin.kit.labeledSecretField
import com.xiaozhi.admin.kit.labeledSelect

/**
 * 涂鸦设备零件库：凭证卡（设备管理中心「齿轮」弹层用）+ 只读说明行 + 设备卡（中心列表用）。
 *
 * 原「涂鸦设备」独立页面已并入 [DeviceManagementCenter]（2026-11-03 导航合并：统一卡片列表
 * + 齿轮一次性配置）；本文件只保留跨处复用的零件，页面级编排不再这里。
 * 状态全部来自 [TuyaState]（stateVar/stateListOf）；secret 输入保存成功后由状态层立即清空
 * （服务端也永不回显），界面上只见「已配置（掩码）」。
 */

/**
 * 凭证卡：状态行（已配置 + 掩码 / 未配置）+ 区域/ID/secret 表单 + 保存/清除。
 * secret 保存成功后由状态层清空草稿；这里只渲染「已配置」与掩码，不出现原值。
 */
@Composable
internal fun TuyaCredentialsCard(
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
            .padding(vertical = spacing.xs),
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
        labeledSecretField(
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

/**
 * 单张涂鸦设备卡：视觉对齐 [EchoMuseDeviceCard]（头部信息条 → 图标 → 状态条），
 * 来源标识「涂鸦」放头部右侧（卡片在中心的分区标签下，徽标只做卡片级溯源）。
 */
@Composable
internal fun TuyaDeviceCard(row: TuyaDeviceRow, modifier: Modifier, onOpen: () -> Unit) {
    val spacing = LocalAdminSpacing.current
    Card(modifier = modifier.clickable(onClick = onOpen)) {
        Column(modifier = Modifier.fillMaxWidth()) {
            Column(
                modifier = Modifier.fillMaxWidth().background(Theme.colors.muted)
                    .padding(horizontal = spacing.md, vertical = spacing.sm),
            ) {
                Row(
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(spacing.xs),
                ) {
                    Text(
                        text = row.name.ifEmpty { row.deviceId },
                        style = Theme.typography.titleSmall,
                        color = Theme.colors.foreground,
                        maxLines = 1,
                        modifier = Modifier.weight(1f),
                    )
                    Text(text = "涂鸦", style = Theme.typography.caption, color = Theme.colors.mutedForeground)
                }
                Text(text = row.displayModel, maxLines = 1, style = Theme.typography.caption,
                    color = Theme.colors.mutedForeground)
            }
            Box(modifier = Modifier.fillMaxWidth().padding(vertical = spacing.lg), contentAlignment = Alignment.Center) {
                GearIcon(
                    token = IconToken.Wifi,
                    size = 64.dp,
                    tint = if (row.online) Theme.colors.success else Theme.colors.mutedForeground,
                )
            }
            Column(modifier = Modifier.fillMaxWidth().padding(horizontal = spacing.md).padding(bottom = spacing.md)) {
                Column(
                    modifier = Modifier.fillMaxWidth().background(Theme.colors.muted).padding(spacing.sm),
                    horizontalAlignment = Alignment.CenterHorizontally,
                ) {
                    Text(
                        text = if (row.online) "在线" else "离线",
                        style = Theme.typography.bodyMedium,
                        color = if (row.online) Theme.colors.success else Theme.colors.mutedForeground,
                    )
                    Text(
                        text = row.ip.ifEmpty { "IP 未上报" },
                        style = Theme.typography.caption,
                        color = Theme.colors.mutedForeground,
                        maxLines = 1,
                    )
                }
            }
        }
    }
}

/** 只读说明行（浅色主题 caption；设备中心与详情弹窗共用）。 */
@Composable
internal fun TuyaNote(text: String) {
    Text(
        text = text,
        style = Theme.typography.caption,
        color = Theme.colors.mutedForeground,
    )
}
