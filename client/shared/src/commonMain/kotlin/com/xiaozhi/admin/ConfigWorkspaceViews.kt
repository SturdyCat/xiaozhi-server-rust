package com.xiaozhi.admin

import com.tencent.kuikly.core.base.CursorType
import com.tencent.kuikly.core.base.ViewContainer
import com.tencent.kuikly.core.directives.vif
import com.tencent.kuikly.core.pager.IPager
import com.tencent.kuikly.core.pager.Pager
import com.tencent.kuikly.core.views.Scroller
import com.tencent.kuikly.core.views.Text
import com.tencent.kuikly.core.views.View

/** Width below the global workflow, excluding the persistent primary shell sidebar. */
internal fun adminContentWidth(ctx: IPager): Float {
    val width = ctx.pageData.pageViewWidth
    val sidebar = if (ctx is AdminPage && width >= NARROW_BREAKPOINT) ADMIN_SIDEBAR_WIDTH + 1f else 0f
    return (width - sidebar).coerceAtLeast(0f)
}

internal fun workspaceIsCompact(ctx: IPager): Boolean = adminContentWidth(ctx) < 700f

internal data class WorkspaceTestEntry(val node: PipelineFlowNode, val descriptor: NodeTestDescriptor)

private fun workspaceTests(form: ConfigFormState): List<WorkspaceTestEntry> =
    form.workflowPosition?.nodes.orEmpty().flatMap { node -> nodeVisibleTests(node).map { WorkspaceTestEntry(node, it) } }

internal fun selectedWorkspaceTest(form: ConfigFormState): WorkspaceTestEntry? {
    val entries = workspaceTests(form)
    return entries.firstOrNull { it.node.id == form.selectedFlowNodeId && it.descriptor.id == form.selectedTestActionId }
        ?: entries.firstOrNull { it.node.id == form.selectedFlowNodeId }
        ?: entries.firstOrNull()
}

/** Navigation cells adapt to the persistent compact rail; no copied active-node state. */
internal fun ViewContainer<*, *>.workspaceMenu(form: ConfigFormState, meta: PluginMetaState, ctx: Pager) {
    val position = form.workflowPosition
    Scroller {
        attr { flex(1f); showScrollerIndicator(false); paddingTop(AdminSpace.xs); paddingBottom(AdminSpace.lg) }
        workspaceMenuHeading(if (form.workspaceTab == NodeWorkspaceTab.Configuration) "配置项目" else "测试项目")
        if (form.workspaceTab == NodeWorkspaceTab.Testing) {
            val entries = workspaceTests(form)
            if (entries.isEmpty()) workspaceMenuHeading("暂无测试")
            for (node in position?.nodes.orEmpty()) {
                val tests = entries.filter { it.node.id == node.id }
                if (position?.isCore == false && tests.isNotEmpty()) workspaceMenuHeading(node.label)
                for (entry in tests) {
                    workspaceMenuItem(entry.descriptor.label, ctx, IconToken.Play,
                        selected = { selectedWorkspaceTest(form)?.let { it.node.id == node.id && it.descriptor.id == entry.descriptor.id } == true },
                    ) { form.selectWorkspaceTest(node.id, entry.descriptor.id) }
                }
            }
        } else if (position != null) {
            if (position.nodes.isEmpty()) workspaceMenuHeading("暂无扩展")
            for (node in position.nodes) {
                workspaceMenuItem(if (position.isCore) "模型与参数" else node.label, ctx, node.icon,
                    selected = { !form.showingNodeStatus && form.selectedFlowNodeId == node.id },
                    warning = { sectionCapability(meta, node.sectionId)?.phase in setOf("failed", "degraded") },
                ) { form.showNodeParameters(node.id) }
            }
            if (position.nodes.isNotEmpty()) {
                workspaceMenuItem("运行状态", ctx, IconToken.Info, selected = { form.showingNodeStatus }) { form.showNodeStatus() }
            }
        } else {
            // Global settings remain reachable without becoming obligatory voice stages.
            for (section in ConfigSections.all.filter { it.id !in VoiceStageId.entries.map { stage -> stage.id } }) {
                workspaceMenuItem(section.label, ctx, IconToken.Settings,
                    selected = { form.configSection == section.id },
                    warning = { sectionCapability(meta, section.id)?.phase in setOf("failed", "degraded") },
                ) { form.configSection = section.id; form.showNodeParameters() }
            }
        }
    }
}

