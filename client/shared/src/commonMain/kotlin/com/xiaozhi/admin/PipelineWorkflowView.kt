package com.xiaozhi.admin

import androidx.compose.runtime.Composable
import com.gearui.components.button.Button
import com.gearui.components.button.ButtonShape
import com.gearui.components.button.ButtonSize
import com.gearui.components.button.ButtonType
import com.gearui.foundation.layout.Spacing
import com.gearui.foundation.primitives.Text
import com.gearui.theme.Theme
import com.tencent.kuikly.compose.foundation.background
import com.tencent.kuikly.compose.foundation.lazy.LazyRow
import com.tencent.kuikly.compose.foundation.layout.*
import com.tencent.kuikly.compose.ui.Alignment
import com.tencent.kuikly.compose.ui.Modifier
import com.tencent.kuikly.compose.ui.unit.dp
import com.xiaozhi.admin.contracts.IconToken
import com.xiaozhi.admin.icons.GearIcon
import com.xiaozhi.admin.kit.statusBadge

/** One horizontal axis, including the selectable inter-stage hooks. Never a vertical branch list. */
@Composable
fun PipelineWorkflowHeader(
    form: ConfigFormState,
    modifier: Modifier = Modifier,
    active: Boolean = true,
    onSelect: () -> Unit = {},
) {
    val pager = LocalAdminPagerOrThrow()
    val available = (adminContentWidth(pager) - Spacing.lg.value * 2).coerceAtLeast(0f)
    val sideWidth = 84f
    val linkWidth = 44f
    val minimumWidth = sideWidth * 2 + linkWidth * 4 + 100f * 5
    val coreWidth = ((available - sideWidth * 2 - linkWidth * 4) / 5).coerceAtLeast(100f)
    Column(modifier.fillMaxWidth().background(Theme.colors.surface).padding(vertical = Spacing.sm)) {
        Row(Modifier.fillMaxWidth().padding(horizontal = Spacing.lg), verticalAlignment = Alignment.CenterVertically) {
            Text("语音流程", style = Theme.typography.label, color = Theme.colors.foreground)
            Box(Modifier.weight(1f))
            Button(text = "全局设置", onClick = { form.openConfigSection("server"); onSelect() }, type = ButtonType.TEXT, size = ButtonSize.SMALL,
                iconWidget = { color -> GearIcon(token = IconToken.Settings, size = 16.dp, tint = color) })
        }
        if (available < minimumWidth) {
            LazyRow(modifier = Modifier.fillMaxWidth().height(82.dp), contentPadding = PaddingValues(horizontal = Spacing.lg)) {
                for (position in PipelineWorkflow.positions) {
                    item(key = position.id) {
                        WorkflowAxisItem(form, position, active, if (position.isCore) coreWidth else if (position.id in setOf("start", "end")) sideWidth else linkWidth, onSelect)
                    }
                }
            }
        } else {
            Row(Modifier.fillMaxWidth().height(82.dp).padding(horizontal = Spacing.lg), verticalAlignment = Alignment.CenterVertically) {
                for (position in PipelineWorkflow.positions) {
                    WorkflowAxisItem(form, position, active, if (position.isCore) coreWidth else if (position.id in setOf("start", "end")) sideWidth else linkWidth, onSelect)
                }
            }
        }
        if (form.workflow.metadata?.mode == "aiui") {
            Text("当前为云端托管流程，本地主节点并非实际执行记录。", Modifier.padding(horizontal = Spacing.lg), style = Theme.typography.caption, color = Theme.colors.warning)
        }
    }
}

@Composable
private fun WorkflowAxisItem(form: ConfigFormState, position: WorkflowPosition, active: Boolean, width: Float, onSelect: () -> Unit) {
    val selected = active && form.workflowPosition?.id == position.id
    val endpoint = position.id in setOf("start", "end")
    Column(Modifier.width(width.dp).height(64.dp).padding(horizontal = 2.dp), horizontalAlignment = Alignment.CenterHorizontally) {
        if (position.isCore || endpoint) {
            Button(
                text = if (position.id == "start") "接入" else position.label,
                onClick = { form.selectWorkflowPosition(position.id); onSelect() },
                type = if (selected) ButtonType.FILL else ButtonType.OUTLINE,
                size = ButtonSize.SMALL,
                modifier = Modifier.fillMaxWidth(),
                contentDescription = "打开${position.label}节点",
            )
        } else {
            Button(
                onClick = { form.selectWorkflowPosition(position.id); onSelect() },
                type = if (selected) ButtonType.FILL else ButtonType.TEXT,
                size = ButtonSize.SMALL, shape = ButtonShape.CIRCLE,
                contentDescription = "打开${position.label}衔接插件",
                iconWidget = { GearIcon(if (form.showAdvanced) IconToken.Plus else IconToken.ChevronRight, size = 16.dp,
                    tint = if (selected) Theme.colors.primaryForeground else Theme.colors.mutedForeground) },
            )
        }
        val caption = when {
            position.isCore -> position.nodes.firstOrNull()?.label?.substringAfter(" · ").orEmpty()
            endpoint -> if (position.id == "start") "设备入口" else "记忆收尾"
            else -> "衔接"
        }
        Text(caption, style = Theme.typography.caption, color = Theme.colors.mutedForeground, maxLines = 1)
    }
}

/** Kept for native host layout calculations; the header itself remains responsive. */
internal const val WORKFLOW_HEADER_HEIGHT = 121f

@Composable
internal fun StageStatusBadge(phase: String) {
    when (phase) {
        "failed" -> statusBadge(state = "error", text = "异常")
        "degraded" -> statusBadge(state = "busy", text = "降级")
        "active" -> Unit
        "disabled" -> statusBadge(state = "idle", text = "已关闭")
        else -> if (phase.isNotEmpty()) statusBadge(state = "idle", text = phase)
    }
}
internal val StageIconSize = 18.dp
