package com.xiaozhi.admin

import com.tencent.kuikly.core.base.Border
import com.tencent.kuikly.core.base.BorderStyle
import com.tencent.kuikly.core.base.Color
import com.tencent.kuikly.core.base.PagerScope
import com.tencent.kuikly.core.base.ViewBuilder
import com.tencent.kuikly.core.base.ViewContainer
import com.tencent.kuikly.core.base.ViewRef
import com.tencent.kuikly.core.directives.vfor
import com.tencent.kuikly.core.directives.velse
import com.tencent.kuikly.core.directives.vif
import com.tencent.kuikly.core.nvi.serialization.json.JSONObject
import com.tencent.kuikly.core.reactive.collection.ObservableList
import com.tencent.kuikly.core.reactive.handler.observable
import com.tencent.kuikly.core.views.AlertDialog
import com.tencent.kuikly.core.views.Input
import com.tencent.kuikly.core.views.PageList
import com.tencent.kuikly.core.views.PageListView
import com.tencent.kuikly.core.views.Scroller
import com.tencent.kuikly.core.views.ScrollParams
import com.tencent.kuikly.core.views.Switch
import com.tencent.kuikly.core.views.TabItem
import com.tencent.kuikly.core.views.Tabs
import com.tencent.kuikly.core.views.Text
import com.tencent.kuikly.core.views.TextArea
import com.tencent.kuikly.core.views.View
import com.tencent.kuikly.core.views.compose.Button


// ===================== 输入 =====================

/** 字段标签（只读）：顶部 fieldGap 与上一字段拉开间距。 */
fun ViewContainer<*, *>.fieldLabel(label: String) {
    Text {
        attr {
            fontSize(AdminType.body)
            fontWeightMedium()
            color(AdminColors.textSecondary)
            marginTop(AdminSpace.fieldGap)
            marginBottom(AdminSpace.xs)
            text(label)
        }
    }
}

/**
 * 标签 + 输入框：默认高 48（触屏友好）。
 * ⚠️ Input 内边距方案：iOS 渲染层 Input 无 padding 属性、文字贴边，
 * 故由外层容器承担 fieldBg 背景 / 1px 边框 / radiusSm / 左右内边距 sm(12)，
 * Input 透明背景 flex(1) 填满容器。
 * ⚠️ Input 必须显式传 textDidChange 回调，否则输入不更新。
 */
fun ViewContainer<*, *>.labeledField(
    label: String,
    getValue: () -> String,
    onChange: (String) -> Unit,
    placeholder: String = "",
    height: Float = 48f,
) {
    fieldLabel(label)
    View {
        attr {
            flexDirectionRow()
            alignItemsCenter()
            height(height)
            backgroundColor(AdminColors.fieldBg)
            border(Border(1f, BorderStyle.SOLID, AdminColors.divider))
            borderRadius(AdminShape.radiusSm)
            paddingLeft(AdminSpace.sm)
            paddingRight(AdminSpace.sm)
        }
        Input {
            attr {
                flex(1f)
                height(height)
                backgroundColor(AdminColors.transparent)
                fontSize(AdminType.body)
                color(AdminColors.textPrimary)
                text(getValue())
                placeholder(placeholder)
            }
            event { textDidChange { params -> onChange(params.text) } }
        }
    }
}

/**
 * 标签 + 多行输入框（官方 TextArea 基座）：散文/提示词/系统提示等多行文本。
 * 官方 TextArea = 多行 Input（属性/事件同 Input，textDidChange 回调必须显式设置）。
 * 容器承担背景/边框/内边距（叶子组件不支持 padding，见文件头铁律），TextArea 透明背景 flex(1) 填满。
 */
