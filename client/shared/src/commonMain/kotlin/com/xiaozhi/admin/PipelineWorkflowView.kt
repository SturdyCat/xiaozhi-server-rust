package com.xiaozhi.admin

import com.xiaozhi.admin.ui.*

import com.tencent.kuikly.core.base.Border
import com.tencent.kuikly.core.base.BorderStyle
import com.tencent.kuikly.core.base.CursorType
import com.tencent.kuikly.core.base.ViewContainer
import com.tencent.kuikly.core.pager.Pager
import com.tencent.kuikly.core.views.Scroller
import com.tencent.kuikly.core.views.Text
import com.tencent.kuikly.core.views.View

/** Same global workflow in the admin shell and standalone config page. */
internal const val WORKFLOW_HEADER_HEIGHT = 121f
private const val CONNECTION_WIDTH = 44f

fun ViewContainer<*, *>.pipelineWorkflowHeader(
    form: ConfigFormState,
    ctx: Pager,
    active: () -> Boolean = { true },
    onSelect: () -> Unit = {},
    topInset: Float = 0f,
) {
    // 阶段宽按可见内容区自适应（adminContentWidth 扣常驻侧栏；独立配置页无侧栏取整窗）：
    // 桌面视口 ≥1280 时整条轨道完整呈现、无横向滚动，且不设最大上限（更宽继续撑满）；
    // 更窄回退 112f 最小阶段宽，由下方 Scroller 承接横向滚动（窄屏契约不变）。
    val stageWidth = { ((adminContentWidth(ctx) - 32f - 6 * CONNECTION_WIDTH) / 5f).coerceAtLeast(112f) }
    View {
        attr {
            flexDirectionColumn()
            height(WORKFLOW_HEADER_HEIGHT + topInset)
            paddingTop(topInset)
            backgroundColor(AdminColors.sidebarBg)
        }
        // 标题行：统一 headerRow（左标题 │ 右操作）；高度由外层 44 固定，故不留底距
        View {
            attr { height(44f); paddingLeft(AdminSpace.md); paddingRight(AdminSpace.md) }
            headerRow(
                title = { headerTitle("语音工作流", HeaderTone.MICRO) },
                actions = {
                    appButton("全局设置", variant = ButtonVariant.TEXT, icon = IconToken.Settings) {
                        form.openConfigSection("server")
                        onSelect()
                    }
                },
                bottomGap = 0f,
            )
        }
        Scroller {
            attr { height(76f); flexDirectionRow(); showScrollerIndicator(false) }
            View {
                attr {
                    flexDirectionRow(); alignItemsCenter(); height(76f)
                    width(5 * stageWidth() + 6 * CONNECTION_WIDTH)
                    marginLeft(AdminSpace.md); marginRight(AdminSpace.md)
                }
                for (position in PipelineWorkflow.positions) {
                    val selected = { active() && form.workflowPosition?.id == position.id }
                    val select = { form.selectWorkflowPosition(position.id); onSelect() }
                    if (position.isCore) workflowStage(position, stageWidth, selected, select)
                    else workflowConnection(position, selected, select)
                }
            }
        }
        // 轨道底边线：紧贴下方 Tabs，无尾距
        dividerH(trailingGap = 0f)
    }
}

private fun ViewContainer<*, *>.workflowStage(
    position: WorkflowPosition,
    stageWidth: () -> Float,
    selected: () -> Boolean,
    onSelect: () -> Unit,
) {
    val node = position.nodes.single()
    // 图标在左且放大（28），右侧两行文字：阶段名 / 中文说明（不再把图标挤在第一行文字前）。
    View {
        attr {
            width(stageWidth()); height(60f); flexDirectionRow(); justifyContentCenter(); alignItemsCenter()
            borderRadius(AdminShape.radiusSm)
            border(Border(1f, BorderStyle.SOLID, if (selected()) AdminColors.accent else AdminColors.dividerStrong))
            backgroundColor(if (selected()) AdminColors.accentTintBg else AdminColors.cardBg)
            cursor(CursorType.POINTER)
            accessibility("流程模块：${position.label}${if (selected()) "，已选中" else ""}")
        }
        event { click { onSelect() } }
        appIcon(
            { node.icon }, size = 28f,
            tint = { if (selected()) AdminColors.accentTintText else AdminColors.textSecondary },
        )
        View {
            attr { flexDirectionColumn(); justifyContentCenter(); marginLeft(AdminSpace.xs); touchEnable(false) }
            Text {
                attr {
                    text(position.label); fontSize(AdminType.body); fontWeightMedium()
                    color(if (selected()) AdminColors.accentTintText else AdminColors.textPrimary); touchEnable(false)
                }
            }
            Text {
                attr {
                    text(node.label.substringAfter(" · ")); fontSize(AdminType.micro)
                    color(AdminColors.textSecondary); marginTop(2f); touchEnable(false)
                }
            }
        }
    }
}

private fun ViewContainer<*, *>.workflowConnection(
    position: WorkflowPosition,
    selected: () -> Boolean,
    onSelect: () -> Unit,
) {
    val label = when (position.id) { "start" -> "流程开始"; "end" -> "流程结束"; else -> position.label }
    View {
        attr {
            width(CONNECTION_WIDTH); height(76f); cursor(CursorType.POINTER)
            accessibility("流程连接点：$label${if (selected()) "，已选中" else ""}")
        }
        event { click { onSelect() } }
        View {
            attr {
                absolutePosition(left = if (position.id == "start") 22f else 0f, right = if (position.id == "end") 22f else 0f, top = 38f)
                height(1f); backgroundColor(AdminColors.dividerStrong); touchEnable(false)
            }
        }
        View {
            attr {
                absolutePosition(left = 13f, top = 29f); size(18f, 18f); borderRadius(9f)
                border(Border(2f, BorderStyle.SOLID, if (selected()) AdminColors.accent else AdminColors.textTertiary))
                backgroundColor(if (selected()) AdminColors.accent else AdminColors.sidebarBg); touchEnable(false)
            }
        }
        if (position.id == "start" || position.id == "end") {
            Text {
                attr {
                    absolutePosition(top = 50f, left = 8f); text(position.label); fontSize(11f)
                    color(if (selected()) AdminColors.accentTintText else AdminColors.textTertiary); touchEnable(false)
                }
            }
        }
    }
}
