package com.xiaozhi.admin

import com.xiaozhi.admin.contracts.*

/** Stable display order, independent of the server capability-array order. */
enum class VoiceStageId(val id: String) { Vad("vad"), Asr("asr"), Nlu("nlu"), Llm("llm"), Tts("tts") }

enum class ClientTestCapability { MicrophoneCapture, NativePlayback, NativeOpus }

data class NodeTestDescriptor(
    val id: String,
    val label: String,
    val requiredClientCapabilities: Set<ClientTestCapability> = emptySet(),
) {
    fun visibleTo(capabilities: Set<ClientTestCapability>): Boolean =
        capabilities.containsAll(requiredClientCapabilities)

    fun availability(
        server: ServerTestCapability?,
        clientCapabilities: Set<ClientTestCapability> = emptySet(),
        clientReadiness: Availability = Availability.Enabled,
    ): Availability = when {
        !visibleTo(clientCapabilities) -> Availability.Disabled("当前客户端不支持此测试")
        clientReadiness is Availability.Disabled -> clientReadiness
        server == null -> Availability.Disabled("服务端未声明支持此测试")
        server.actionId != id -> Availability.Disabled("测试能力标识不匹配")
        !server.available -> Availability.Disabled(server.reason.ifBlank { "服务端测试暂不可用" })
        else -> Availability.Enabled
    }
}

/** Only server-reported readiness may enable a node test; a client platform is not evidence. */
data class ServerTestCapability(val actionId: String, val available: Boolean, val reason: String = "")

enum class PanelRendererKey { Vad, Asr, Nlu, Llm, Tts }

data class NodePanelDescriptor(
    val stage: VoiceStageId,
    val label: String,
    val summary: String,
    val icon: IconToken,
    val renderer: PanelRendererKey,
    val relatedSections: List<String>,
    val configPaths: List<String>,
    val tests: List<NodeTestDescriptor>,
) {
    val sectionId: String get() = stage.id
    val capabilityId: String get() = stage.id
}

/** Compiled panel bindings. No downloaded code, captured Pager, or network calls. */
object PipelinePanels {
    val all: List<NodePanelDescriptor> = listOf(
        NodePanelDescriptor(
            VoiceStageId.Vad, "VAD · 语音分段", "检测语音起止，输出完整语音段", IconToken.Waveform,
            PanelRendererKey.Vad, emptyList(), listOf("vad"),
            listOf(NodeTestDescriptor("vad.sample", "样本分段")),
        ),
        NodePanelDescriptor(
            VoiceStageId.Asr, "ASR · 语音识别", "将语音段识别为文本", IconToken.Microphone,
            PanelRendererKey.Asr, emptyList(), listOf("asr"),
            listOf(
                NodeTestDescriptor("asr.sample", "样本识别"),
                NodeTestDescriptor("asr.recording", "本机录音识别", setOf(ClientTestCapability.MicrophoneCapture, ClientTestCapability.NativeOpus)),
            ),
        ),
        NodePanelDescriptor(
            VoiceStageId.Nlu, "NLU · 意图与技能", "规则与轻量模型提名技能，命中后可跳过 LLM", IconToken.Intent,
            PanelRendererKey.Nlu, listOf("command"), listOf("nlu", "command"),
            listOf(NodeTestDescriptor("nlu.classify", "文本意图测试")),
        ),
        NodePanelDescriptor(
            VoiceStageId.Llm, "LLM · 对话生成", "未命中本地技能时生成回复，上下文增强单独配置", IconToken.Model,
            PanelRendererKey.Llm, listOf("soul", "memory"), listOf("llm", "soul", "memory"),
            listOf(NodeTestDescriptor("llm.chat", "节点调用测试")),
        ),
        NodePanelDescriptor(
            VoiceStageId.Tts, "TTS · 语音合成", "流式合成 PCM，由终端适配器编码与播放", IconToken.Speaker,
            PanelRendererKey.Tts, emptyList(), listOf("tts"),
            listOf(
                NodeTestDescriptor("tts.synthesis", "服务端合成测试"),
                NodeTestDescriptor("tts.playback", "本机播放诊断", setOf(ClientTestCapability.NativePlayback, ClientTestCapability.NativeOpus)),
            ),
        ),
    )

    val flowLabel: String = all.joinToString(" → ") { it.stage.id.uppercase() }

    fun forSection(sectionId: String): NodePanelDescriptor? =
        all.firstOrNull { it.sectionId == sectionId || sectionId in it.relatedSections }

    init {
        require(all.map { it.stage } == VoiceStageId.entries.toList())
        require(all.map { it.renderer }.distinct().size == all.size)
        val sections = all.flatMap { listOf(it.sectionId) + it.relatedSections }
        require(sections.distinct().size == sections.size)
        val tests = all.flatMap { panel -> panel.tests.map { it.id } }
        require(tests.distinct().size == tests.size)
    }
}
