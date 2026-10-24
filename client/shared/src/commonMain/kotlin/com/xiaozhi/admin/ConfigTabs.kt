package com.xiaozhi.admin

import com.xiaozhi.admin.ui.*

import com.tencent.kuikly.core.base.ViewContainer
import com.tencent.kuikly.core.directives.vbind
import com.tencent.kuikly.core.directives.vfor
import com.tencent.kuikly.core.pager.Pager
import com.tencent.kuikly.core.views.Scroller
import com.tencent.kuikly.core.views.View

/** Workflow selection above this layout; exactly two local tabs, each with its own left menu. */
fun ViewContainer<*, *>.renderConfigLayout(
    form: ConfigFormState,
    meta: PluginMetaState,
    health: EngineHealthState,
    mcp: McpToolsState,
    ota: OtaState,
    toast: ToastState,
    ctx: Pager,
) {
    // 上下布局：Tabs 栏 │ 1px 分隔线 │ 工作区；工作区内部再左右分栏（左菜单 │ 面板）
    splitColumn(fillHeight = true, trailingGap = 0f) {
        auto { nodeWorkspaceTabs(form) }
        grow {
            menuPaneRow(
                menuWidth = if (workspaceIsCompact(ctx)) 92f else 192f,
                fillHeight = true,
                menuBackground = AdminColors.sidebarBg,
                menu = {
                    // Rebuild menu structure only when changing tab or workflow position.
                    vbind({ "${form.workflowPosition?.id ?: "global"}@${form.workspaceTab}" }) {
                        workspaceMenu(form, meta, ctx)
                    }
                },
                panel = {
                    Scroller {
                        attr {
                            flex(1f); showScrollerIndicator(false)
                            val compact = workspaceIsCompact(ctx)
                            paddingLeft(if (compact) AdminSpace.xs else AdminSpace.lg)
                            paddingRight(if (compact) AdminSpace.xs else AdminSpace.lg)
                            paddingTop(AdminSpace.md); paddingBottom(AdminSpace.xxl)
                        }
                        vfor({ form.configSectionList }) { _ ->
                            View {
                                attr { flexDirectionColumn() }
                                workspaceContent(form, meta, health, mcp, ota, toast, ctx)
                            }
                        }
                    }
                },
            )
        }
    }
}

/**
 * 工作区顶栏的两个 tab（配置 / 测试）：走组件库 [tabBar]（官方 Tabs 封装、无 PageList 版），
 * 固定宽 112、指示条内缩 34，与切换前的既有视觉一致。
 */
private fun ViewContainer<*, *>.nodeWorkspaceTabs(form: ConfigFormState) {
    val tabs = listOf(
        NodeWorkspaceTab.Configuration to "配置",
        NodeWorkspaceTab.Testing to "测试",
    )
    // 选中下标跟着 form.workspaceTab 走：tabBar 内部按 index 合成 scrollParams 驱动指示条
    tabBar(
        tabs = tabs.map { it.second },
        selectedIndex = { tabs.indexOfFirst { it.first == form.workspaceTab }.coerceAtLeast(0) },
        onSelect = { form.selectWorkspaceTab(tabs[it].first) },
        itemWidth = 112f,
        barHeight = 44f,
        indicatorInset = 34f,
        background = AdminColors.sidebarBg,
    )
}

private fun ViewContainer<*, *>.workspaceContent(
    form: ConfigFormState,
    meta: PluginMetaState,
    health: EngineHealthState,
    mcp: McpToolsState,
    ota: OtaState,
    toast: ToastState,
    ctx: Pager,
) {
    val position = form.workflowPosition
    val node = form.selectedFlowNodeId?.let { PipelineFlow.forId(it) }
    if (position != null && position.nodes.isEmpty()) {
        groupedCard("${position.label} · 连接点") {
            textNote("暂无扩展", micro = false)
            textNote("此处保留流程衔接位置，不额外增加必经阶段。尚未挂载扩展，因此没有配置或可执行测试。", micro = true)
        }
        return
    }
    if (form.workspaceTab == NodeWorkspaceTab.Testing) {
        val test = selectedWorkspaceTest(form)
        if (test != null) {
            groupedCard(test.node.label, withDivider = false) {
                textNote(test.node.summary, micro = true)
            }
            nodeTestPanel(form, ctx, test.node, selectedActionId = test.descriptor.id)
        } else {
            groupedCard("测试") {
                textNote("当前项目暂无共享节点测试。", micro = true)
                textNote("本机录音与播放诊断仅在原生测试台提供；不会在 Web 伪装可用。", micro = true)
            }
        }
        return
    }
    if (form.showingNodeStatus && node != null) {
        workspaceNodeStatus(form, meta, health, ctx, node)
        return
    }
    if (node != null) {
        groupedCard(node.label, withDivider = false) {
            textNote(node.summary, micro = true)
            if (!node.isCore) textNote("扩展位置：${position?.label.orEmpty()}；复用原功能配置与草稿，不代表本轮已执行。", micro = true)
        }
    }
    renderConfigSection(form, meta, health, mcp, ota, toast, ctx, form.configSection)
}
