package com.xiaozhi.admin

import com.tencent.kuikly.core.base.Border
import com.tencent.kuikly.core.base.BorderStyle
import com.tencent.kuikly.core.base.CursorType
import com.tencent.kuikly.core.base.ViewContainer
import com.tencent.kuikly.core.directives.vif
import com.tencent.kuikly.core.views.View
import com.tencent.kuikly.core.views.compose.Button

/** Visual appearance is separate from operation state, selection, availability and danger. */
enum class ButtonVariant { PRIMARY, SECONDARY, OUTLINE, TEXT }

/**
 * Official Button remains the sole click/press/title host. The loading indicator is a
 * non-interactive sibling overlay, NOT an unsupported child inside official Button.
 * New controllers pass typed state/availability; legacy lambdas retain the same behavior.
 */
fun ViewContainer<*, *>.appButton(
    text: String,
    variant: ButtonVariant = ButtonVariant.PRIMARY,
    enabled: () -> Boolean = { true },
    danger: () -> Boolean = { false },
    loading: () -> Boolean = { false },
    result: () -> String = { "" },
    dynamicText: (() -> String)? = null,
    loadingText: String? = null,
    successText: String? = null,
    errorText: String? = null,
    state: (() -> ActionState)? = null,
    availability: (() -> Availability)? = null,
    icon: IconToken? = null,
    iconOnly: Boolean = false,
    accessibilityLabel: String? = null,
    selected: () -> Boolean = { false },
    onClick: () -> Unit,
) {
    require(!iconOnly || icon != null) { "Icon-only buttons need an icon" }
    require(!iconOnly || !(accessibilityLabel ?: text).isBlank()) { "Icon-only buttons need an accessible label" }
    val operation: () -> ActionState = {
        state?.invoke() ?: when {
            loading() -> ActionState.Running(0)
            result() == "ok" -> ActionState.Finished(ActionOutcome.Success())
            result() == "error" -> ActionState.Finished(ActionOutcome.Failure("操作失败"))
            else -> ActionState.Idle
        }
    }
    val currentAvailability: () -> Availability = {
        availability?.invoke() ?: if (enabled()) Availability.Enabled else Availability.Disabled("操作不可用")
    }
    val isLoading = { operation() is ActionState.Running }
    val isActive = { enabled() && canInvokeAction(currentAvailability(), operation()) }
    val isFail = { (operation() as? ActionState.Finished)?.outcome is ActionOutcome.Failure }
    val isSuccess = { (operation() as? ActionState.Finished)?.outcome is ActionOutcome.Success }
    val primary = variant == ButtonVariant.PRIMARY
    val height = if (primary) 48f else 44f
    val foreground = {
        when {
            !isActive() -> AdminColors.disabledText
            primary -> AdminColors.textOnAccent
            isFail() || danger() -> AdminColors.dangerTintText
            selected() || variant == ButtonVariant.TEXT -> AdminColors.accentTintText
            else -> AdminColors.textPrimary
        }
    }
    View {
        attr { flexDirectionRow(); alignItemsCenter() }
        Button {
            attr {
                height(height)
                if (iconOnly) width(height) else if (primary) minWidth(88f)
                // The vector sibling and spinner share this reserved leading slot.
                paddingLeft(if (iconOnly) 0f else AdminSpace.md + if (icon != null || isLoading()) 24f else 0f)
                paddingRight(if (iconOnly) 0f else AdminSpace.md)
                borderRadius(AdminShape.radiusSm)
                cursor(CursorType.POINTER)
                accessibility(buildString {
                    append(accessibilityLabel ?: dynamicText?.invoke() ?: text)
                    if (selected()) append("，已选中")
                    if (isLoading()) append("，处理中")
                    (currentAvailability() as? Availability.Disabled)?.let { append("，${it.reason}") }
                })
                if (variant == ButtonVariant.OUTLINE) {
                    border(Border(1f, BorderStyle.SOLID, if (isFail() || danger()) AdminColors.danger else AdminColors.dividerStrong))
                }
                backgroundColor(when {
                    !isActive() -> AdminColors.disabledBg
                    primary && (danger() || isFail()) -> AdminColors.danger
                    primary -> AdminColors.accent
                    isFail() -> AdminColors.dangerTintBg
                    selected() -> AdminColors.accentTintBg
                    variant == ButtonVariant.TEXT || variant == ButtonVariant.OUTLINE -> AdminColors.transparent
                    else -> AdminColors.insetBg
                })
                highlightBackgroundColor(when {
                    !isActive() -> AdminColors.transparent
                    primary && (danger() || isFail()) -> AdminColors.dangerActive
                    primary -> AdminColors.accentActive
                    else -> AdminColors.cardHover
                })
                if (!iconOnly) {
                    titleAttr {
                        // Override official Button's default 5px image/title gap: the reserved
                        // absolute image slot is already included in the button's padding.
                        marginLeft(0f)
                        fontSize(AdminType.body)
                        fontWeightMedium()
                        lines(1)
                        text(when {
                            isLoading() -> loadingText ?: dynamicText?.invoke() ?: text
                            isSuccess() -> successText ?: dynamicText?.invoke() ?: text
                            isFail() -> errorText ?: dynamicText?.invoke() ?: text
                            else -> dynamicText?.invoke() ?: text
                        })
                        color(foreground())
                    }
                }
            }
            event { click { if (isActive()) onClick() } }
        }
        if (icon != null) {
            Svg {
                attr {
                    absolutePosition(
                        left = if (iconOnly) (height - 20f) / 2f else AdminSpace.md,
                        top = (height - 20f) / 2f,
                    )
                    size(20f, 20f)
                    src(icon.svgSource())
                    tintColor(foreground())
                    opacity(if (isLoading()) 0f else 1f)
                    touchEnable(false)
                }
            }
        }
        vif({ isLoading() }) {
            View {
                attr {
                    absolutePosition(
                        left = if (iconOnly) (height - 16f) / 2f else AdminSpace.md + if (icon != null) 2f else 0f,
                        top = (height - 16f) / 2f,
                    )
                    size(16f, 16f)
                    touchEnable(false)
                }
                activityIndicator()
            }
        }
    }
}

