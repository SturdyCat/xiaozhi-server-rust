package com.xiaozhi.admin

import androidx.compose.runtime.Composable
import com.gearui.components.tabs.Tab
import com.gearui.components.tabs.Tabs
import com.gearui.components.tabs.TabsSize
import com.gearui.foundation.layout.Spacing
import com.gearui.foundation.primitives.Text
import com.gearui.theme.Theme
import com.tencent.kuikly.compose.foundation.layout.Column
import com.tencent.kuikly.compose.foundation.layout.Row
import com.tencent.kuikly.compose.foundation.layout.fillMaxHeight
import com.tencent.kuikly.compose.foundation.layout.fillMaxWidth
import com.tencent.kuikly.compose.foundation.layout.height
import com.tencent.kuikly.compose.foundation.layout.padding
import com.tencent.kuikly.compose.foundation.layout.width
import com.tencent.kuikly.compose.ui.Modifier
import com.tencent.kuikly.compose.ui.unit.dp
import com.xiaozhi.admin.kit.LocalAdminViewport
import com.xiaozhi.admin.kit.dividerV
import com.xiaozhi.admin.kit.pageScroller

/**
 * 「语音流程」工作区布局：**配置 / 测试双 Tab → 左菜单 │ 右面板**。
 *
 * ```
 * ┌ 配置 │ 测试 ────────────────────────────┐
 * │ 左菜单（配置项目/测试项目） │ 右面板（滚动区）│
 * └────────────────────────────────────────┘
 * ```
 *
 * ## 组件映射（方案 §4.1）
 *
 * | 旧 DSL | 现在 |
 * | --- | --- |
 * | 自绘 `tabBar`（固定宽 112 / 指示条内缩 34） | GearUI `Tabs`（胶囊态，宽度自适应） |
 * | `menuPaneRow(92/192, menuBackground=sidebarBg)` | `Row` + [WorkspaceMenu] + 1dp [dividerV] |
 * | `Scroller` + 列表指令包 `form.configSectionList` | `pageScroller` + 直接调用 [WorkspaceContent] |
 *
 * ## 为什么不再有 `vbind` / 列表指令
 *
 * 旧版用 `vbind("${position}@${tab}")` 手动限定菜单重建时机、用列表指令（`v` + `for`）
 * 触发面板重建 —— 那是 Kuikly DSL 的响应式机制。Compose 里 `WorkspaceMenu` / `WorkspaceContent`
 * 直接读 `stateVar` 快照字段，值一变就重组，**不需要也不应该**再手工圈定重建范围。
 */
@Composable
fun WorkflowWorkspaceLayout(
    form: ConfigFormState,
    meta: PluginMetaState,
    health: EngineHealthState,
    mcp: McpToolsState,
    ota: OtaState,
    toast: ToastState,
) {
    Column(modifier = Modifier.fillMaxWidth().fillMaxHeight()) {
        WorkspaceTabs(form)
        Row(modifier = Modifier.fillMaxWidth().fillMaxHeight()) {
            Column(
                modifier = Modifier
                    .width(if (LocalAdminViewport.current.workspaceCompact) 92.dp else 192.dp)
                    .fillMaxHeight(),
            ) {
                WorkspaceMenu(form, meta)
            }
            // 「左菜单 │ 面板」的 1dp 竖线：与侧栏、卡片分隔线同一出处（kit.dividerV）
            dividerV()
            pageScroller(
                horizontalPadding = Spacing.sm,
                paddingTop = Spacing.md,
                paddingBottom = Spacing.xxxl,
            ) {
                WorkspaceContent(form, meta, health, mcp, ota, toast)
            }
        }
    }
}

/** 双 Tab：配置 / 测试。选中项跟着 `form.workspaceTab` 走（状态唯一真源）。 */
@Composable
private fun WorkspaceTabs(form: ConfigFormState) {
    val tabs = listOf(
        Tab(id = NodeWorkspaceTab.Configuration.name, label = "配置"),
        Tab(id = NodeWorkspaceTab.Testing.name, label = "测试"),
    )
    Tabs(
        items = tabs,
        selectedId = form.workspaceTab.name,
        onSelect = { id ->
            val tab = if (id == NodeWorkspaceTab.Testing.name) NodeWorkspaceTab.Testing else NodeWorkspaceTab.Configuration
            form.selectWorkspaceTab(tab)
        },
        modifier = Modifier.fillMaxWidth().height(44.dp).padding(horizontal = Spacing.lg),
        size = TabsSize.SMALL,
    )
}

/**
 * 右面板内容（按"当前在选择什么"分四种情形，与旧 `workspaceContent` 逐条对应）：
 *
 * 1. 停在**空连接点**（有扩展位置但没挂任何节点）→ 说明"此处保留流程衔接位置"；
 * 2. 测试 Tab → 选中动作的说明卡 + [NodeTestPanel]；
 * 3. 「运行状态」→ [WorkspaceNodeStatus]；
 * 4. 其余 → 节点说明卡 + [ConfigSectionHost]（真正渲染该 section 的表单）。
 */
@Composable
private fun WorkspaceContent(
    form: ConfigFormState,
    meta: PluginMetaState,
    health: EngineHealthState,
    mcp: McpToolsState,
    ota: OtaState,
    toast: ToastState,
) {
    val position = form.workflowPosition
    val node = form.selectedFlowNodeId?.let { PipelineFlow.forId(it) }

    if (position != null && position.nodes.isEmpty()) {
        LabeledCard(title = "${position.label} · 连接点") {
            Text(
                text = "暂无扩展",
                style = Theme.typography.bodyMedium,
                color = Theme.colors.foreground,
            )
            HintLine("此处保留流程衔接位置，不额外增加必经阶段。尚未挂载扩展，因此没有配置或可执行测试。")
        }
        return
    }

    if (form.workspaceTab == NodeWorkspaceTab.Testing) {
        val test = selectedWorkspaceTest(form)
        if (test != null) {
            LabeledCard(title = test.node.label) { HintLine(test.node.summary) }
            NodeTestPanel(form, test.node, selectedActionId = test.descriptor.id)
        } else {
            LabeledCard(title = "测试") {
                Text("当前项目暂无共享节点测试。", style = Theme.typography.bodyMedium, color = Theme.colors.foreground)
                HintLine("本机录音与播放诊断仅在原生测试台提供；不会在 Web 伪装可用。")
            }
        }
        return
    }

    if (form.showingNodeStatus && node != null) {
        WorkspaceNodeStatus(form, meta, health, node)
        return
    }

    if (node != null) {
        LabeledCard(title = node.label) {
            HintLine(node.summary)
            if (!node.isCore) {
                HintLine("扩展位置：${position?.label.orEmpty()}；复用原功能配置与草稿，不代表本轮已执行。")
            }
        }
    }
    ConfigSectionHost(form, meta, health, mcp, ota, toast, form.configSection)
}