private fun ViewContainer<*, *>.workspaceMenuHeading(label: String) {
    Text {
        attr {
            text(label); fontSize(AdminType.micro); color(AdminColors.textTertiary)
            marginLeft(AdminSpace.xs); marginRight(AdminSpace.xs); marginTop(AdminSpace.sm); marginBottom(AdminSpace.xs)
        }
    }
}

private fun ViewContainer<*, *>.workspaceMenuItem(
    label: String,
    ctx: Pager,
    icon: IconToken,
    selected: () -> Boolean,
    warning: () -> Boolean = { false },
    onClick: () -> Unit,
) {
    View {
        attr {
            minHeight(44f); flexDirectionRow(); alignItemsCenter()
            marginLeft(4f); marginRight(4f); padding(AdminSpace.xs); borderRadius(AdminShape.radiusSm)
            backgroundColor(if (selected()) AdminColors.sidebarSelectedBg else AdminColors.transparent)
            cursor(CursorType.POINTER); accessibility("$label${if (selected()) "，已选中" else ""}")
        }
        event { click { onClick() } }
        vif({ !workspaceIsCompact(ctx) }) {
            View {
                attr { marginRight(AdminSpace.xs); touchEnable(false) }
                appIcon({ icon }, size = 16f, tint = { if (selected()) AdminColors.accentTintText else AdminColors.textTertiary })
            }
        }
        Text {
            attr {
                flex(1f); text(label); lines(3); fontWeightMedium()
                fontSize(if (workspaceIsCompact(ctx)) AdminType.micro else AdminType.caption)
                color(if (selected()) AdminColors.accentTintText else AdminColors.textPrimary); touchEnable(false)
            }
        }
        vif({ warning() }) {
            View { attr { size(5f, 5f); borderRadius(3f); marginLeft(3f); backgroundColor(AdminColors.warning); touchEnable(false) } }
        }
    }
}

internal fun ViewContainer<*, *>.workspaceNodeStatus(
    form: ConfigFormState,
    meta: PluginMetaState,
    health: EngineHealthState,
    ctx: Pager,
    node: PipelineFlowNode,
) {
    groupedCard("${node.label} · 运行状态") {
        textNote(node.summary, micro = true)
        textNote({ sectionCapability(meta, node.sectionId)?.let {
            "配置能力：${it.phase} · ${it.implDisplay.ifBlank { "未声明实现" }}${if (it.reason.isNotBlank()) "\n${it.reason}" else ""}"
        } ?: "配置能力：${meta.loadError.ifBlank { "服务端未声明此节点的独立能力" }}" })
        textNote({ health.rows.firstOrNull { it.capability == node.capabilityId }?.let {
            "健康探测：${it.phase} · ${it.probeState.ifBlank { "未探测" }}\n${it.probeDetail}\n${it.lastError}"
        } ?: "健康探测：${health.loadError.ifBlank { "暂无此节点的独立探测数据" }}" }, micro = true)
        textNote({ form.schema.sectionNote(node.sectionId) }, micro = true)
        textNote("以上为服务端配置与运行期探测投影，不代表完成了一次模型推理或设备播放。", micro = true)
        actionRow {
            appButton("刷新状态", variant = ButtonVariant.SECONDARY, icon = IconToken.Refresh,
                loading = { meta.loading || health.loading }, loadingText = "读取中…",
            ) { meta.load(ctx, form.serverBase); health.load(ctx, form.serverBase) }
        }
    }
}
