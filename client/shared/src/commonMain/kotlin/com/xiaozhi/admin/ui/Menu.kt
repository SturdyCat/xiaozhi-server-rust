package com.xiaozhi.admin.ui

import com.tencent.kuikly.core.base.CursorType
import com.tencent.kuikly.core.base.ViewBuilder
import com.tencent.kuikly.core.base.ViewContainer
import com.tencent.kuikly.core.directives.vif
import com.tencent.kuikly.core.views.Text
import com.tencent.kuikly.core.views.View

/**
 * **菜单项**：主壳侧栏、配置工作区左菜单、卡片内菜单（EchoMuse 的「概览/WiFi/设备管理」）
 * 的同一实现，基于官方 `View` + `Text` 组合（图标走 [appIcon]，不做位图/自绘文字）。
 *
 * 三档样式覆盖现有全部用法，参数差异只在尺寸与强调程度，不再是三份手工抄写的 View：
 *
 * | style | 用在哪 | 尺寸 | 文字 |
 * |---|---|---|---|
 * | [MenuItemStyle.MAIN] | 主壳侧栏 | 44 高、左右留 12、3px 选中条 | body/500 |
 * | [MenuItemStyle.WORKSPACE] | 配置工作区左菜单 | 最小 44 高、左右留 4 | caption/500，窄窗隐藏图标 |
 * | [MenuItemStyle.CARD] | 卡片内左菜单 | 36 高、左右留 12 | body |
 *
 * ⚠️ `isSelected`/`showIcon` 必须是构建期可求值的值或 lambda：
 * 选中态在 attr 闭包内求值（Kuikly 只跟踪闭包内的 observable 读取），
 * 传一次性 Boolean 会导致「点击后选中态不刷新」。
 */
fun ViewContainer<*, *>.menuListItem(
    label: String,
    isSelected: () -> Boolean,
    onClick: () -> Unit,
    icon: IconToken? = null,
    style: MenuItemStyle = MenuItemStyle.MAIN,
    enabled: Boolean = true,
    showIcon: Boolean = true,
    fontSize: Float = style.spec.fontSize,
    trailing: ViewBuilder? = null,
) {
    val spec = style.spec
    View {
        attr {
            if (spec.fixedHeight > 0f) height(spec.fixedHeight) else minHeight(spec.minHeight)
            flexDirectionRow()
            alignItemsCenter()
            marginLeft(spec.marginH)
            marginRight(spec.marginH)
            if (spec.paddingAll > 0f) padding(spec.paddingAll) else {
                paddingLeft(spec.paddingH)
                paddingRight(spec.paddingH)
            }
            borderRadius(spec.radius)
            backgroundColor(if (isSelected()) AdminColors.sidebarSelectedBg else AdminColors.transparent)
            // 菜单是点击目标：悬停指针（web 由 h5App 的 cursor 处理器落 CSS）
            cursor(CursorType.POINTER)
            accessibility("$label${if (isSelected()) "，已选中" else ""}")
        }
        event { click { if (enabled) onClick() } }
        // 主侧栏的 3px 选中指示条（其余档位不画，避免卡片/工作区里出现第二条竖线）
        if (spec.selectedBar) {
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
        }
        if (icon != null && showIcon) {
            appIcon(
                { icon },
                size = spec.iconSize,
                tint = {
                    when {
                        !enabled -> AdminColors.disabledText
                        isSelected() -> AdminColors.accentTintText
                        else -> AdminColors.textTertiary
                    }
                },
            )
            View { attr { width(spec.iconGap) } }
        }
        Text {
            attr {
                flex(1f)
                text(label)
                lines(3)
                fontSize(fontSize)
                if (spec.bold) fontWeightMedium()
                color(
                    when {
                        !enabled -> AdminColors.textTertiary
                        isSelected() -> AdminColors.accentTintText
                        else -> spec.restColor
                    },
                )
                touchEnable(false)
            }
        }
        if (trailing != null) trailing(this)
    }
}

/** [menuListItem] 的三档样式。 */
enum class MenuItemStyle {
    /** 主壳侧栏：44 高、3px 选中条、body/500。 */
    MAIN,

    /** 配置工作区左菜单：最小 44 高（菜单项可折行）、caption/500、窄窗可隐藏图标。 */
    WORKSPACE,

    /** 卡片内菜单：36 高、body、无选中条。 */
    CARD,
    ;

    internal val spec: MenuItemSpec
        get() = when (this) {
            MAIN -> MenuItemSpec(
                fixedHeight = 44f, minHeight = 44f, marginH = AdminSpace.sm,
                paddingH = AdminSpace.md, paddingAll = 0f, radius = AdminShape.radiusMd,
                iconSize = 18f, iconGap = AdminSpace.sm, fontSize = AdminType.body, bold = true,
                restColor = AdminColors.textPrimary, selectedBar = true,
            )
            WORKSPACE -> MenuItemSpec(
                fixedHeight = 0f, minHeight = 44f, marginH = 4f,
                paddingH = AdminSpace.xs, paddingAll = AdminSpace.xs, radius = AdminShape.radiusSm,
                iconSize = 16f, iconGap = AdminSpace.xs, fontSize = AdminType.caption, bold = true,
                restColor = AdminColors.textPrimary, selectedBar = false,
            )
            CARD -> MenuItemSpec(
                fixedHeight = 36f, minHeight = 36f, marginH = 4f,
                paddingH = AdminSpace.sm, paddingAll = 0f, radius = AdminShape.radiusSm,
                iconSize = 16f, iconGap = AdminSpace.xs, fontSize = AdminType.body, bold = false,
                restColor = AdminColors.textSecondary, selectedBar = false,
            )
        }
}

/** 档位对应的具体尺寸/字号（内部数据，避免调用方各写一套数字）。 */
internal data class MenuItemSpec(
    val fixedHeight: Float,
    val minHeight: Float,
    val marginH: Float,
    val paddingH: Float,
    val paddingAll: Float,
    val radius: Float,
    val iconSize: Float,
    val iconGap: Float,
    val fontSize: Float,
    val bold: Boolean,
    val restColor: com.tencent.kuikly.core.base.Color,
    val selectedBar: Boolean,
)
