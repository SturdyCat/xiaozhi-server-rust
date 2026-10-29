package com.xiaozhi.admin.contracts

/**
 * 管理台**导航常量表**（纯数据契约，零 UI 依赖）。
 *
 * ## 为什么在 contracts 而不是 `ui/`
 *
 * 原实现与 DSL 侧边栏原语（`sidebarItem`）同住 `ui/Nav.kt`。T06 起壳层是 Compose
 * （`AdminShell.kt`），T12 会整体删除 `ui/`——若导航表留在 `ui/`，删 `ui/` 时壳层会
 * 一起断掉。导航是"有哪些页面"的业务事实，与渲染无关，属于契约层。
 *
 * ## 为什么是常量表而不是运行时槽位
 *
 * 与 `ConfigSections` 同一取舍（见 `docs/plugin-architecture-unification.md` §5.7）：
 * 本项目的 Kuikly 侧**没有运行时 UI 插件加载**，所以这里就是一张编译期常量表，
 * 集中定义 id/order/label，构建期校验 id 唯一与 order 升序。收益是新增一个页面从
 * "改 `renderXxx` 里的一行 + 记住顺序 + 别漏文案"收敛为"表加一行 + `when` 加一条分支"。
 */
class NavDescriptor(
    val id: String,
    val order: Int,
    val label: String,
    val group: String,
    /** true = 仅 macOS 客户端显示（如测试台，依赖原生麦克风/音频链路）；web 不渲染该菜单。 */
    val macOnly: Boolean = false,
)

/** 管理台一级导航（order 升序即展示顺序）。 */
object AdminNav {

    /** 侧边栏分组标题（P1-4）：项数多了以后按"观测 / 配置 / 存储"分段，渲染时 group 变化处插标题。 */
    const val GROUP_OBSERVE = "观测"
    const val GROUP_CONFIG = "配置"
    const val GROUP_STORAGE = "存储"

    val all: List<NavDescriptor> = listOf(
        // order 以 10 为步长：留出插入空间，避免每次新增都要重排既有值
        NavDescriptor("overview", 0, "概览", GROUP_OBSERVE),
        // 测试台（P1-3）：macOnly —— web 端彻底隐藏该菜单（麦克风/音频链路不可用），真实现只在 macOS 客户端
        NavDescriptor("testbench", 5, "测试台", GROUP_OBSERVE, macOnly = true),
        NavDescriptor("devices", 10, "设备", GROUP_OBSERVE),
        // EchoMuse 设备（批准制登记 + 重启 / WiFi 下发指令，`GET /api/echomuse/devices`）
        NavDescriptor("echomuse", 15, "EchoMuse", GROUP_OBSERVE),
        // 控制台（官方 dashboard 的 CONSOLE / 命令行）：实时服务器日志流 GET /api/logs，
        // 服务端已有实现、此前客户端未建页面消费（见 ConsoleState / ConsolePage）。
        // （2026-10-29 范围削减：sessions/conversations/usage/metrics 四项已移除，
        //  见 docs/plans/gearui-kit-scope-reduction.md §二；order 保持原步长不重排，
        //  以免无谓地动 EchoMuse 之外各页面的声明位置。）
        NavDescriptor("console", 47, "控制台", GROUP_OBSERVE),
        // 语音流程：只装流程节点（VAD/ASR/NLU/LLM/TTS 与挂在连接点上的分支），配置/测试双 Tab
        NavDescriptor("config", 50, "语音流程", GROUP_CONFIG),
        // 全局配置：没有流程节点的 section（能力总览 / 预设 / AIUI / 服务 / EchoMuse 接入 / MCP / OTA）。
        // 独立成一级菜单（2026-10）：此前它藏在工作流右上的「全局设置」里，和流程节点挤同一个左菜单，
        // 用户分不清"改某个阶段"和"改整套部署"的区别。
        NavDescriptor("settings", 55, "全局配置", GROUP_CONFIG),
        // 文件（服务端存储目录：新建/上传/删除；EchoMuse 自定义唤醒词模型放 oww-models/）
        NavDescriptor("files", 60, "文件", GROUP_STORAGE),
    )

    /** 默认进入主流程；设备、运行观测与基础设施仍保留稳定导航 ID。 */
    const val DEFAULT = "config"

    init {
        val ids = all.map { it.id }
        require(ids.distinct().size == ids.size) { "导航 id 必须唯一：$ids" }
        val orders = all.map { it.order }
        require(orders == orders.sorted()) { "导航必须按 order 升序声明：$orders" }
        require(ids.all { it.isNotBlank() } && all.all { it.label.isNotBlank() && it.group.isNotBlank() }) {
            "导航的 id/label/group 不得为空"
        }
        require(ids.contains(DEFAULT)) { "默认落地页 $DEFAULT 必须在导航表里" }
        // 分组相邻性：同 group 的项在 order 升序下必须连续，否则渲染时同组会被拆成多段
        val segments = all.zipWithNext().count { it.first.group != it.second.group } + 1
        require(segments == all.map { it.group }.distinct().size) {
            "同分组的导航项必须相邻声明：${all.map { "${it.id}:${it.group}" }}"
        }
    }

    /** id → 文案（找不到时回退 id 本身，避免标题变成空白）。 */
    fun label(id: String): String = all.firstOrNull { it.id == id }?.label ?: id

    /**
     * 按平台过滤后的导航表：macOnly 项只在 macOS 客户端出现。
     * 侧边栏渲染一律走本函数（不要直接遍历 [all]），保证 web 端彻底看不到 mac 专属入口。
     */
    fun visible(macos: Boolean): List<NavDescriptor> = all.filter { !it.macOnly || macos }

    /**
     * 按分组切段（给 Compose 壳层的 `CellGroup` 用）：保留 [all] 的声明顺序，
     * 依赖 `init` 里已断言的"同组必须相邻"。
     */
    fun grouped(macos: Boolean): List<Pair<String, List<NavDescriptor>>> {
        val out = mutableListOf<Pair<String, MutableList<NavDescriptor>>>()
        for (item in visible(macos)) {
            val last = out.lastOrNull()
            if (last != null && last.first == item.group) last.second.add(item)
            else out.add(item.group to mutableListOf(item))
        }
        return out.map { it.first to it.second.toList() }
    }
}

/** 连接状态 → 中文（macOS 连接页与 web 顶栏共用）。 */
fun connectionLabel(state: String): String = when (state) {
    "connected" -> "已连接"
    "connecting" -> "连接中"
    "error" -> "连接错误"
    else -> "未连接"
}
