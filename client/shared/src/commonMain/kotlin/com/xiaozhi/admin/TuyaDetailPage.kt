package com.xiaozhi.admin

import androidx.compose.runtime.*
import com.gearui.components.alert.Alert
import com.gearui.components.alert.AlertStatus
import com.gearui.components.button.Button
import com.gearui.components.button.ButtonType
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
import com.xiaozhi.admin.kit.INT_INPUT_FILTER
import com.xiaozhi.admin.kit.LocalAdminSpacing
import com.xiaozhi.admin.kit.labeledField

/**
 * 涂鸦设备**详情视图**（列表页选中一台后替换内容区，返回即回列表）：
 * 设备基本信息 + DP 实时状态（可写项行内控制：布尔 → 开关按钮，数值 → 行内输入）。
 *
 * 控制失败的服务端三归类（offline/timeout/unsupported + 建议动作）原样展示——
 * 客户端不自己编降级文案（与 OpsState「直接展示服务端原话」同一口径）。
 */
@Composable
fun TuyaDetailPage(state: TuyaState, toast: ToastState) {
    val pager = LocalAdminPagerOrThrow()
    val spacing = LocalAdminSpacing.current

    Column(modifier = Modifier.fillMaxSize()) {
        Row(
            modifier = Modifier.fillMaxWidth().padding(spacing.pagePadding),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(spacing.sm),
        ) {
            Text(
                text = state.detailName.ifEmpty { state.detailId },
                modifier = Modifier.weight(1f),
                style = Theme.typography.titleMedium,
                color = Theme.colors.foreground,
            )
            Button(text = "返回列表", type = ButtonType.OUTLINE, onClick = { state.closeDetail() })
        }
        ScrollView(modifier = Modifier.weight(1f).fillMaxWidth()) {
            Column(
                modifier = Modifier.fillMaxWidth().padding(horizontal = spacing.pagePadding),
                verticalArrangement = Arrangement.spacedBy(spacing.gutter),
            ) {
                if (state.detailError.isNotEmpty()) {
                    Alert(
                        title = "详情读取失败",
                        description = state.detailError,
                        status = AlertStatus.DANGER,
                        modifier = Modifier.fillMaxWidth(),
                    )
                }
                TuyaDeviceInfoCard(state)
                TuyaAliasCard(state, pager, toast)
                if (state.dpMessage.isNotEmpty()) {
                    TuyaNote(state.dpMessage)
                }
                TuyaDpCard(state, pager, toast)
            }
        }
    }
}

/** 设备基本信息卡：型号 / 在线状态 / device_id（local_key 服务端永不下发，页面天然不可见）。 */
@Composable
private fun TuyaDeviceInfoCard(state: TuyaState) {
    val spacing = LocalAdminSpacing.current
    Column(
        modifier = Modifier
            .fillMaxWidth()
            .clip(RoundedCornerShape(12.dp))
            .background(Theme.colors.surface)
            .padding(16.dp),
        verticalArrangement = Arrangement.spacedBy(spacing.xs),
    ) {
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(6.dp)) {
            Box(
                modifier = Modifier
                    .size(8.dp)
                    .background(
                        if (state.detailOnline) Theme.colors.primary else Theme.colors.mutedForeground,
                        RoundedCornerShape(4.dp),
                    ),
            )
            Text(
                text = if (state.detailOnline) "在线" else "离线",
                style = Theme.typography.bodyMedium,
                color = Theme.colors.foreground,
            )
        }
        TuyaNote("型号：${state.detailModel.ifEmpty { "未知" }}")
        TuyaNote("device_id：${state.detailId}")
        if (state.detailLoading) TuyaNote("DP 状态读取中…")
    }
}

/** 本地别名卡（P1-4：只影响本管理台展示名，不影响云端名称）。 */
@Composable
private fun TuyaAliasCard(state: TuyaState, pager: com.tencent.kuikly.core.pager.Pager, toast: ToastState) {
    labeledField(
        label = "本地别名（仅本管理台展示，不影响云端名称）",
        value = state.aliasDraft,
        onValueChange = { state.aliasDraft = it },
        placeholder = "留空 = 用设备原始名称",
        trailing = {
            Button(
                text = "保存别名",
                type = ButtonType.OUTLINE,
                disabled = state.dpBusy,
                onClick = {
                    state.saveAlias(pager) { ok, msg -> toast.show(msg, if (ok) "ok" else "error") }
                },
            )
        },
    )
}