fun ViewContainer<*, *>.primaryButton(
    text: String,
    enabled: () -> Boolean = { true },
    danger: () -> Boolean = { false },
    loading: () -> Boolean = { false },
    result: () -> String = { "" },
    dynamicText: (() -> String)? = null,
    loadingText: String? = null,
    successText: String? = null,
    errorText: String? = null,
    onClick: () -> Unit,
) = appButton(
    text, ButtonVariant.PRIMARY, enabled, danger, loading, result, dynamicText,
    loadingText, successText, errorText, onClick = onClick,
)

fun ViewContainer<*, *>.secondaryButton(
    text: String,
    enabled: () -> Boolean = { true },
    loading: () -> Boolean = { false },
    result: () -> String = { "" },
    dynamicText: (() -> String)? = null,
    loadingText: String? = null,
    successText: String? = null,
    errorText: String? = null,
    onClick: () -> Unit,
) = appButton(
    text, ButtonVariant.SECONDARY, enabled = enabled, loading = loading, result = result,
    dynamicText = dynamicText, loadingText = loadingText, successText = successText,
    errorText = errorText, onClick = onClick,
)

fun ViewContainer<*, *>.textButton(text: String, enabled: () -> Boolean = { true }, onClick: () -> Unit) =
    appButton(text, ButtonVariant.TEXT, enabled = enabled, onClick = onClick)

fun ViewContainer<*, *>.outlineButton(text: String, enabled: () -> Boolean = { true }, onClick: () -> Unit) =
    appButton(text, ButtonVariant.OUTLINE, enabled = enabled, onClick = onClick)
