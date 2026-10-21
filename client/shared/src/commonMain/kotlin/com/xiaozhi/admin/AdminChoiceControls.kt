package com.xiaozhi.admin

import com.tencent.kuikly.core.base.Border
import com.tencent.kuikly.core.base.BorderStyle
import com.tencent.kuikly.core.base.CursorType
import com.tencent.kuikly.core.base.ViewBuilder
import com.tencent.kuikly.core.base.ViewContainer
import com.tencent.kuikly.core.directives.vfor
import com.tencent.kuikly.core.directives.vif
import com.tencent.kuikly.core.reactive.collection.ObservableList
import com.tencent.kuikly.core.views.AlertDialog
import com.tencent.kuikly.core.views.CheckBox
import com.tencent.kuikly.core.views.Scroller
import com.tencent.kuikly.core.views.Text
import com.tencent.kuikly.core.views.View

/** Presentation descriptor only; owning controllers retain selection and domain serialization. */
data class ChoiceOption<T>(
    val value: T,
    val label: String,
    val icon: IconToken? = null,
    val availability: Availability = Availability.Enabled,
)

fun ViewContainer<*, *>.buttonGroup(content: ViewBuilder) {
    View {
        attr { flexDirectionRow(); alignItemsCenter() }
        content()
    }
}

/** An immutable, bounded option set. Selection and availability remain reactive lambdas. */
fun <T> ViewContainer<*, *>.segmentedChoice(
    label: String,
    options: List<ChoiceOption<T>>,
    selectedValue: () -> T,
    availability: () -> Availability = { Availability.Enabled },
    onSelect: (T) -> Unit,
) {
    fieldLabel(label)
    buttonGroup {
        options.forEachIndexed { index, option ->
            if (index > 0) View { attr { width(AdminSpace.xs) } }
            appButton(
                text = option.label,
                variant = ButtonVariant.SECONDARY,
                availability = { if (availability() is Availability.Disabled) availability() else option.availability },
                icon = option.icon,
                selected = { selectedValue() == option.value },
            ) {
                if (selectedValue() != option.value) onSelect(option.value)
            }
        }
    }
}

/** Inherit is a nullable override, never CheckState.Indeterminate or an operation result. */
fun ViewContainer<*, *>.overrideChoiceRow(
    label: String,
    choice: () -> OverrideChoice,
    availability: () -> Availability = { Availability.Enabled },
    onChange: (OverrideChoice) -> Unit,
) = segmentedChoice(
    label,
    listOf(
        ChoiceOption(OverrideChoice.Inherit, "继承"),
        ChoiceOption(OverrideChoice.ForceOn, "强制开启"),
        ChoiceOption(OverrideChoice.ForceOff, "强制关闭"),
    ),
    choice,
    availability,
    onChange,
)

/**
 * Official CheckBox supplies interaction; custom creators supply the typed mixed glyph.
 * checkedDidChanged also fires on programmatic attr updates in Kuikly. Comparing with
 * the owner's Boolean projection prevents external fill/reset from firing onChange.
 * The only copied Boolean is the official control's own internal implementation state.
 */
fun ViewContainer<*, *>.checkBoxRow(
    label: String,
    state: () -> CheckState,
    availability: () -> Availability = { Availability.Enabled },
    onChange: (CheckState) -> Unit,
) {
    val canChange = { availability() is Availability.Enabled }
    View {
        attr { flexDirectionRow(); alignItemsCenter(); marginTop(AdminSpace.fieldGap) }
        CheckBox {
            attr {
                size(44f, 44f)
                cursor(CursorType.POINTER)
                checked(state() == CheckState.Checked)
                disable(!canChange())
                accessibility(choiceAccessibility(label, state(), availability()))
                checkedViewCreator { checkIndicator(state, canChange) }
                defaultViewCreator { checkIndicator(state, canChange) }
            }
            event {
                checkedDidChanged { checked ->
                    if (canChange() && checked != (state() == CheckState.Checked)) {
                        onChange(if (checked) CheckState.Checked else CheckState.Unchecked)
                    }
                }
            }
        }
        Text {
            attr {
                flex(1f)
                fontSize(AdminType.body)
                color(if (canChange()) AdminColors.textPrimary else AdminColors.disabledText)
                text(label)
                cursor(CursorType.POINTER)
                accessibility(choiceAccessibility(label, state(), availability()))
            }
            event { click { if (canChange()) onChange(state().toggled()) } }
        }
    }
}

private fun choiceAccessibility(label: String, state: CheckState, availability: Availability): String =
    "$label，${when (state) {
        CheckState.Checked -> "已选中"
        CheckState.Unchecked -> "未选中"
        CheckState.Indeterminate -> "部分选中"
    }}" + ((availability as? Availability.Disabled)?.let { "，${it.reason}" } ?: "")

private fun ViewContainer<*, *>.checkIndicator(state: () -> CheckState, enabled: () -> Boolean) {
    View {
        attr { flex(1f); allCenter(); touchEnable(false); opacity(if (enabled()) 1f else 0.5f) }
        View {
            attr {
                size(22f, 22f)
                allCenter()
                borderRadius(5f)
                border(Border(1f, BorderStyle.SOLID, AdminColors.dividerStrong))
                backgroundColor(if (state() == CheckState.Unchecked) AdminColors.fieldBg else AdminColors.accent)
            }
            vif({ state() == CheckState.Checked }) {
                appIcon({ IconToken.Check }, 18f, { AdminColors.textOnAccent })
            }
            vif({ state() == CheckState.Indeterminate }) {
                View { attr { size(12f, 2f); borderRadius(1f); backgroundColor(AdminColors.textOnAccent) } }
            }
        }
    }
}

/**
 * Official modal, callback-only choice menu. Dismissal is explicit and idempotent at the
 * owner; unlike dropdownField this never toggles open state after onSelect/onDismiss.
 */
fun <T> ViewContainer<*, *>.choiceMenu(
    label: String,
    options: () -> ObservableList<ChoiceOption<T>>,
    selectedValue: () -> T,
    isOpen: () -> Boolean,
    onDismiss: () -> Unit,
    onSelect: (T) -> Unit,
) {
    val menuWidth = { (getPager().pageData.pageViewWidth - 2f * AdminSpace.md).coerceIn(0f, 420f) }
    AlertDialog {
        attr {
            showAlert(isOpen())
            inWindow(true)
            customContentView {
                View {
                    attr {
                        width(menuWidth())
                        padding(AdminSpace.cardPadding)
                        borderRadius(AdminShape.radiusLg)
                        backgroundColor(AdminColors.cardBg)
                    }
                    Text { attr { text(label); fontSize(AdminType.section); color(AdminColors.textPrimary) } }
                    Scroller {
                        attr { height(320f); marginTop(AdminSpace.sm) }
                        vfor(options) { option ->
                            View {
                                attr { marginBottom(AdminSpace.xs) }
                                appButton(
                                    text = option.label,
                                    variant = ButtonVariant.TEXT,
                                    availability = { option.availability },
                                    icon = option.icon,
                                    selected = { selectedValue() == option.value },
                                ) {
                                    onSelect(option.value)
                                    onDismiss()
                                }
                            }
                        }
                    }
                    secondaryButton("取消") { onDismiss() }
                }
            }
        }
        event {
            clickBackgroundMask { onDismiss() }
            willDismiss { onDismiss() }
        }
    }
}
