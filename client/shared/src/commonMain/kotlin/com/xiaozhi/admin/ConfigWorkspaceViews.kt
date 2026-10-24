package com.xiaozhi.admin

import com.xiaozhi.admin.ui.*

import com.tencent.kuikly.core.base.ViewContainer
import com.tencent.kuikly.core.directives.vbind
import com.tencent.kuikly.core.directives.vfor
import com.tencent.kuikly.core.directives.vif
import com.tencent.kuikly.core.pager.IPager
import com.tencent.kuikly.core.pager.Pager
import com.tencent.kuikly.core.views.Scroller
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
        groupLabel(if (form.workspaceTab == NodeWorkspaceTab.Configuration) "配置项目" else "测试项目", inset = AdminSpace.xs, topGap = AdminSpace.sm)
        if (form.workspaceTab == NodeWorkspaceTab.Testing) {
            val entries = workspaceTests(form)
            if (entries.isEmpty()) groupLabel("暂无测试", inset = AdminSpace.xs, topGap = AdminSpace.sm)
            for (node in position?.nodes.orEmpty()) {
                val tests = entries.filter { it.node.id == node.id }
                if (position?.isCore == false && tests.isNotEmpty()) groupLabel(node.label, inset = AdminSpace.xs, topGap = AdminSpace.sm)
                for (entry in tests) {
                    workspaceMenuItem(entry.descriptor.label, ctx, IconToken.Play,
                        selected = { selectedWorkspaceTest(form)?.let { it.node.id == node.id && it.descriptor.id == entry.descriptor.id } == true },
                    ) { form.selectWorkspaceTest(node.id, entry.descriptor.id) }
                }
            }
        } else if (position != null) {
            if (position.nodes.isEmpty()) groupLabel("暂无扩展", inset = AdminSpace.xs, topGap = AdminSpace.sm)
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
            // 单页模式兜底（`@Page("config")` 没有侧边栏，「全局配置」菜单不可达）：停在全局
            // section 时这里列出它们，保证独立配置页仍能改服务 / MCP / OTA。
            // 统一管理台（AdminPage）不会走到这里——全局配置是独立菜单，有专属布局
            //（[renderGlobalConfigLayout]）。
            for (section in ConfigSections.global) {
                workspaceMenuItem(section.label, ctx, IconToken.Settings,
                    selected = { form.configSection == section.id },
                    warning = { sectionCapability(meta, section.id)?.phase in setOf("failed", "degraded") },
                ) { form.configSection = section.id; form.showNodeParameters() }
            }
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
    // 统一菜单项组件（WORKSPACE 档）：紧凑窗口隐藏图标、窄菜单可折三行
    menuListItem(
        label = label,
        isSelected = selected,
        onClick = onClick,
        icon = icon,
        style = MenuItemStyle.WORKSPACE,
        showIcon = !workspaceIsCompact(ctx),
        // 窄菜单收字号，避免三个字以上折成四行
        fontSize = if (workspaceIsCompact(ctx)) AdminType.micro else AdminType.caption,
        trailing = {
            vif({ warning() }) {
                hGap(3f)
                dot({ AdminColors.warning }, size = 5f)
            }
        },
    )
}

/**
 * 「全局配置」页布局（侧边栏独立菜单）：**没有**语音工作流轨道、也**没有**配置/测试 Tab。
 *
 * 左侧是全局 section 的分组菜单（总览 / 上下文增强 / 服务与接入），右侧复用
 * [renderConfigSection] 渲染同一套卡片；保存仍由顶部「保存配置」按钮统一提交
 *（与「语音流程」共用一份 [ConfigFormState]、一次 `POST /api/config`）。
 *
 * 之所以另起布局而不是复用 [renderConfigLayout]：那套布局的 Tab 语义是"节点的配置 / 测试"，
 * 全局设置没有节点、也没有可执行测试，套上 Tab 会造出空壳页面。
 */
fun ViewContainer<*, *>.renderGlobalConfigLayout(
    form: ConfigFormState,
    meta: PluginMetaState,
    health: EngineHealthState,
    mcp: McpToolsState,
    ota: OtaState,
    toast: ToastState,
    ctx: Pager,
) {
    // 左右布局：全局 section 菜单 │ 1px 分隔线 │ 同一套配置卡片
    menuPaneRow(
        menuWidth = if (workspaceIsCompact(ctx)) 92f else 192f,
        fillHeight = true,
        menuBackground = AdminColors.sidebarBg,
        // 与工作流左菜单同一口径：菜单是静态项，只在选中 section 变化时重建
        menu = { vbind({ form.configSection }) { globalConfigMenu(form, meta, ctx) } },
        panel = {
            Scroller {
                attr {
                    flex(1f); showScrollerIndicator(false)
                    val compact = workspaceIsCompact(ctx)
                    paddingLeft(if (compact) AdminSpace.xs else AdminSpace.lg)
                    paddingRight(if (compact) AdminSpace.xs else AdminSpace.lg)
                    paddingTop(AdminSpace.md); paddingBottom(AdminSpace.xxl)
                }
                // 与工作区同一重建口径：路由/快照变化才重建整块内容
                vfor({ form.configSectionList }) { _ ->
                    View {
                        attr { flexDirectionColumn() }
                        renderConfigSection(form, meta, health, mcp, ota, toast, ctx, form.configSection)
                    }
                }
            }
        },
    )
}

/** 全局配置左菜单：只列没有流程节点的 section，按 [ConfigSections.globalGroups] 分组。 */
private fun ViewContainer<*, *>.globalConfigMenu(form: ConfigFormState, meta: PluginMetaState, ctx: Pager) {
    Scroller {
        attr { flex(1f); showScrollerIndicator(false); paddingTop(AdminSpace.xs); paddingBottom(AdminSpace.lg) }
        for ((groupName, sections) in ConfigSections.globalGroups) {
            groupLabel(groupName)
            for (section in sections) {
                workspaceMenuItem(section.label, ctx, globalSectionIcon(section.id),
                    selected = { form.configSection == section.id },
                    warning = { sectionCapability(meta, section.id)?.phase in setOf("failed", "degraded") },
                ) { form.selectGlobalSection(section.id) }
            }
        }
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
