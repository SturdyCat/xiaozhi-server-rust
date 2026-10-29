package com.xiaozhi.admin

import androidx.compose.runtime.Composable
import com.gearui.components.button.Button
import com.gearui.components.button.ButtonType
import com.gearui.components.cell.Cell
import com.gearui.components.cellgroup.CellGroup
import com.gearui.components.steps.StepItem
import com.gearui.components.steps.StepStatus
import com.gearui.components.steps.Steps
import com.gearui.components.steps.StepsDirection
import com.gearui.foundation.layout.Spacing
import com.gearui.foundation.primitives.Text
import com.gearui.theme.Theme
import com.tencent.kuikly.compose.foundation.background
import com.tencent.kuikly.compose.foundation.shape.RoundedCornerShape
import com.tencent.kuikly.compose.foundation.layout.Box
import com.tencent.kuikly.compose.foundation.layout.Column
import com.tencent.kuikly.compose.foundation.layout.Row
import com.tencent.kuikly.compose.foundation.layout.fillMaxWidth
import com.tencent.kuikly.compose.foundation.layout.padding
import com.tencent.kuikly.compose.foundation.layout.size
import com.tencent.kuikly.compose.ui.Alignment
import com.tencent.kuikly.compose.ui.Modifier
import com.tencent.kuikly.compose.ui.draw.clip
import com.tencent.kuikly.compose.ui.unit.dp
import com.xiaozhi.admin.contracts.IconToken
import com.xiaozhi.admin.icons.GearIcon
import com.xiaozhi.admin.kit.statusBadge

/**
 * 语音工作流的**流程轨道**（Compose 版）；管理台与独立配置页共用。
 *
 * ## 结构（三段，自上而下）
 *
 * ```
 * 语音工作流                                    全局设置
 * ── Steps：VAD → ASR → NLU → LLM → TTS（点阶段即切到该阶段的配置）──
 * 连接点分支（CellGroup）：开始 / 各阶段之间 / 结束，挂着唤醒、指令、人格、记忆、音频入口输出
 * ```
 *
 * ## 为什么轨道用 `Steps`、分支用 `Cell`
 *
 * 方案 §4.1 的口径：**轨道是语音流程的视觉主语**，不能退化成普通列表 —— 用户要一眼看出
 * "一次对话按 VAD→ASR→NLU→LLM→TTS 走"。`Steps` 正好是这条横向轨道；而连接点上的分支
 * 节点（唤醒 / 指令 / 人格 / 记忆 / 音频入口输出）不是必经阶段，它们**挂在**轨道上，
 * 用 `CellGroup` + `Cell` 表达"某个位置可以插什么"，层次上就不会被误读成第六阶段。
 *
 * ## 与旧 DSL 实现的差异（登记）
 *
 * - 旧版是自绘的横向轨道（44pt 连接点 + 自适应阶段宽 + 横向滚动 + 绝对定位连线与小圆点）。
 *   新版交给 `Steps`：**阶段宽、连线、圆点、当前步高亮全部由组件负责**，不再有像素常量
 *   （旧的 `CONNECTION_WIDTH` / `stageWidth()` 计算随之删除）。这条差异属方案 §4.1 允许的
 *   "组件自带视觉"范围。
 * - 旧的横向滚动是"窄屏时轨道可左右滑"；`Steps` 在窄视口下会压缩标题而不是滚动，
 *   故这里不再包 `ScrollView`（轨道只有 5 段，压缩后可读）。
 * - 分支位置**从轨道里移到下方 CellGroup**：旧版把连接点画在轨道中间（点小圆圈切换），
 *   但那需要绝对定位的自绘轨道；换成 `Cell` 之后每个分支有了可读的标题与说明，
 *   信息量比一个小圆点更多，且不需要"猜圆圈里是什么"。
 *
 * @param active 本轨道是否处于"当前生效"状态（独立配置页的全局模式传 false，避免高亮误导）
 * @param onSelect 选中任意阶段/分支后的回调（壳层用它收起抽屉等）
 */
@Composable
fun PipelineWorkflowHeader(
    form: ConfigFormState,
    modifier: Modifier = Modifier,
    active: Boolean = true,
    onSelect: () -> Unit = {},
) {
    Column(
        modifier = modifier
            .fillMaxWidth()
            .background(Theme.colors.surface)
            .padding(top = Spacing.sm, bottom = Spacing.sm),
    ) {
        WorkflowTitleRow(form, onSelect)
        WorkflowSteps(form, active, onSelect)
        WorkflowBranches(form, active, onSelect)
    }
}

/** 标题行：「语音工作流」左，控制动作「全局设置」右（跳到 `[server]` 段）。 */
@Composable
private fun WorkflowTitleRow(form: ConfigFormState, onSelect: () -> Unit) {
    Row(
        modifier = Modifier.fillMaxWidth().padding(horizontal = Spacing.lg, vertical = Spacing.xs),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(
            text = "语音工作流",
            style = Theme.typography.caption,
            color = Theme.colors.mutedForeground,
        )
        Box(Modifier.weight(1f))
        Button(
            text = "全局设置",
            onClick = {
                form.openConfigSection("server")
                onSelect()
            },
            type = ButtonType.TEXT,
        )
    }
}

