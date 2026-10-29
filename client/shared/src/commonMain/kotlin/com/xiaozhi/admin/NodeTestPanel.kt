package com.xiaozhi.admin

import androidx.compose.runtime.Composable
import com.gearui.components.alert.Alert
import com.gearui.components.alert.AlertStatus
import com.gearui.components.button.Button
import com.gearui.components.button.ButtonType
import com.gearui.components.result.Result
import com.gearui.components.result.ResultStatus
import com.gearui.foundation.layout.Spacing
import com.gearui.foundation.primitives.Text
import com.gearui.theme.Theme
import com.tencent.kuikly.compose.foundation.layout.fillMaxWidth
import com.tencent.kuikly.compose.foundation.layout.padding
import com.tencent.kuikly.compose.ui.unit.dp
import com.tencent.kuikly.compose.ui.Modifier
import com.xiaozhi.admin.contracts.ActionOutcome
import com.xiaozhi.admin.contracts.ActionState
import com.xiaozhi.admin.contracts.Availability
import com.xiaozhi.admin.kit.labeledField
import com.xiaozhi.admin.kit.labeledSelect

/**
 * 节点测试面板（Compose 版）—— 测试菜单与面板共用同一份动作清单。
 *
 * ## 关键语义（与迁移前逐字一致，不要"优化"掉）
 *
 * - **测的是已保存的全局快照，不是草稿**：`form.dirty` 时用 `Alert` 明确提示，
 *   绝不把草稿伪装成已生效配置；
 * - **能力来自服务端**：读不到能力就不启用对应动作（`Availability.Disabled` 附原因）；
 * - **本机麦克风/播放不在 Web 伪装**：音频类动作只有服务端给了样本与能力才出现；
 * - 计时口径、串联验证跳过未运行阶段、不写对话历史 —— 这些说明文字全部保留。
 *
 * ## 组件映射
 *
 * | 旧 DSL | 现在 |
 * | --- | --- |
 * | `groupedCard` | [LabeledCard] |
 * | `actionRow + appButton(state=…, availability=…)` | `actionRow` + GearUI `Button`（`loading`/`disabled`） |
 * | `insetPanel { … }` 里逐行 `textNote` | GearUI `Result`（成功/失败态）+ 阶段明细行 |
 * | `dropdownField(真实语音样本)` | [labeledSelect]（自带面板） |
 */
@Composable
fun NodeTestPanel(form: ConfigFormState, node: PipelineFlowNode, selectedActionId: String? = null) {
    val ctx = LocalAdminPagerOrThrow()
    val tests = nodeVisibleTests(node).filter { selectedActionId == null || it.id == selectedActionId }
    if (tests.isEmpty()) return
    val state = form.nodeTests

    LabeledCard(title = "节点测试 · 已保存配置") {
        HintLine(state.capabilityMessage.ifBlank { "读取服务端能力后启用对应测试；不依赖本机麦克风。" })
        if (form.dirty) {
            Alert(
                title = "当前有未保存草稿",
                description = "本区域仍测试已保存的全局快照，不会把草稿伪装成已生效配置。",
                status = AlertStatus.WARNING,
                modifier = Modifier.fillMaxWidth().padding(vertical = Spacing.xs),
            )
        }
        if (tests.any { state.capability(it.id)?.inputKind == TestInputKind.Text }) {
            labeledField(
                label = "测试文本",
                value = state.inputText,
                onValueChange = {
                    state.inputText = it
                    state.invalidateRuns()
                },
                placeholder = "请输入测试文本",
            )
        }
        val needsAudio = tests.any { state.capability(it.id)?.inputKind == TestInputKind.Audio }
        if (needsAudio && state.samples.isNotEmpty()) {
            labeledSelect(
                label = "真实语音样本",
                selectedId = state.selectedSampleId,
                options = state.sampleOptions,
                onSelect = {
                    state.selectedSampleId = it
                    state.invalidateRuns()
                },
            )
            val sample = state.samples.firstOrNull { it.id == state.selectedSampleId }
            if (sample != null) {
                HintLine("参考文本：${sample.transcript}\n授权：${sample.license}；识别结果以真实返回为准。")
            }
        }
        for (test in tests) NodeTestAction(form, test)
        HintLine(
            "计时是服务端/提供方处理口径，不代表设备收到音频或已经播放。" +
                "串联验证明确跳过未运行阶段，不执行设备动作，也不写入对话历史。",
        )
    }
    // ctx 只用于保持"必须在页面内渲染"的契约检查（动作内部经 form 调用时仍需 Pager）
    @Suppress("UNUSED_EXPRESSION")
    ctx
}

