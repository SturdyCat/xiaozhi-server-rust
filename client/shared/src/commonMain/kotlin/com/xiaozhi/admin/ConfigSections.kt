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

    /**
     * 「全局配置」菜单包含的 section：**没有流程节点**的那些。
     *
     * 划分口径（2026-10 菜单拆分）：有流程节点的 section（VAD/ASR/NLU/LLM/TTS 与挂在
     * 连接点上的唤醒、指令、人格、记忆、音频入口/输出）归「语音流程」，按节点修改；
     * 其余（能力总览 / 预设 / AIUI 全链路 / 服务 / EchoMuse 接入 / MCP / OTA）在流程里
     * 没有位置，集中到侧边栏独立菜单「全局配置」，不再混进工作流的左菜单。
     *
     * 判据取 [PipelineFlow.primaryForSection]（而不是 group 名）：只要在流程里有节点，
     * 就一定能从「语音流程」到达，避免同一 section 出现在两套导航里对不上。
     */
    val global: List<SectionDescriptor> get() = all.filter { PipelineFlow.primaryForSection(it.id) == null }

    fun isGlobal(sectionId: String): Boolean = global.any { it.id == sectionId }

    /** 全局配置左菜单的分组（总览 / 上下文增强 / 服务与接入；空组不出现）。 */
    val globalGroups: List<Pair<String, List<SectionDescriptor>>>
        get() {
            val ids = listOf<String?>(null) + groups.map { it.first }
            val labels = listOf("总览") + groups.map { it.second }
            return ids.zip(labels)
                .map { (id, label) -> label to global.filter { it.group == id } }
                .filter { it.second.isNotEmpty() }
        }

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
