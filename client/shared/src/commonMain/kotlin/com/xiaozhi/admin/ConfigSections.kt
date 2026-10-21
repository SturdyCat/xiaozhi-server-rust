package com.xiaozhi.admin

/**
 * Compiled navigation metadata, not executable/downloaded UI plugins.
 * Stable section IDs preserve links and config paths; renderers stay explicit ViewContainer
 * extensions. The voice group follows PipelinePanels, not server capability-array order.
 */
class SectionDescriptor(
    val id: String,
    val order: Int,
    val label: String,
    val group: String? = null,
    val capabilityId: String? = when (id) {
        "aiui" -> "fullchain"
        "soul", "memory", "wakeup", "command" -> id
        else -> PipelinePanels.all.firstOrNull { it.sectionId == id }?.capabilityId
    },
)

object ConfigSections {
    val groups: List<Pair<String, String>> = listOf(
        "voice" to "语音主流程",
        "agent" to "上下文增强",
        "service" to "服务与接入",
    )

    val all: List<SectionDescriptor> = listOf(
        SectionDescriptor("overview", 0, "能力总览"),
        SectionDescriptor("presets", 5, "预设"),
        SectionDescriptor("vad", 10, "VAD · 语音分段", "voice"),
        SectionDescriptor("asr", 20, "ASR · 语音识别", "voice"),
        SectionDescriptor("nlu", 30, "NLU · 意图与技能", "voice"),
        SectionDescriptor("llm", 40, "LLM · 对话生成", "voice"),
        SectionDescriptor("tts", 50, "TTS · 语音合成", "voice"),
        // Branches are separate sections, not additional mandatory pipeline stages.
        SectionDescriptor("command", 60, "结束与指令", "voice"),
        SectionDescriptor("wakeup", 70, "唤醒与问候", "voice"),
        SectionDescriptor("soul", 80, "人格", "agent"),
        SectionDescriptor("memory", 90, "记忆", "agent"),
        SectionDescriptor("aiui", 100, "AIUI 全链路模式", "service"),
        SectionDescriptor("audio", 110, "音频与播放", "service"),
        SectionDescriptor("server", 120, "服务", "service"),
        SectionDescriptor("echomuse", 130, "EchoMuse 接入", "service"),
        SectionDescriptor("mcp", 140, "MCP 工具", "service"),
        SectionDescriptor("ota", 150, "固件与 OTA", "service"),
    )

    val pinned: List<SectionDescriptor> get() = all.filter { it.group == null }
    val tabs: List<Pair<String, String>> = pinned.map { it.id to it.label } + groups

    fun tabOf(sectionId: String): String = all.firstOrNull { it.id == sectionId }?.group ?: sectionId

    fun firstSectionIn(tabId: String): String =
        if (groups.any { it.first == tabId }) sectionsIn(tabId).first().id else tabId

    fun sectionsIn(group: String): List<SectionDescriptor> = all.filter { it.group == group }

    init {
        val ids = all.map { it.id }
        require(ids.distinct().size == ids.size) { "配置 section id 必须唯一：$ids" }
        require(all.map { it.order } == all.map { it.order }.sorted()) { "配置 section 必须按 order 升序声明" }
        require(ids.all { it.isNotBlank() } && all.all { it.label.isNotBlank() })
        val groupIds = groups.map { it.first }.toSet()
        require(all.none { it.group != null && it.group !in groupIds })
        require(sectionsIn("voice").take(5).map { it.id } == PipelinePanels.all.map { it.sectionId })
    }
}
