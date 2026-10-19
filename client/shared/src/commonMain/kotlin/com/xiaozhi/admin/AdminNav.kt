package com.xiaozhi.admin

import com.tencent.kuikly.core.base.CursorType
import com.tencent.kuikly.core.base.ViewContainer
import com.tencent.kuikly.core.directives.vif
import com.tencent.kuikly.core.views.Text
import com.tencent.kuikly.core.views.View

/**
 * 管理台**导航常量表**与侧边栏原语（commonMain：web 与 macOS 共用）。
 *
 * ## 为什么单独一个文件
 *
 * `sidebarItem` 原先只定义在 `macosArm64Main/AdminShell.kt`，因此 **web 端根本没有侧边栏**——
 * H5 产物里只有 `@Page("config")` 一个页面（[ConfigPage]），能力上等于"一个配置编辑器"。
 * 把它与导航表下沉到 commonMain 后，web（h5App）与 macOS 可以各自组壳、共用同一套原语。
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

    /** 侧边栏分组标题（P1-4）：项数多了以后按"观测 / 配置"两段分区，渲染时 group 变化处插标题。 */
    const val GROUP_OBSERVE = "观测"
    const val GROUP_CONFIG = "配置"

    val all: List<NavDescriptor> = listOf(
        // order 以 10 为步长：留出插入空间，避免每次新增都要重排既有值
        NavDescriptor("overview", 0, "概览", GROUP_OBSERVE),
        // 测试台（P1-3）：macOnly —— web 端彻底隐藏该菜单（麦克风/音频链路不可用），真实现只在 macOS 客户端
        NavDescriptor("testbench", 5, "测试台", GROUP_OBSERVE, macOnly = true),
        NavDescriptor("devices", 10, "设备", GROUP_OBSERVE),
        // EchoMuse 设备（批准制登记 + 重启 / WiFi 下发指令，`GET /api/echomuse/devices`）
        NavDescriptor("echomuse", 15, "EchoMuse", GROUP_OBSERVE),
        NavDescriptor("sessions", 20, "会话", GROUP_OBSERVE),
        NavDescriptor("conversations", 30, "对话记录", GROUP_OBSERVE),
        NavDescriptor("usage", 40, "用量", GROUP_OBSERVE),
        NavDescriptor("metrics", 45, "延迟", GROUP_OBSERVE),
        NavDescriptor("config", 50, "配置", GROUP_CONFIG),
    )

    /** 默认落地页：概览（打开管理台先看"现在什么状态"，而不是先看一堆配置字段）。 */
    const val DEFAULT = "overview"

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
}

/** 连接状态 → 中文（macOS 连接页与 web 顶栏共用）。 */
fun connectionLabel(state: String): String = when (state) {
    "connected" -> "已连接"
    "connecting" -> "连接中"
    "error" -> "连接错误"
    else -> "未连接"
}

/**
 * 侧边栏项：高 44（触屏目标 ≥44）、左右内距 12、左右外距 8。
 * 选中 = 亮灰胶囊 sidebarSelectedBg + 文字 accentTintText（品牌绿）+ 左侧 3px accent 竖条；
 * 普通 = 透明底 textPrimary(body 17/500)；disabled 置灰 textTertiary 且无点击；
 * showDot() 为 true 时右侧显示小橙点（warning 色，用于「配置」项 dirty 提示）。
 *
 * ⚠️ selected/showDot 必须是 lambda 且只在 attr/vif 闭包内求值：
 * Kuikly 仅对闭包内的 observable 读取做响应式跟踪；构建期传入的一次性 Boolean
 * 在 selectedSection 变化后不会重算（这就是此前「点击菜单无选中效果」的根因）。
 */
fun ViewContainer<*, *>.sidebarItem(
    label: String,
    isSelected: () -> Boolean,
    enabled: Boolean = true,
    showDot: () -> Boolean = { false },
    onClick: () -> Unit,
) {
    View {
        attr {
            height(44f)
            flexDirectionRow()
            alignItemsCenter()
            marginLeft(AdminSpace.sm)
            marginRight(AdminSpace.sm)
            paddingLeft(AdminSpace.md)
            paddingRight(AdminSpace.md)
            borderRadius(AdminShape.radiusMd)
            backgroundColor(if (isSelected()) AdminColors.sidebarSelectedBg else AdminColors.transparent)
            // 桌面端悬停指针：菜单是点击目标，不是文本输入区（web 靠 h5App 的 cursor 处理器落 CSS）
            cursor(CursorType.POINTER)
        }
        event { click { if (enabled) onClick() } }
        // 左侧选中指示条（3px 品牌绿竖条，vif 响应式）
        vif({ isSelected() }) {
            View {
                attr {
                    width(3f)
                    height(20f)
                    borderRadius(AdminShape.radiusPill)
                    backgroundColor(AdminColors.accent)
                    marginRight(AdminSpace.sm)
                }
            }
        }
        Text {
            attr {
                flex(1f)
                fontSize(AdminType.body)
                fontWeightMedium()
                color(
                    if (!enabled) {
                        AdminColors.textTertiary
                    } else if (isSelected()) {
                        AdminColors.accentTintText
                    } else {
                        AdminColors.textPrimary
                    },
                )
                text(label)
            }
        }
        vif({ showDot() }) {
            View {
                attr {
                    width(8f)
                    height(8f)
                    borderRadius(AdminShape.radiusPill)
                    backgroundColor(AdminColors.warning)
                }
            }
        }
    }
}
