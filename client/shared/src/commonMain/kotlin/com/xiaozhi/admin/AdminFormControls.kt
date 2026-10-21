package com.xiaozhi.admin

import com.tencent.kuikly.core.base.Attr
import com.tencent.kuikly.core.base.Border
import com.tencent.kuikly.core.base.BorderStyle
import com.tencent.kuikly.core.base.CursorType
import com.tencent.kuikly.core.base.ViewBuilder
import com.tencent.kuikly.core.base.ViewContainer
import com.tencent.kuikly.core.directives.vif
import com.tencent.kuikly.core.reactive.handler.observable
import com.tencent.kuikly.core.views.Input
import com.tencent.kuikly.core.views.Switch
import com.tencent.kuikly.core.views.Text
import com.tencent.kuikly.core.views.TextArea
import com.tencent.kuikly.core.views.TextInputState
import com.tencent.kuikly.core.views.View


// ===================== 输入 =====================

/**
 * 整数输入过滤：只保留 ASCII 数字。整数字段（端口/线程数/条数等）禁止小数点、
 * 负号、字母——否则保存路径 `toIntOrNull() ?: 默认值` 会把脏输入**静默回落成默认值**
 *（比如把 "80." 存成 8000），比报错更隐蔽。
 */
internal val INT_INPUT_FILTER: (String) -> String = { raw -> raw.filter { it.isDigit() } }

/** 小数输入过滤：数字 + 至多一个小数点（temperature/speed/threshold 等浮点字段防脏输入）。 */
internal val DECIMAL_INPUT_FILTER: (String) -> String = { raw ->
    buildString {
        var dotSeen = false
        for (ch in raw) {
            when {
                ch.isDigit() -> append(ch)
                ch == '.' && !dotSeen -> {
                    dotSeen = true
                    append(ch)
                }
            }
        }
    }
}

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
 * ⚠️ 输入绑定走 `textInputState` 通道（不可用 `text()`+`textDidChange` 受控回写：
 * Catalyst 上编程式 setText 会打断 IME 组词并形成回声循环 → 文字/拼音交替闪烁）。
 * ⚠️ web 渲染器例外：`setProp` 不处理 textInputState attr，不绑 `text()` 输入框永远显示
 * placeholder（误读成「保存的值丢了」）——经 [INPUT_NEEDS_TEXT_PROP] 仅 web 补绑 `text()`。
 * textInputState 通道由框架内置防回环（InputView/TextAreaView 的
 * isProcessingNativeEvent + lastSyncedTextInputState 挡住同态回填）；组词期间不回传 onChange。
 */
fun ViewContainer<*, *>.labeledField(
    label: String,
    getValue: () -> String,
    onChange: (String) -> Unit,
    placeholder: String = "",
    height: Float = 48f,
    /** 字段级提示（如"需重启 server 生效"）；传 lambda，见 [com.xiaozhi.admin.FormSchemaState.hotNote]。 */
    note: () -> String = { "" },
    /** 输入过滤（进状态前剥离非法字符）：[INT_INPUT_FILTER] / [DECIMAL_INPUT_FILTER]，null = 不过滤。 */
    inputFilter: ((String) -> String)? = null,
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
                // web 渲染器不处理 textInputState attr（只认 text prop），不绑则输入框
                // 永远显示 placeholder——用户会误读成「保存的值丢了」（仅 web 绑，见
                // AdminPlatform.INPUT_NEEDS_TEXT_PROP；原生绑 text() 会打断 IME 组词）
                if (INPUT_NEEDS_TEXT_PROP) {
                    text(getValue())
                }
                textInputState { TextInputState(text = getValue()) }
                placeholder(placeholder)
            }
            event {
                // 组词中（compositionStart != NO_COMPOSITION）不上报，避免应用态与拼音组词互相踩；
                // 过滤在去重判定**之前**：剥离后与现值相同就不落 onChange（不打无谓的 dirty 标）。
                textInputStateChange { state ->
                    if (state.compositionStart == TextInputState.NO_COMPOSITION) {
                        val next = inputFilter?.invoke(state.text) ?: state.text
                        if (next != getValue()) {
                            onChange(next)
                        }
                    }
                }
                // web 渲染器用户键入只走 textDidChange（input 事件→To 回调），
                // textInputStateChange 仅在框架回写文本时触发——不补挂则 web 上用户
                // 输入永远进不了表单状态，「保存」回传的是 fill 的旧值（保存无效的根因）。
                // 与 textInputStateChange 双挂安全：过滤后 `next != getValue()` 幂等去重；
                // web 的 text prop 回显通道会把过滤后的干净值写回输入框（圆点等被剥离字符即时消失）。
                if (INPUT_NEEDS_TEXT_PROP) {
                    textDidChange { params ->
                        val next = inputFilter?.invoke(params.text) ?: params.text
                        if (next != getValue()) {
                            onChange(next)
                        }
                    }
                }
            }
        }
    }
    // 字段级生效语义（schema 驱动）：`live` 不显示，只有"新会话/需重启"才提示——见 FormSchemaState
    vif({ note().isNotEmpty() }) {
        Text {
            attr {
                fontSize(AdminType.micro)
                color(AdminColors.warningTintText)
                marginTop(4f)
                text(note())
            }
        }
    }
}

