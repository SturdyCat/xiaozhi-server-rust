package com.xiaozhi.admin

/**
 * 配置页的 **section 常量表**（docs/plugin-architecture-unification.md §5.2/§5.7）。
 *
 * 借 DSH 的"声明 + 有序注册表 + 加载期校验"思想，但**不模拟运行时槽位**：
 * 本项目 Kuikly 侧没有运行时 UI 插件加载，因此这里就是一张编译期常量表——
 * 集中定义 id / order / label / group / keywords，构建期校验 id 唯一与 order 单调。
 *
 * 与「全部配置塞进 14 个平铺 tab」相比，本表额外承载**分组与搜索**信息：
 * - `group`：把 14 个 section 收拢为三个语义分组（服务与接入 / 语音与对话 / 智能体），
 *   配置页据此渲染「左侧分组导航 + 右侧单 section」两栏布局（见 `renderConfigLayout`），
 *   不再让用户面对一长条 tab 而无从下手；
 * - `keywords`：为搜索框提供「中文意图 → section」的映射（如搜"音色"命中 TTS、
 *   搜"固件"命中 OTA），弥补单纯按 label 匹配覆盖不到的口语化查询。
 *
 * ⚠️ 页面内容仍写在 `ConfigCards.renderConfigSection` 的 `when` 里（而不是本文件返回 lambda）：
 * Kuikly 的 `ViewBuilder` 接收者由**调用点**的期望类型推断，把 lambda 装进 `when` 再返回
 * 会让接收者丢失（实测编译报 "receiver type mismatch"）。表与内容分开也正好符合
 * "表声明 + 渲染"的分工。
 */
class SectionDescriptor(
    val id: String,
    val order: Int,
    val label: String,
    /** 所属分组（见 [ConfigSections.groups]）；null = 置顶项（如总览）。 */
    val group: String? = null,
    /** 搜索关键词（口语化 / 中文意图 → 该 section），用于配置页搜索框。 */
    val keywords: List<String> = emptyList(),
)

object ConfigSections {

    /** 分组（order 升序即展示顺序）：id → 中文标题。 */
    val groups: List<Pair<String, String>> = listOf(
        // order 以 10 为步长：留出插入空间，避免每次新增都要重排既有值
        "service" to "服务与接入",
        "voice" to "语音与对话",
        "agent" to "智能体",
    )

    /**
     * 全部 section，**按展示顺序声明**：先置顶项，再按分组顺序，组内保持逻辑顺序。
     * `order` 仅用于构建期单调校验，展示顺序由本表声明顺序决定（与分组一致）。
     */
    val all: List<SectionDescriptor> = listOf(
        SectionDescriptor("overview", 0, "总览", null, listOf("总览", "能力", "插件", "overview")),
        // 预设（P1-5）：置顶项。快照/应用全局配置；设备个性化模式下仅显示不可用提示。
        SectionDescriptor(
            "presets", 5, "预设", null,
            listOf("预设", "快照", "备份", "回滚", "preset", "profile"),
        ),
        // —— 服务与接入 ——
        SectionDescriptor(
            "server", 10, "Server", "service",
            listOf("端口", "地址", "token", "令牌", "服务", "server", "worker", "后台"),
        ),
        SectionDescriptor(
            "aiui", 20, "AIUI 全链路", "service",
            listOf("全链路", "讯飞", "aiui", "极速超拟人", "云端闭环"),
        ),
        SectionDescriptor("mcp", 30, "MCP 工具", "service", listOf("工具", "mcp", "插件", "能力")),
        SectionDescriptor("ota", 40, "OTA", "service", listOf("固件", "升级", "ota", "firmware", "设备")),
        // —— 语音与对话（VAD→ASR→LLM→TTS 流水线）——
        SectionDescriptor(
            "audio", 50, "Audio", "voice",
            listOf("采样率", "帧长", "缓冲", "下行", "audio", "音频", "抖动"),
        ),
        SectionDescriptor("asr", 60, "ASR", "voice", listOf("识别", "语音识别", "sensevoice", "asr")),
        SectionDescriptor("vad", 70, "VAD", "voice", listOf("静音", "语音端点", "vad", "切段")),
        SectionDescriptor(
            "llm", 80, "LLM", "voice",
            listOf("大模型", "模型", "提示词", "prompt", "llm", "system", "对话"),
        ),
        SectionDescriptor("tts", 90, "TTS", "voice", listOf("语音合成", "音色", "tts", "合成", "发音人")),
        SectionDescriptor("command", 100, "指令", "voice", listOf("指令", "退下", "闭嘴", "command", "拦截")),
        SectionDescriptor(
            "nlu", 105, "技能", "voice",
            listOf("技能", "意图", "槽位", "模板", "级联", "nlu", "skill"),
        ),
        SectionDescriptor("wakeup", 110, "唤醒", "voice", listOf("唤醒", "问候", "wakeup", "我在")),
        // —— 智能体（上下文生产者）——
        SectionDescriptor("soul", 120, "灵魂", "agent", listOf("人格", "灵魂", "人设", "soul", "角色")),
        SectionDescriptor("memory", 130, "记忆", "agent", listOf("记忆", "图记忆", "memory", "上下文")),
    )

    /** 置顶项（group == null），按声明顺序。 */
    val pinned: List<SectionDescriptor> get() = all.filter { it.group == null }

    /** 某分组下的 section，按声明顺序。 */
    fun sectionsIn(group: String): List<SectionDescriptor> = all.filter { it.group == group }

    /**
     * 按关键词（label / keywords / 分组标题，不区分大小写）过滤 section。
     * 空查询返回全部（用于"未搜索"态）。搜索命中口语化意图：如"音色"→TTS、"固件"→OTA、"识别"→ASR。
     */
    fun search(query: String): List<SectionDescriptor> {
        val q = query.trim().lowercase()
        if (q.isEmpty()) return all
        val groupLabels = groups.associate { it.first to it.second.lowercase() }
        return all.filter { d ->
            d.label.lowercase().contains(q) ||
                d.keywords.any { it.lowercase().contains(q) } ||
                (d.group?.let { groupLabels[it]?.contains(q) } ?: false)
        }
    }

    init {
        val ids = all.map { it.id }
        require(ids.distinct().size == ids.size) { "配置 section id 必须唯一：$ids" }
        val orders = all.map { it.order }
        require(orders == orders.sorted()) { "配置 section 必须按 order 升序声明：$orders" }
        require(ids.all { it.isNotBlank() } && all.all { it.label.isNotBlank() }) {
            "配置 section 的 id/label 不得为空"
        }
        val groupIds = groups.map { it.first }.toSet()
        require(all.none { it.group != null && it.group !in groupIds }) {
            "配置 section 的 group(${all.mapNotNull { it.group }}) 必须先在 groups 里声明"
        }
    }
}
