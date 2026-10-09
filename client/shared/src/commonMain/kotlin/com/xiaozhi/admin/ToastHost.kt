package com.xiaozhi.admin

import com.tencent.kuikly.core.base.Border
import com.tencent.kuikly.core.base.BorderStyle
import com.tencent.kuikly.core.base.BoxShadow
import com.tencent.kuikly.core.base.Color
import com.tencent.kuikly.core.base.ViewContainer
import com.tencent.kuikly.core.directives.vif
import com.tencent.kuikly.core.pager.Pager
import com.tencent.kuikly.core.reactive.handler.observable
import com.tencent.kuikly.core.timer.setTimeout
import com.tencent.kuikly.core.views.Text
import com.tencent.kuikly.core.views.View

/**
 * 顶部 toast 数据：level 决定图标与描边——ok=绿（成功）/error=红（失败）/info=中性（提示）。
 *
 * 跨端统一（commonMain）：macOS 壳（AdminShell）与 web 配置页（ConfigPage）共用同一套
 * 状态、配色与渲染，不再各写一套（此前 macOS 在壳内、web 在 DOM 里各实现一次）。
 */
data class ToastData(val msg: String, val level: String = "info")

/** toast 停留时长（毫秒）：到点自动消失，不需要用户关闭。 */
private const val TOAST_HOLD_MS = 3000

/** 浮动 toast 默认顶距（pt）：避开 macOS 窗口红黄绿按钮所在的行，又保持"贴在顶部"的观感。 */
private const val TOAST_TOP_OFFSET = 44f

/** toast 文案最大宽度（pt）：超过自动折行，避免长服务端文案把浮层拉成通栏。 */
private const val TOAST_TEXT_MAX_WIDTH = 420f

/** toast 文案最多行数：再多则省略号收尾，浮层始终只占顶部一小块。 */
private const val TOAST_TEXT_MAX_LINES = 3

/**
 * 顶部 toast 状态（跨端）：持有当前 toast 与自动消失定时器。
 * 由 Pager 创建（val toast = ToastState(this)），经 [ToastHost] 渲染。
 */
class ToastState(private val scope: Pager) {
    var current by scope.observable<ToastData?>(null)
    private var seq = 0

    /** 顶部弹出一条提示并在 [TOAST_HOLD_MS] 后自动消失；连续调用以最新一条为准（旧定时器失效，不会误清最新 toast）。 */
    fun show(message: String, level: String = "info") {
        val s = ++seq
        current = ToastData(message, level)
        scope.setTimeout(TOAST_HOLD_MS) { if (seq == s) current = null }
    }
}

/**
 * 顶部**浮动** toast（跨端）：**绝对定位**覆盖在页面之上，不占布局（不会把下方内容推下去），
 * 水平居中、距页面顶部 [topOffset]；内容 = 前置状态图标（✓ 成功 / ✕ 失败 / i 提示）+ 文案。
 *
 * 结构：外层承接 View 绝对定位并左右拉伸到整页宽度（只负责居中定位），
 * 内层卡片是真正的浮层（深色底 + 语义色描边 + 投影），随内容自适应宽高。
 *
 * 用法：在**页面根容器**闭包内非限定调用（接收者=根容器）——放在 body 最后，
 * 才能盖在标题栏/内容之上，且水平居中相对整个窗口而不是某个内容列。
 * ⚠️ 状态在 attr 闭包内实时读取（Kuikly 只跟踪闭包内的 observable 读取）：
 * 连续两条 toast 替换时（vif 条件恒为 true、不重建），文案/配色仍会就地刷新。
 */
fun ViewContainer<*, *>.ToastHost(state: ToastState, topOffset: Float = TOAST_TOP_OFFSET) {
    vif({ state.current != null }) {
        // 承接层：绝对定位 + 左右拉伸 → 完全脱离布局；alignItemsCenter 使卡片水平居中
        View {
            attr {
                absolutePosition(top = topOffset, left = 0f, right = 0f)
                alignItemsCenter()
                zIndex(10)
            }
            // 浮层卡片：深色胶囊，左侧状态图标 + 右侧文案
            View {
                attr {
                    flexDirectionRow()
                    alignItemsCenter()
                    paddingLeft(AdminSpace.md)
                    paddingRight(AdminSpace.lg)
                    paddingTop(AdminSpace.sm)
                    paddingBottom(AdminSpace.sm)
                    borderRadius(AdminShape.radiusLg)
                    backgroundColor(AdminColors.toastBg)
                    border(Border(1f, BorderStyle.SOLID, toastStroke(state.current?.level ?: "info")))
                    boxShadow(BoxShadow(0f, 8f, 24f, AdminColors.shadowSm))
                }
                // 前置图标：实心语义色圆 + 白色对勾/叉（成功失败一眼可辨；info 用 i）
                View {
                    attr {
                        width(20f)
                        height(20f)
                        borderRadius(AdminShape.radiusPill)
                        allCenter()
                        marginRight(AdminSpace.xs)
                        backgroundColor(toastAccent(state.current?.level ?: "info"))
                    }
                    Text {
                        attr {
                            fontSize(AdminType.micro)
                            fontWeightMedium()
                            color(AdminColors.textOnAccent)
                            text(toastGlyph(state.current?.level ?: "info"))
                        }
                    }
                }
                // 跟随内容：长文案自动折行（上限 [TOAST_TEXT_MAX_LINES] 行）
                Text {
                    attr {
                        maxWidth(TOAST_TEXT_MAX_WIDTH)
                        lines(TOAST_TEXT_MAX_LINES)
                        fontSize(AdminType.body)
                        fontWeightMedium()
                        color(AdminColors.textPrimary)
                        text(state.current?.msg ?: "")
                    }
                }
            }
        }
    }
}

/** 图标底色：成功=品牌绿 / 失败=危险红 / 提示=中性灰。 */
private fun toastAccent(level: String): Color = when (level) {
    "ok" -> AdminColors.accent
    "error" -> AdminColors.danger
    else -> AdminColors.textTertiary
}

/** 浮层描边：同图标语义色（提示态用常规分隔色）。 */
private fun toastStroke(level: String): Color = when (level) {
    "ok" -> AdminColors.accent
    "error" -> AdminColors.danger
    else -> AdminColors.dividerStrong
}

/** 图标字形：✓ 成功 / ✕ 失败 / i 提示（纯文本字形，无图片资源依赖，跨端一致）。 */
private fun toastGlyph(level: String): String = when (level) {
    "ok" -> "✓"
    "error" -> "✕"
    else -> "i"
}
