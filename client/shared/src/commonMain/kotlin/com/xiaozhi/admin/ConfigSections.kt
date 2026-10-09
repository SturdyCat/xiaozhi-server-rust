package com.xiaozhi.admin

/**
 * 配置页的 **section 常量表**（`docs/plugin-architecture-unification.md` §5.2/§5.7）。
 *
 * 借 DSH 的"声明 + 有序注册表 + 加载期校验"思想，但**不模拟运行时槽位**：
 * 本项目 Kuikly 侧没有运行时 UI 插件加载，因此这里就是一张编译期常量表——
 * 集中定义 id / order / label，构建期校验 id 唯一与 order 单调。
 *
 * 收益（§5.9）：新增或调整一个 tab 从"改 `renderForm` 里的一行 + 记住顺序 + 别漏文案"
 * 收敛为"改这张表 + 在 `renderForm` 的 `when` 里加一条分支"；`id` 同时是稳定标识
 * （可用于定位/埋点，不随 label 文案变化）。
 *
 * ⚠️ 页面内容仍写在 `ConfigCards.renderForm` 的 `when` 里（而不是本文件返回 lambda）：
 * Kuikly 的 `ViewBuilder` 接收者由**调用点**的期望类型推断，把 lambda 装进 `when` 再返回
 * 会让接收者丢失（实测编译报 "receiver type mismatch"）。表与内容分开也正好符合
 * "表声明 + 渲染"的分工。
 */
class SectionDescriptor(val id: String, val order: Int, val label: String)

object ConfigSections {

    /** 全部 section（order 升序即展示顺序）。 */
    val all: List<SectionDescriptor> = listOf(
        // order 以 10 为步长：留出插入空间，避免每次新增都要重排既有值
        SectionDescriptor("overview", 0, "总览"),
        SectionDescriptor("server", 10, "Server"),
        SectionDescriptor("audio", 20, "Audio"),
        SectionDescriptor("asr", 30, "ASR"),
        SectionDescriptor("vad", 40, "VAD"),
        SectionDescriptor("tts", 50, "TTS"),
        SectionDescriptor("llm", 60, "LLM"),
        // 指令闸门：ASR → LLM 之间的拦截（退下/闭嘴/关闭 → 断开会话）
        SectionDescriptor("command", 65, "指令"),
        SectionDescriptor("aiui", 70, "AIUI"),
        SectionDescriptor("soul", 80, "灵魂"),
        SectionDescriptor("memory", 90, "记忆"),
    )

    init {
        val ids = all.map { it.id }
        require(ids.distinct().size == ids.size) { "配置 section id 必须唯一：$ids" }
        val orders = all.map { it.order }
        require(orders == orders.sorted()) { "配置 section 必须按 order 升序声明：$orders" }
        require(ids.all { it.isNotBlank() } && all.all { it.label.isNotBlank() }) {
            "配置 section 的 id/label 不得为空"
        }
    }
}
