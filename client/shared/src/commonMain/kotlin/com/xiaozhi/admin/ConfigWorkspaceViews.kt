package com.xiaozhi.admin

import androidx.compose.runtime.Composable
import com.gearui.components.button.Button
import com.gearui.components.button.ButtonType
import com.gearui.components.cell.Cell
import com.gearui.components.cellgroup.CellGroup
import com.gearui.foundation.layout.Spacing
import com.gearui.foundation.primitives.Text
import com.gearui.theme.Theme
import com.tencent.kuikly.compose.foundation.layout.Box
import com.tencent.kuikly.compose.foundation.layout.Column
import com.tencent.kuikly.compose.foundation.layout.Row
import com.tencent.kuikly.compose.foundation.layout.fillMaxHeight
import com.tencent.kuikly.compose.foundation.layout.fillMaxWidth
import com.tencent.kuikly.compose.foundation.layout.padding
import com.tencent.kuikly.compose.foundation.layout.width
import com.tencent.kuikly.compose.ui.Alignment
import com.tencent.kuikly.compose.ui.Modifier
import com.tencent.kuikly.compose.ui.unit.Dp
import com.tencent.kuikly.compose.ui.unit.dp
import com.tencent.kuikly.core.pager.IPager
import com.xiaozhi.admin.contracts.IconToken
import com.xiaozhi.admin.icons.GearIcon
import com.xiaozhi.admin.kit.GearUiBreakpoints
import com.xiaozhi.admin.kit.LocalAdminViewport
import com.xiaozhi.admin.kit.dividerV
import com.xiaozhi.admin.kit.dot
import com.xiaozhi.admin.kit.pageScroller

/** 内容区宽度（扣掉常驻侧栏）；独立配置页无侧栏时取整窗宽度。 */
internal fun adminContentWidth(ctx: IPager): Float {
    val width = ctx.pageData.pageViewWidth
    val sidebar = if (ctx is AdminPage && width >= GearUiBreakpoints.NARROW) {
        GearUiBreakpoints.SIDEBAR_WIDTH + 1f
    } else {
        0f
    }
    return (width - sidebar).coerceAtLeast(0f)
}

/** 工作区是否紧凑（< 700：卡片内距降档、左菜单收窄到只留文字）。 */
internal fun workspaceIsCompact(ctx: IPager): Boolean =
    adminContentWidth(ctx) < GearUiBreakpoints.WORKSPACE_COMPACT

/** 左菜单宽度：紧凑 92dp、常规 192dp（旧 DSL 的同一口径，换成 Dp）。 */
@Composable
private fun workspaceMenuWidth(): Dp {
    val viewport = LocalAdminViewport.current
    return if (viewport.workspaceCompact) 92.dp else 192.dp
}

/** 一个可测试节点 + 它的一个动作，用来给测试菜单和内容区提供同一份数据。 */
internal data class WorkspaceTestEntry(val node: PipelineFlowNode, val descriptor: NodeTestDescriptor)

private fun workspaceTests(form: ConfigFormState): List<WorkspaceTestEntry> =
    form.workflowPosition?.nodes.orEmpty().flatMap { node ->
        nodeVisibleTests(node).map { WorkspaceTestEntry(node, it) }
    }

internal fun selectedWorkspaceTest(form: ConfigFormState): WorkspaceTestEntry? {
    val entries = workspaceTests(form)
    return entries.firstOrNull { it.node.id == form.selectedFlowNodeId && it.descriptor.id == form.selectedTestActionId }
        ?: entries.firstOrNull { it.node.id == form.selectedFlowNodeId }
        ?: entries.firstOrNull()
}

/**
 * 语音流程的**左菜单**（配置项目 / 测试项目两套条目）。
 *
 * 用 `CellGroup` + `Cell`：分组标题（「配置项目」/「测试项目」/节点名）+ 每行一个可点项。
 * 选中态由 **品牌色文字 + leading 的品牌色图标** 两个信号表达（`Cell` 没有 `selected` 参数，
 * 与侧栏同一套视觉语言）；能力 phase 偏离时行尾挂 5dp 告警点（K5）。
 *
 * 旧版用 `menuListItem(WORKSPACE 档)`——同一份 `IconToken` 图标、同一份选中语义，
 * 只是容器从自绘列表换成 `CellGroup`（分组标题、圆角、分隔线由组件负责）。
 */