/**
 * 标签 + **整数**输入框：[labeledField] + [INT_INPUT_FILTER]——键入实时剥离小数点/负号/字母，
 * 状态里永远是纯数字串。整型配置字段（端口/线程/条数）必须用它，不要用裸 [labeledField]。
 */
fun ViewContainer<*, *>.labeledIntField(
    label: String,
    getValue: () -> String,
    onChange: (String) -> Unit,
    placeholder: String = "",
    height: Float = 48f,
    note: () -> String = { "" },
) = labeledField(label, getValue, onChange, placeholder, height, note, inputFilter = INT_INPUT_FILTER)

/**
 * 标签 + **小数**输入框：[labeledField] + [DECIMAL_INPUT_FILTER]——放行数字与至多一个小数点
 * （temperature/speed/threshold 等浮点字段）。
 */
fun ViewContainer<*, *>.labeledDecimalField(
    label: String,
    getValue: () -> String,
    onChange: (String) -> Unit,
    placeholder: String = "",
    height: Float = 48f,
    note: () -> String = { "" },
) = labeledField(label, getValue, onChange, placeholder, height, note, inputFilter = DECIMAL_INPUT_FILTER)

/**
 * 标签 + 多行输入框（官方 TextArea 基座）：散文/提示词/系统提示等多行文本。
 * 官方 TextArea = 多行 Input；绑定通道与防闪烁约束同 [labeledField]（textInputState 通道）。
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
                // 同 labeledField：仅 web 需要 text() 才能显示已存值
                if (INPUT_NEEDS_TEXT_PROP) {
                    text(getValue())
                }
                textInputState { TextInputState(text = getValue()) }
                placeholder(placeholder)
            }
            event {
                // 组词中不上报，约束同 labeledField
                textInputStateChange { state ->
                    if (state.compositionStart == TextInputState.NO_COMPOSITION && state.text != getValue()) {
                        onChange(state.text)
                    }
                }
                // web 用户键入走 textDidChange（同 labeledField；onChange 幂等去重）
                if (INPUT_NEEDS_TEXT_PROP) {
                    textDidChange { params ->
                        if (params.text != getValue()) {
                            onChange(params.text)
                        }
                    }
                }
            }
        }
    }
}

/** 卡片内动作行：按钮/徽标靠右且垂直居中，窄屏自动换行；顶部留 sm(12) 间距。
 * 元素间距由调用处 `View { width(12f) }` 控制。 */
fun ViewContainer<*, *>.actionRow(content: ViewBuilder) {
    View {
        attr {
            flexDirectionRow()
            alignItemsCenter()
            justifyContentFlexEnd()
            flexWrapWrap()
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
