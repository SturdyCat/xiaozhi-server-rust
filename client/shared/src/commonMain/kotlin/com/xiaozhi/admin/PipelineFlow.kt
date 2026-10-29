package com.xiaozhi.admin

import com.xiaozhi.admin.contracts.*

/** One navigation source, including when two nodes share a configuration controller. */
data class ConfigSelection(
    val sectionId: String,
    val nodeId: String? = null,
    val workflowPositionId: String? = null,
)

/** Explicit attachment points; never inferred from capability-array order or UI labels. */
sealed class PipelineAnchor {
    object Start : PipelineAnchor()
    data class Before(val stage: VoiceStageId) : PipelineAnchor()
    data class After(val stage: VoiceStageId) : PipelineAnchor()
    object End : PipelineAnchor()
}

data class FlowFeatureDescriptor(
    val id: String,
    val label: String,
    val summary: String,
    val anchor: PipelineAnchor,
    val sectionId: String,
    val icon: IconToken,
    val capabilityId: String? = null,
    val configPaths: List<String> = emptyList(),
    val tests: List<NodeTestDescriptor> = emptyList(),
    val order: Int = 0,
) {
    init {
        require(id.isNotBlank() && label.isNotBlank() && sectionId.isNotBlank())
    }
}

/** Presentation/test binding, not proof that a runtime hook executed this turn. */
data class PipelineFlowNode(
    val id: String,
    val label: String,
    val summary: String,
    val sectionId: String,
    val icon: IconToken,
    val capabilityId: String?,
    val tests: List<NodeTestDescriptor>,
    val coreStage: VoiceStageId? = null,
    val anchor: PipelineAnchor? = null,
) {
    val isCore: Boolean get() = coreStage != null
    val selection: ConfigSelection get() = ConfigSelection(sectionId, id)
}

object PipelineFlow {
    // A feature may have multiple attachment nodes but shares its existing config/controller.
    // Availability remains server-reported; declaring an action never enables a fake test.
    val features: List<FlowFeatureDescriptor> = listOf(
        FlowFeatureDescriptor(
            "input", "终端与音频入口", "接入协议与输入格式；并非所有终端都具备相同能力", PipelineAnchor.Start,
            "audio", IconToken.Microphone, configPaths = listOf("audio", "echomuse"),
            tests = listOf(NodeTestDescriptor("input.profile", "输入能力验证")),
        ),
        FlowFeatureDescriptor(
            "wakeup", "唤醒与问候", "唤醒事件旁路；固定问候可直接进入语音输出", PipelineAnchor.Before(VoiceStageId.Vad),
            "wakeup", IconToken.Waveform, "wakeup", listOf("wakeup"),
            listOf(NodeTestDescriptor("wakeup.preview", "问候配置验证")),
        ),
        FlowFeatureDescriptor(
            "command", "会话指令", "对结束等固定指令做只读匹配预览，生产执行仍遵循原技能门控", PipelineAnchor.Before(VoiceStageId.Nlu),
            "command", IconToken.Intent, "command", listOf("command"),
            listOf(NodeTestDescriptor("command.match", "指令匹配验证")),
        ),
        FlowFeatureDescriptor(
            "soul", "人格与提示词", "在调用 LLM 前组装上下文；AIUI 托管模式不使用本地增强", PipelineAnchor.Before(VoiceStageId.Llm),
            "soul", IconToken.Text, "soul", listOf("soul"),
            listOf(NodeTestDescriptor("soul.preview", "提示词预览")), order = 10,
        ),
        FlowFeatureDescriptor(
            "memory.recall", "记忆召回", "只读召回当前作用域的上下文", PipelineAnchor.Before(VoiceStageId.Llm),
            "memory", IconToken.Model, "memory", listOf("memory"),
            listOf(NodeTestDescriptor("memory.recall", "召回验证")), order = 20,
        ),
        FlowFeatureDescriptor(
            "output", "终端输出适配", "编码、分帧、缓冲与播放结束按终端能力适配", PipelineAnchor.After(VoiceStageId.Tts),
            "audio", IconToken.Speaker, configPaths = listOf("audio", "playback"),
            tests = listOf(NodeTestDescriptor("output.profile", "输出能力验证")),
        ),
        FlowFeatureDescriptor(
            "memory.writeback", "轮次结束与记忆写回", "结束阶段策略；测试默认 dry-run，不写入真实记忆", PipelineAnchor.End,
            "memory", IconToken.Save, "memory", listOf("memory"),
            listOf(NodeTestDescriptor("memory.writeback.dry_run", "写回计划验证")),
        ),
    )

    val all: List<PipelineFlowNode> = build(features)

    fun build(features: List<FlowFeatureDescriptor>): List<PipelineFlowNode> {
        val sorted = features.sortedWith(compareBy<FlowFeatureDescriptor> { it.order }.thenBy { it.id })
        fun attached(anchor: PipelineAnchor): List<PipelineFlowNode> = sorted.filter { it.anchor == anchor }.map {
            PipelineFlowNode(it.id, it.label, it.summary, it.sectionId, it.icon, it.capabilityId, it.tests, anchor = it.anchor)
        }
        val nodes = buildList {
            addAll(attached(PipelineAnchor.Start))
            for (panel in PipelinePanels.all) {
                addAll(attached(PipelineAnchor.Before(panel.stage)))
                add(PipelineFlowNode(panel.sectionId, panel.label, panel.summary, panel.sectionId, panel.icon, panel.capabilityId, panel.tests, coreStage = panel.stage))
                addAll(attached(PipelineAnchor.After(panel.stage)))
            }
            addAll(attached(PipelineAnchor.End))
        }
        require(nodes.map { it.id }.distinct().size == nodes.size) { "Flow node IDs must be unique" }
        return nodes
    }

    fun forId(nodeId: String): PipelineFlowNode? = all.firstOrNull { it.id == nodeId }
    fun primaryForSection(sectionId: String): PipelineFlowNode? =
        all.firstOrNull { it.id == sectionId && it.sectionId == sectionId } ?: all.firstOrNull { it.sectionId == sectionId }
    fun selectionForNode(nodeId: String): ConfigSelection? = forId(nodeId)?.selection

    fun tabFor(selection: ConfigSelection): String =
        if (PipelineWorkflow.resolve(selection) != null) "voice"
        else ConfigSections.tabOf(selection.sectionId)
}