fun ViewContainer<*, *>.labeledTextArea(
    label: String,
    getValue: () -> String,
    onChange: (String) -> Unit,
    placeholder: String = "",
    height: Float = 100f,
) {
    fieldLabel(label)
    View {
        attr {
            height(height)
            backgroundColor(AdminColors.fieldBg)
            border(Border(1f, BorderStyle.SOLID, AdminColors.divider))
            borderRadius(AdminShape.radiusSm)
            paddingLeft(AdminSpace.sm)
            paddingRight(AdminSpace.sm)
            paddingTop(AdminSpace.xs)
            paddingBottom(AdminSpace.xs)
        }
        TextArea {
            attr {
                flex(1f)
                backgroundColor(AdminColors.transparent)
                fontSize(AdminType.body)
                color(AdminColors.textPrimary)
                text(getValue())
                placeholder(placeholder)
            }
            event { textDidChange { params -> onChange(params.text) } }
        }
    }
}

// ===================== 按钮 =====================

/** 按钮视觉变体：主按钮（实心强调色）/ 次按钮（浅底内嵌）。危险态（danger）仅主按钮适用。 */
enum class ButtonVariant { PRIMARY, SECONDARY }

/**
 * 全站唯一按钮封装（appButton）：所有可点击的 CTA 按钮**必须**经由它。
 *
 * 基座 = **官方 Button 组件**（`com.tencent.kuikly.core.views.compose.Button`，ButtonView）：
 * 内部 `justifyContentCenter + alignItemsCenter` + 自适应宽度 Text——文字居中由布局层保证，
 * 与 Catalyst 文字测量/对齐无关（手搓 View+Text 的 textAlignCenter/flex 方案实测不可靠，
 * 「连接/断开」二字多次偏侧就是它）。官方 Button 还自带按压态高亮（highlightBackgroundColor）。
 *
 * 三态（正常 / 加载中 / 禁用）由 lambda 响应式驱动（⚠️ Kuikly 响应式铁律）：
 * enabled/loading/danger/dynamicText 必须是 lambda，在 attr/titleAttr 闭包内读取才随状态刷新。
 * - loading=true：中性灰底 + 禁用文字 + 文案切为 loadingText（官方 Button 无子节点插槽，
 *   塞不了菊花——官方机制优先，进度反馈靠文案 + 页面内状态徽标），且点击被拦截
 *   （isActive() 在 event 闭包内实时重读 observable，处理中真挡得住重复点击）。
 * - danger=true（仅主按钮）：红色系（断开/停止等破坏性操作）。
 * - dynamicText：响应式文案（如 连接↔断开 随状态切换）；提供时优先于 text。
 *
 * [primaryButton] / [secondaryButton] 仅是本函数的语义别名，最终都收敛到这一个封装。
 */
fun ViewContainer<*, *>.appButton(
    text: String,
    variant: ButtonVariant = ButtonVariant.PRIMARY,
    enabled: () -> Boolean = { true },
    danger: () -> Boolean = { false },
    loading: () -> Boolean = { false },
    dynamicText: (() -> String)? = null,
    loadingText: String? = null,
    onClick: () -> Unit,
) {
    // 实时「是否可点」：在 event/attr/titleAttr 闭包内调用，读取最新 observable（非构建期快照）
    val isActive: () -> Boolean = { enabled() && !loading() }
    val isPrimary = variant == ButtonVariant.PRIMARY
    Button {
        attr {
            height(if (isPrimary) 48f else 44f)
            if (isPrimary) minWidth(88f)
            paddingLeft(AdminSpace.md)
            paddingRight(AdminSpace.md)
            borderRadius(AdminShape.radiusSm)
            backgroundColor(
                when {
                    !isActive() -> AdminColors.disabledBg // 禁用与处理中统一中性灰底（动作不可用语义）
                    isPrimary && danger() -> AdminColors.danger
                    isPrimary -> AdminColors.accent
                    else -> AdminColors.insetBg
                },
            )
            // 官方按压态高亮（touchDown/touchUp 内部自治）；禁用/处理中不高亮（透明）
            highlightBackgroundColor(
                when {
                    !isActive() -> AdminColors.transparent
                    isPrimary && danger() -> AdminColors.dangerActive
                    isPrimary -> AdminColors.accentActive
                    else -> AdminColors.cardHover
                },
            )
            // titleAttr 属 ButtonAttr（须写在 attr 块内）；即 TextAttr，闭包内读 observable，
            // 由 Text 自身的 attr 响应式绑定驱动——状态变化时文字/颜色实时刷新
            titleAttr {
                fontSize(AdminType.body)
                fontWeightMedium()
                lines(1)
                text(
                    when {
                        loading() -> loadingText ?: dynamicText?.invoke() ?: text
                        else -> dynamicText?.invoke() ?: text
                    },
                )
                color(
                    when {
                        !isActive() -> AdminColors.disabledText
                        isPrimary -> AdminColors.textOnAccent
                        else -> AdminColors.textPrimary
                    },
                )
            }
        }
        event { click { if (isActive()) onClick() } }
    }
}

