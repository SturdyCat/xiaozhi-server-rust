package com.xiaozhi.admin

import com.tencent.kuikly.core.base.CursorType
import com.tencent.kuikly.core.base.ViewContainer
import com.tencent.kuikly.core.directives.vbind
import com.tencent.kuikly.core.directives.vfor
import com.tencent.kuikly.core.pager.Pager
import com.tencent.kuikly.core.views.ScrollParams
import com.tencent.kuikly.core.views.Scroller
import com.tencent.kuikly.core.views.TabItem
import com.tencent.kuikly.core.views.Tabs
import com.tencent.kuikly.core.views.Text
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
    View {
        attr { flex(1f); flexDirectionColumn() }
        nodeWorkspaceTabs(form)
        View {
            attr { height(1f); backgroundColor(AdminColors.divider) }
        }
        View {
            attr { flex(1f); flexDirectionRow() }
            View {
                attr {
                    width(if (workspaceIsCompact(ctx)) 92f else 192f)
                    backgroundColor(AdminColors.sidebarBg); flexDirectionColumn()
                }
                // Rebuild menu structure only when changing tab or workflow position.
                vbind({ "${form.workflowPosition?.id ?: "global"}@${form.workspaceTab}" }) {
                    workspaceMenu(form, meta, ctx)
                }
            }
            View { attr { width(1f); backgroundColor(AdminColors.divider) } }
            View {
                attr { flex(1f); flexDirectionColumn() }
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
            }
        }
    }
}

private fun ViewContainer<*, *>.nodeWorkspaceTabs(form: ConfigFormState) {
    val tabs = listOf(NodeWorkspaceTab.Configuration to "配置", NodeWorkspaceTab.Testing to "测试")
    Tabs {
        attr {
            height(44f)
            backgroundColor(AdminColors.sidebarBg)
            defaultInitIndex(if (form.workspaceTab == NodeWorkspaceTab.Testing) 1 else 0)
            scrollParams(ScrollParams(
                offsetX = if (form.workspaceTab == NodeWorkspaceTab.Testing) 112f else 0f,
                offsetY = 0f, contentWidth = 224f, contentHeight = 44f, viewWidth = 112f, viewHeight = 44f, isDragging = false,
            ))
            // Official Tabs slider: a rounded pill slides beneath the selected label.
            indicatorAlignCenter()
            indicatorInTabItem {
                View {
                    attr {
                        absolutePosition(left = 34f, right = 34f, bottom = 5f)
                        height(3f)
                        borderRadius(1.5f)
                        backgroundColor(AdminColors.accent)
                    }
                }
            }
        }
        for ((tab, label) in tabs) {
            TabItem { _ ->
                attr { width(112f); height(44f); allCenter(); cursor(CursorType.POINTER) }
                Text {
                    attr {
                        text(label); fontSize(AdminType.body); fontWeightMedium()
                        color(if (form.workspaceTab == tab) AdminColors.accentTintText else AdminColors.textSecondary)
                    }
                }
                event { click { form.selectWorkspaceTab(tab) } }
            }
        }
    }
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
