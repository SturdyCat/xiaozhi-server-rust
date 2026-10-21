package com.xiaozhi.admin

import com.tencent.kuikly.core.base.ViewContainer
import com.tencent.kuikly.core.directives.vif
import com.tencent.kuikly.core.pager.Pager
import com.tencent.kuikly.core.views.View

/** One source for test menus and content; native-only actions stay in native diagnostics. */
fun nodeVisibleTests(node: PipelineFlowNode): List<NodeTestDescriptor> =
    node.tests.filter { it.visibleTo(emptySet()) } + if (node.id == "input") listOf(
        NodeTestDescriptor("pipeline.text", "文本流程验证"), NodeTestDescriptor("pipeline.audio", "音频流程验证"),
    ) else emptyList()

/** Null retains the legacy all-actions panel; a selected ID renders only that action's content. */
fun ViewContainer<*, *>.nodeTestPanel(
    form: ConfigFormState,
    ctx: Pager,
    node: PipelineFlowNode,
    selectedActionId: String? = null,
) {
    val tests = nodeVisibleTests(node).filter { selectedActionId == null || it.id == selectedActionId }
    if (tests.isEmpty()) return
    val state = form.nodeTests
    groupedCard("节点测试 · 已保存配置") {
        textNote({ state.capabilityMessage.ifBlank { "读取服务端能力后启用对应测试；不依赖本机麦克风。" } }, micro = true)
        vif({ form.dirty }) {
            textNote("当前有未保存草稿：本区域仍测试已保存的全局快照，不会把草稿伪装成已生效配置。", tone = "warn", micro = true)
        }
        actionRow {
            appButton("刷新测试能力", variant = ButtonVariant.TEXT, state = { state.capabilityState }, loadingText = "读取中…", icon = IconToken.Refresh) {
                form.refreshNodeTests(ctx)
            }
        }
        vif({ tests.any { state.capability(it.id)?.inputKind == TestInputKind.Text } }) {
            labeledField("测试文本", { state.inputText }, { state.inputText = it; state.invalidateRuns() }, "请输入测试文本")
        }
        vif({ tests.any { state.capability(it.id)?.inputKind == TestInputKind.Audio } && state.samples.isNotEmpty() }) {
            dropdownField(
                label = "真实语音样本",
                currentLabel = { state.samples.firstOrNull { it.id == state.selectedSampleId }?.label.orEmpty() },
                options = { state.sampleOptions }, selectedId = { state.selectedSampleId },
                isOpen = { state.openMenu == "sample" },
                onToggle = { state.openMenu = if (state.openMenu == "sample") "" else "sample" },
                onSelect = { state.selectedSampleId = it; state.invalidateRuns() },
            )
            textNote({ state.samples.firstOrNull { it.id == state.selectedSampleId }?.let {
                "参考文本：${it.transcript}\n授权：${it.license}；识别结果以真实返回为准。"
            }.orEmpty() }, micro = true)
        }
        for (test in tests) nodeTestAction(form, ctx, test)
        textNote("计时是服务端/提供方处理口径，不代表设备收到音频或已经播放。串联验证明确跳过未运行阶段，不执行设备动作，也不写入对话历史。", micro = true)
    }
}

private fun ViewContainer<*, *>.nodeTestAction(form: ConfigFormState, ctx: Pager, descriptor: NodeTestDescriptor) {
    val state = form.nodeTests
    View {
        attr { flexDirectionColumn(); marginTop(AdminSpace.sm) }
        actionRow {
            appButton(
                descriptor.label, variant = ButtonVariant.SECONDARY,
                state = { state.runState(descriptor.id).operation }, availability = { state.availability(descriptor) },
                loadingText = "测试中…", successText = "测试完成", errorText = "测试失败 · 重试", icon = IconToken.Play,
            ) { form.runNodeTest(ctx, descriptor) }
        }
        vif({ state.availability(descriptor) is Availability.Disabled }) {
            textNote({ (state.availability(descriptor) as? Availability.Disabled)?.reason.orEmpty() }, tone = { "warn" }, micro = true)
        }
        vif({ state.runState(descriptor.id).result != null }) {
            View {
                attr { flexDirectionColumn(); padding(AdminSpace.sm); backgroundColor(AdminColors.insetBg); borderRadius(AdminShape.radiusSm) }
                textNote({ "本次输入：${state.runState(descriptor.id).inputSummary}" }, micro = true)
                textNote({ state.runState(descriptor.id).result?.let {
                    "快照 ${it.revision.take(16)} · ${if (it.contextMode == "pipeline_preview") "串联预览" else "隔离节点"} · 请求 ${it.requestId.take(16)}"
                }.orEmpty() }, micro = true)
                textNote({ state.runState(descriptor.id).result?.let { formatNodeTestStage(it.stage) }.orEmpty() },
                    tone = { if (state.runState(descriptor.id).result?.succeeded == true) "tertiary" else "error" })
                textNote({ state.runState(descriptor.id).result?.stages?.joinToString("\n\n") { formatNodeTestStage(it) }.orEmpty() }, micro = true)
            }
        }
    }
}

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