/**
 * 主按钮（三态）：[appButton] 的 PRIMARY 别名——实心强调色，danger 可用于断开/停止等破坏性操作。
 * 参数语义同 [appButton]。
 */
fun ViewContainer<*, *>.primaryButton(
    text: String,
    enabled: () -> Boolean = { true },
    danger: () -> Boolean = { false },
    loading: () -> Boolean = { false },
    dynamicText: (() -> String)? = null,
    loadingText: String? = null,
    onClick: () -> Unit,
) = appButton(
    text, ButtonVariant.PRIMARY,
    enabled = enabled, danger = danger, loading = loading,
    dynamicText = dynamicText, loadingText = loadingText, onClick = onClick,
)

/**
 * 次按钮（三态）：[appButton] 的 SECONDARY 别名——浅底内嵌样式，用于次要动作。
 * 参数语义同 [appButton]。
 */
fun ViewContainer<*, *>.secondaryButton(
    text: String,
    enabled: () -> Boolean = { true },
    loading: () -> Boolean = { false },
    dynamicText: (() -> String)? = null,
    loadingText: String? = null,
    onClick: () -> Unit,
) = appButton(
    text, ButtonVariant.SECONDARY,
    enabled = enabled, loading = loading,
    dynamicText = dynamicText, loadingText = loadingText, onClick = onClick,
)

/** 卡片内动作行：横向排按钮/徽标，顶部留 sm(12) 间距；元素间距由调用处 `View { width(12f) }` 控制。
 *  ⚠️ allCenter()：行内元素水平+垂直居中（按钮不再贴左，连接按钮等 CTA 居中更清晰）。 */
fun ViewContainer<*, *>.actionRow(content: ViewBuilder) {
    View {
        attr {
            flexDirectionRow()
            allCenter()
            marginTop(AdminSpace.sm)
        }
        content()
    }
}

// ===================== 开关行 =====================

/**
 * 开关行：高 52（触屏），左标签 body(17)/500 textPrimary，右开关。
 * 开关用**官方 Switch 组件**（组合组件：iOS 走原生 UISwitch，其他平台官方内置实现，跨端一致），
 * 颜色对齐主题 token：开=accent、关=switchOff、滑块=knob。
 * 注：checked 为初始值；点击后的视觉翻转由官方组件内部状态自治，业务在 onToggle 同步数据。
 */
fun ViewContainer<*, *>.switchRow(label: String, checked: Boolean, onToggle: () -> Unit) {
    View {
        attr {
            flexDirectionRow()
            alignItemsCenter()
            height(52f)
            marginTop(AdminSpace.fieldGap)
        }
        Text {
            attr {
                flex(1f)
                fontSize(AdminType.body)
                fontWeightMedium()
                color(AdminColors.textPrimary)
                text(label)
            }
        }
        Switch {
            attr {
                width(46f)
                height(26f)
                isOn(checked)
                onColor(AdminColors.accent)
                unOnColor(AdminColors.switchOff)
                thumbColor(AdminColors.knob)
            }
            event {
                switchOnChanged { onToggle() }
            }
        }
    }
}