/** 单个测试动作：按钮 + 不可用原因 + 运行结果（成功/失败两态 + 阶段明细）。 */
@Composable
private fun NodeTestAction(form: ConfigFormState, descriptor: NodeTestDescriptor) {
    val ctx = LocalAdminPagerOrThrow()
    val state = form.nodeTests
    val availability = state.availability(descriptor)
    val disabledReason = (availability as? Availability.Disabled)?.reason.orEmpty()
    val operation = state.runState(descriptor.id).operation
    val result = state.runState(descriptor.id).result

    Text(
        text = descriptor.label,
        modifier = Modifier.padding(top = Spacing.sm),
        style = Theme.typography.label,
        color = Theme.colors.foreground,
    )
    // 按钮文案跟着动作状态走（Idle / Running / Finished(Success|Failure)），不用 vif 切三份按钮
    val running = operation is ActionState.Running
    val outcome = (operation as? ActionState.Finished)?.outcome
    Button(
        text = when {
            running -> "测试中…"
            outcome != null && outcome is ActionOutcome.Success -> "测试完成"
            outcome != null -> "测试失败 · 重试"
            else -> "运行测试"
        },
        onClick = { form.runNodeTest(ctx, descriptor) },
        type = ButtonType.OUTLINE,
        loading = running,
        disabled = availability is Availability.Disabled,
    )
    if (disabledReason.isNotEmpty()) {
        Text(
            text = disabledReason,
            modifier = Modifier.padding(top = 4.dp),
            style = Theme.typography.caption,
            color = Theme.colors.warning,
        )
    }
    if (result != null) {
        Text(
            text = "本次输入：${state.runState(descriptor.id).inputSummary}",
            modifier = Modifier.padding(top = Spacing.xs),
            style = Theme.typography.caption,
            color = Theme.colors.mutedForeground,
        )
        Text(
            text = "快照 ${result.revision.take(16)} · " +
                "${if (result.contextMode == "pipeline_preview") "串联预览" else "隔离节点"} · " +
                "请求 ${result.requestId.take(16)}",
            style = Theme.typography.caption,
            color = Theme.colors.mutedForeground,
        )
        // 整体结论用 Result（成功/失败两态一句话说清），下面是阶段明细
        Result(
            status = if (result.succeeded) ResultStatus.SUCCESS else ResultStatus.ERROR,
            title = if (result.succeeded) "测试完成" else "测试失败",
            description = formatNodeTestStage(result.stage),
            modifier = Modifier.fillMaxWidth(),
        )
        for (stage in result.stages) {
            Text(
                text = formatNodeTestStage(stage),
                modifier = Modifier.padding(top = Spacing.xs),
                style = Theme.typography.caption,
                color = Theme.colors.mutedForeground,
            )
        }
    }
}

/** 阶段结果 → 可读一行（口径、耗时、首 token/PCM、正文、错误码全部保留）。 */
private fun formatNodeTestStage(stage: NodeTestStage): String = buildString {
    append("${stage.stage} · ${stage.state}")
    if (stage.engine.isNotBlank()) append(" · ${stage.engine}")
    stage.elapsedMs?.let { append(" · ${it}ms") }
    stage.firstTokenMs?.let { append("\n提供方首 token：${it}ms") }
    stage.firstPcmMs?.let { append("\n提供方首 PCM：${it}ms") }
    if (stage.measurement.isNotBlank()) append(" · 口径 ${stage.measurement}")
    if (stage.text.isNotBlank()) append("\n${stage.text}")
    if (stage.error.isNotBlank()) append("\n${stage.error}")
    if (stage.errorCode.isNotBlank()) append(" [${stage.errorCode}]")
    if (stage.detailsText.isNotBlank()) append("\n${stage.detailsText}")
}

/** 节点可见的测试动作：节点自带 + 输入节点额外的两条串联验证。 */
fun nodeVisibleTests(node: PipelineFlowNode): List<NodeTestDescriptor> =
    node.tests.filter { it.visibleTo(emptySet()) } + if (node.id == "input") {
        listOf(
            NodeTestDescriptor("pipeline.text", "文本流程验证"),
            NodeTestDescriptor("pipeline.audio", "音频流程验证"),
        )
    } else {
        emptyList()
    }