@Composable
fun WorkspaceMenu(form: ConfigFormState, meta: PluginMetaState) {
    val position = form.workflowPosition
    val testing = form.workspaceTab == NodeWorkspaceTab.Testing
    Column(modifier = Modifier.fillMaxHeight().padding(vertical = Spacing.xs)) {
        Text(
            text = if (testing) "测试项目" else "配置项目",
            modifier = Modifier.padding(horizontal = Spacing.md, vertical = Spacing.xs),
            style = Theme.typography.caption,
            color = Theme.colors.mutedForeground,
        )
        if (testing) {
            val entries = workspaceTests(form)
            if (entries.isEmpty()) {
                MenuNote("暂无测试")
            } else {
                // 扩展位置的测试按节点分组（核心阶段不分组，直接列动作）
                for (node in position?.nodes.orEmpty()) {
                    val tests = entries.filter { it.node.id == node.id }
                    if (tests.isEmpty()) continue
                    val grouped = position?.isCore == false
                    CellGroup(
                        items = tests,
                        title = if (grouped) node.label else null,
                        key = { "${it.node.id}#${it.descriptor.id}" },
                        itemContent = { entry ->
                            WorkspaceRow(
                                label = entry.descriptor.label,
                                icon = IconToken.Play,
                                selected = selectedWorkspaceTest(form)?.let {
                                    it.node.id == node.id && it.descriptor.id == entry.descriptor.id
                                } == true,
                                warn = false,
                                onClick = { form.selectWorkspaceTest(node.id, entry.descriptor.id) },
                            )
                        },
                    )
                }
            }
        } else if (position != null) {
            if (position.nodes.isEmpty()) MenuNote("暂无扩展")
            CellGroup(
                items = position.nodes,
                key = { it.id },
                itemContent = { node ->
                    WorkspaceRow(
                        label = if (position.isCore) "模型与参数" else node.label,
                        icon = node.icon,
                        selected = !form.showingNodeStatus && form.selectedFlowNodeId == node.id,
                        warn = sectionCapability(meta, node.sectionId)?.phase in setOf("failed", "degraded"),
                        onClick = { form.showNodeParameters(node.id) },
                    )
                },
            )
            if (position.nodes.isNotEmpty()) {
                Cell(
                    title = "运行状态",
                    onClick = { form.showNodeStatus() },
                    titleColor = if (form.showingNodeStatus) Theme.colors.primary else null,
                    leading = {
                        GearIcon(
                            token = IconToken.Info,
                            size = 18.dp,
                            tint = if (form.showingNodeStatus) Theme.colors.primary else Theme.colors.mutedForeground,
                        )
                    },
                )
            }
        } else {
            // 单页模式兜底（`@Page("config")` 没有侧边栏）：停在全局 section 时列出它们，
            // 保证独立配置页仍能改服务 / MCP / OTA。统一管理台不会走到这里。
            CellGroup(
                items = ConfigSections.global,
                key = { it.id },
                itemContent = { section ->
                    WorkspaceRow(
                        label = section.label,
                        icon = globalSectionIcon(section.id),
                        selected = form.configSection == section.id,
                        warn = sectionCapability(meta, section.id)?.phase in setOf("failed", "degraded"),
                        onClick = {
                            form.configSection = section.id
                            form.showNodeParameters()
                        },
                    )
                },
            )
        }
    }
}

/** 菜单行：图标 + 文案 + 选中色 + 告警点。 */
@Composable
private fun WorkspaceRow(
    label: String,
    icon: IconToken,
    selected: Boolean,
    warn: Boolean,
    onClick: () -> Unit,
) {
    val tint = if (selected) Theme.colors.primary else Theme.colors.mutedForeground
    Cell(
        title = label,
        onClick = onClick,
        titleColor = if (selected) Theme.colors.primary else null,
        leading = { GearIcon(token = icon, size = 18.dp, tint = tint) },
        trailing = if (warn) {
            { dot(color = Theme.colors.warning, size = 5.dp) }
        } else {
            null
        },
    )
}

@Composable
private fun MenuNote(text: String) {
    Text(
        text = text,
        modifier = Modifier.padding(horizontal = Spacing.md, vertical = Spacing.xs),
        style = Theme.typography.caption,
        color = Theme.colors.mutedForeground,
    )
}

/**
 * 「全局配置」页布局（侧边栏独立菜单）：**没有**语音工作流轨道、也**没有**配置/测试 Tab。
 *
 * 左侧是全局 section 的分组菜单（总览 / 上下文增强 / 服务与接入），右侧复用
 * [ConfigSectionHost] 渲染同一套卡片；保存仍由顶部「保存配置」按钮统一提交
 *（与「语音流程」共用一份 [ConfigFormState]、一次 `POST /api/config`）。
 */