/**
 * 横向流程轨道：只渲染**核心阶段**（`position.isCore`），连接点不占步位。
 *
 * `current` 取当前选中位置在核心阶段里的下标；选中态是"用户正在改哪个阶段"，
 * 与"运行到哪一步"无关 —— 后者在「运行状态」里由服务端投影给出，二者不混用。
 */
@Composable
private fun WorkflowSteps(form: ConfigFormState, active: Boolean, onSelect: () -> Unit) {
    val stages = PipelineWorkflow.positions.filter { it.isCore }
    if (stages.isEmpty()) return
    val selectedId = form.workflowPosition?.id
    val current = stages.indexOfFirst { it.id == selectedId }.coerceAtLeast(0)

    Steps(
        current = current,
        items = stages.map { position ->
            StepItem(
                title = position.label,
                // 阶段副标题取节点名去掉"VAD · "前缀后的中文说明（如「语音分段」）
                description = position.nodes.firstOrNull()?.label?.substringAfter(" · "),
            )
        },
        modifier = Modifier.fillMaxWidth().padding(horizontal = Spacing.sm),
        direction = StepsDirection.HORIZONTAL,
        // active=false（独立的全局配置模式）时不显示"进行中"的高亮态
        status = if (active) StepStatus.PROCESS else StepStatus.WAITING,
        onCurrentChange = { index ->
            stages.getOrNull(index)?.let { position ->
                form.selectWorkflowPosition(position.id)
                onSelect()
            }
        },
    )
}

/**
 * 连接点分支：把每个非核心位置挂上来的节点渲染成 `CellGroup` + `Cell`。
 *
 * - 位置没有节点时**不渲染空壳**（旧版会画一个空圆圈；Compose 版直接不占位，
 *   因为「这里能插什么」在无扩展时没有任何可执行动作）；
 * - 有节点时分组标题用位置语义名（开始 / VAD → ASR / 结束 …），每行=一个可点节点；
 * - 节点的能力 phase 偏离（`degraded` / `failed`）时行尾挂状态徽标（K5：降级黄、异常红）。
 */
@Composable
private fun WorkflowBranches(form: ConfigFormState, active: Boolean, onSelect: () -> Unit) {
    val groups = PipelineWorkflow.positions.filter { !it.isCore && it.nodes.isNotEmpty() }
    if (groups.isEmpty()) return
    Column(modifier = Modifier.fillMaxWidth().padding(horizontal = Spacing.lg, vertical = Spacing.xs)) {
        for (position in groups) {
            CellGroup(
                items = position.nodes,
                title = connectionTitle(position),
                key = { it.id },
                itemContent = { node ->
                    WorkflowBranchRow(
                        node = node,
                        selected = active &&
                            form.workflowPosition?.id == position.id &&
                            form.selectedFlowNodeId == node.id,
                        onClick = {
                            form.selectWorkflowPosition(position.id)
                            form.showNodeParameters(node.id)
                            onSelect()
                        },
                    )
                },
            )
        }
    }
}

/** 单个分支节点行：图标 + 名称 + 说明 + 选中指示。 */
@Composable
private fun WorkflowBranchRow(node: PipelineFlowNode, selected: Boolean, onClick: () -> Unit) {
    Cell(
        title = node.label,
        description = node.summary,
        onClick = onClick,
        titleColor = if (selected) Theme.colors.primary else null,
        leading = {
            GearIcon(
                token = node.icon,
                size = 18.dp,
                tint = if (selected) Theme.colors.primary else Theme.colors.mutedForeground,
            )
        },
        trailing = if (selected) {
            { SelectionMark() }
        } else {
            null
        },
    )
}

/** 选中标记：8dp 品牌色圆点（与侧栏选中态同源，不引入新色值）。 */
@Composable
private fun SelectionMark() {
    Box(
        modifier = Modifier
            .size(8.dp)
            .clip(RoundedCornerShape(4.dp))
            .background(Theme.colors.primary),
    )
}

/** 连接点标题：开始 / 结束用中文语义，阶段之间的连接点用「A → B」。 */
private fun connectionTitle(position: WorkflowPosition): String = when (position.id) {
    "start" -> "流程开始 · 可挂载"
    "end" -> "流程结束 · 可挂载"
    else -> position.label
}

/**
 * 轨道区高度（旧的固定 121pt 常量）。
 *
 * T07 起轨道由 `Steps` 自适应高度，**不再有固定高度**；常量保留是因为
 * `AdminPlatform`/测试台的 mac 专属布局仍在读它做窗口内分段计算（T12 随 `ui/` 一起清）。
 */
internal const val WORKFLOW_HEADER_HEIGHT = 121f

/** 阶段状态徽标：能力 phase 偏离时给一眼可辨的角标（K3/K5 语义，不自造色值）。 */
@Composable
internal fun StageStatusBadge(phase: String) {
    when (phase) {
        "failed" -> statusBadge(state = "error", text = "异常")
        "degraded" -> statusBadge(state = "busy", text = "降级")
        "active" -> Unit // 正常不打标
        "disabled" -> statusBadge(state = "idle", text = "已关闭")
        else -> if (phase.isNotEmpty()) statusBadge(state = "idle", text = phase)
    }
}

/** 阶段图标的统一尺寸（与侧栏、分支行同口径），避免各卡自写魔法数。 */
internal val StageIconSize = 18.dp