/** DP 实时状态卡：可写项行内控制（布尔开关 / 数值行内输入），其余只读展示。 */
@Composable
private fun TuyaDpCard(state: TuyaState, pager: com.tencent.kuikly.core.pager.Pager, toast: ToastState) {
    val spacing = LocalAdminSpacing.current
    Column(
        modifier = Modifier
            .fillMaxWidth()
            .clip(RoundedCornerShape(12.dp))
            .background(Theme.colors.surface)
            .padding(16.dp),
        verticalArrangement = Arrangement.spacedBy(spacing.sm),
    ) {
        Text(
            text = "数据点（DP）实时状态",
            style = Theme.typography.titleSmall,
            color = Theme.colors.foreground,
        )
        if (!state.detailLoading && state.dps.isEmpty()) {
            TuyaNote("暂无 DP 快照：设备上线后由服务端轮询/控制回读填充（每 60s 错峰轮询）。")
        }
        for (dp in state.dps) {
            TuyaDpRowView(state, dp, pager, toast)
        }
    }
}

/** 单条 DP 行：只读展示 + 可写控件（switch / number）。 */
@Composable
private fun TuyaDpRowView(
    state: TuyaState,
    dp: TuyaDpRow,
    pager: com.tencent.kuikly.core.pager.Pager,
    toast: ToastState,
) {
    val spacing = LocalAdminSpacing.current
    Row(
        modifier = Modifier.fillMaxWidth(),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(spacing.sm),
    ) {
        Column(modifier = Modifier.weight(1f)) {
            Text(
                text = "DP ${dp.dp} · ${dp.valueText.ifEmpty { "（空）" }}",
                style = Theme.typography.bodyMedium,
                color = Theme.colors.foreground,
            )
            Text(
                text = if (dp.writable) "可控制" else "只读（控制成功一次后显示可控制）",
                style = Theme.typography.caption,
                color = Theme.colors.mutedForeground,
            )
        }
        when {
            // 布尔 DP：开关按钮（当前开 → 显示"关闭"）
            dp.control == "switch" -> Button(
                text = if (dp.valueText == "开") "关闭" else "开启",
                type = ButtonType.FILL,
                disabled = state.dpBusy,
                onClick = {
                    val next = if (dp.valueText == "开") "false" else "true"
                    state.setDp(pager, dp.dp, next) { ok, msg -> toast.show(msg, if (ok) "ok" else "error") }
                },
            )
            // 数值 DP：点"调整"展开行内输入（共用一个草稿，一次改一个点）
            dp.control == "number" && state.numberDp != dp.dp -> Button(
                text = "调整",
                type = ButtonType.OUTLINE,
                disabled = state.dpBusy,
                onClick = {
                    state.numberDp = dp.dp
                    state.numberDraft = dp.rawValue
                },
            )
            else -> {}
        }
    }
    // 数值行内输入（展开态）：整型过滤 + 设置/取消
    if (dp.control == "number" && state.numberDp == dp.dp) {
        val spacing2 = LocalAdminSpacing.current
        Row(
            modifier = Modifier.fillMaxWidth(),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(spacing2.sm),
        ) {
            labeledIntFieldInline(
                value = state.numberDraft,
                onValueChange = { state.numberDraft = it },
                modifier = Modifier.weight(1f),
            )
            Button(
                text = if (state.dpBusy) "下发中…" else "设置",
                type = ButtonType.FILL,
                loading = state.dpBusy,
                disabled = state.dpBusy,
                onClick = {
                    state.setDp(pager, dp.dp, state.numberDraft.trim()) { ok, msg ->
                        toast.show(msg, if (ok) "ok" else "error")
                    }
                },
            )
            Button(text = "取消", type = ButtonType.OUTLINE, onClick = { state.numberDp = -1 })
        }
    }
}

/** 数值行内输入（labeledField 的紧凑变体：整型过滤，无标签行——标签在上一行 DP 概要里）。 */
@Composable
private fun labeledIntFieldInline(value: String, onValueChange: (String) -> Unit, modifier: Modifier) {
    labeledField(
        label = "目标值",
        value = value,
        onValueChange = onValueChange,
        modifier = modifier,
        inputFilter = INT_INPUT_FILTER,
    )
}