@Composable
fun GlobalConfigLayout(
    form: ConfigFormState,
    meta: PluginMetaState,
    health: EngineHealthState,
    mcp: McpToolsState,
    ota: OtaState,
    toast: ToastState,
) {
    Row(modifier = Modifier.fillMaxWidth().fillMaxHeight()) {
        Column(
            modifier = Modifier
                .width(workspaceMenuWidth())
                .fillMaxHeight()
                .padding(vertical = Spacing.xs),
        ) {
            GlobalConfigMenu(form, meta)
        }
        dividerV()
        pageScroller {
            ConfigSectionHost(form, meta, health, mcp, ota, toast, form.configSection)
        }
    }
}

/** 全局配置左菜单：只列没有流程节点的 section，按 [ConfigSections.globalGroups] 分组。 */
@Composable
private fun GlobalConfigMenu(form: ConfigFormState, meta: PluginMetaState) {
    for ((groupName, sections) in ConfigSections.globalGroups) {
        Text(
            text = groupName,
            modifier = Modifier.padding(horizontal = Spacing.md, vertical = Spacing.xs),
            style = Theme.typography.caption,
            color = Theme.colors.mutedForeground,
        )
        CellGroup(
            items = sections,
            key = { it.id },
            itemContent = { section ->
                WorkspaceRow(
                    label = section.label,
                    icon = globalSectionIcon(section.id),
                    selected = form.configSection == section.id,
                    warn = sectionCapability(meta, section.id)?.phase in setOf("failed", "degraded"),
                    onClick = { form.selectGlobalSection(section.id) },
                )
            },
        )
    }
}

/** 全局 section 的菜单图标（无流程位置，故不复用流程节点图标）。 */
private fun globalSectionIcon(sectionId: String): IconToken = when (sectionId) {
    "overview" -> IconToken.Info
    "presets" -> IconToken.Save
    "aiui" -> IconToken.Model
    "echomuse" -> IconToken.Waveform
    "mcp" -> IconToken.Intent
    "ota" -> IconToken.Document
    else -> IconToken.Settings
}

/**
 * 节点「运行状态」卡：服务端配置投影 + 运行期探测投影 + schema 提示。
 *
 * ⚠️ 明确写"不代表完成了一次模型推理或设备播放"——这两者只有真实设备上跑过才算。
 */
@Composable
fun WorkspaceNodeStatus(
    form: ConfigFormState,
    meta: PluginMetaState,
    health: EngineHealthState,
    node: PipelineFlowNode,
) {
    val ctx = LocalAdminPagerOrThrow()
    LabeledCard(
        title = "${node.label} · 运行状态",
        trailing = {
            Button(
                text = if (meta.loading || health.loading) "读取中…" else "刷新状态",
                onClick = {
                    meta.load(ctx, form.serverBase)
                    health.load(ctx, form.serverBase)
                },
                type = ButtonType.OUTLINE,
                loading = meta.loading || health.loading,
            )
        },
    ) {
        Text(
            text = node.summary,
            style = Theme.typography.caption,
            color = Theme.colors.mutedForeground,
        )
        val cap = sectionCapability(meta, node.sectionId)
        HintLine(
            if (cap != null) {
                "配置能力：${cap.phase} · ${cap.implDisplay.ifBlank { "未声明实现" }}" +
                    (if (cap.reason.isNotBlank()) "\n${cap.reason}" else "")
            } else {
                "配置能力：${meta.loadError.ifBlank { "服务端未声明此节点的独立能力" }}"
            },
        )
        val probe = health.rows.firstOrNull { it.capability == node.capabilityId }
        HintLine(
            if (probe != null) {
                "健康探测：${probe.phase} · ${probe.probeState.ifBlank { "未探测" }}\n${probe.probeDetail}\n${probe.lastError}"
            } else {
                "健康探测：${health.loadError.ifBlank { "暂无此节点的独立探测数据" }}"
            },
        )
        HintLine(form.schema.sectionNote(node.sectionId))
        HintLine("以上为服务端配置与运行期探测投影，不代表完成了一次模型推理或设备播放。")
    }
}

/** 行内「左标题 │ 右操作」的窄条（旧 DSL `headerRow` 的最小等价物）。 */
@Composable
internal fun TitleRow(title: String, trailing: (@Composable () -> Unit)? = null) {
    Row(
        modifier = Modifier.fillMaxWidth().padding(bottom = Spacing.xs),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(text = title, style = Theme.typography.titleSmall, color = Theme.colors.foreground)
        Box(Modifier.weight(1f))
        if (trailing != null) trailing()
    }
}
