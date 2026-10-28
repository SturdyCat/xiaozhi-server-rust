package com.tencent.kuikly.compose.material3

import androidx.compose.runtime.Composable
import androidx.compose.runtime.Immutable
import androidx.compose.runtime.ReadOnlyComposable
import com.tencent.kuikly.compose.foundation.BorderStroke
import com.tencent.kuikly.compose.foundation.layout.PaddingValues
import com.tencent.kuikly.compose.foundation.layout.RowScope
import com.tencent.kuikly.compose.ui.Modifier
import com.tencent.kuikly.compose.ui.graphics.Color
import com.tencent.kuikly.compose.ui.graphics.Shape
import com.tencent.kuikly.compose.ui.unit.Dp
import com.tencent.kuikly.compose.ui.unit.dp

object ButtonGroupDefaults {
    val ConnectedSpaceBetween: Dp = 2.dp
}

@Immutable
class ToggleButtonColors(
    val containerColor: Color,
    val contentColor: Color,
    val checkedContainerColor: Color,
    val checkedContentColor: Color,
    val disabledContainerColor: Color,
    val disabledContentColor: Color,
    val disabledCheckedContainerColor: Color,
    val disabledCheckedContentColor: Color,
)

object ToggleButtonDefaults {
    @Composable
    fun toggleButtonColors(
        containerColor: Color = MaterialTheme.colorScheme.surfaceContainerLow,
        contentColor: Color = MaterialTheme.colorScheme.onSurface,
        checkedContainerColor: Color = MaterialTheme.colorScheme.secondaryContainer,
        checkedContentColor: Color = MaterialTheme.colorScheme.onSecondaryContainer,
        disabledContainerColor: Color = containerColor.copy(alpha = DisabledAlpha),
        disabledContentColor: Color = contentColor.copy(alpha = DisabledAlpha),
        disabledCheckedContainerColor: Color = checkedContainerColor.copy(alpha = DisabledAlpha),
        disabledCheckedContentColor: Color = checkedContentColor.copy(alpha = DisabledAlpha),
    ): ToggleButtonColors = ToggleButtonColors(
        containerColor = containerColor,
        contentColor = contentColor,
        checkedContainerColor = checkedContainerColor,
        checkedContentColor = checkedContentColor,
        disabledContainerColor = disabledContainerColor,
        disabledContentColor = disabledContentColor,
        disabledCheckedContainerColor = disabledCheckedContainerColor,
        disabledCheckedContentColor = disabledCheckedContentColor,
    )
}

@OptIn(ExperimentalMaterial3ExpressiveApi::class)
@Composable
fun OutlinedToggleButton(
    checked: Boolean,
    onCheckedChange: (Boolean) -> Unit,
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
    shape: Shape = ButtonDefaults.outlinedShape,
    colors: ToggleButtonColors = ToggleButtonDefaults.toggleButtonColors(),
    border: BorderStroke? = ButtonDefaults.outlinedButtonBorder(enabled),
    contentPadding: PaddingValues = ButtonDefaults.ContentPadding,
    content: @Composable RowScope.() -> Unit,
) {
    val containerColor = when {
        !enabled && checked -> colors.disabledCheckedContainerColor
        !enabled -> colors.disabledContainerColor
        checked -> colors.checkedContainerColor
        else -> colors.containerColor
    }
    val contentColor = when {
        !enabled && checked -> colors.disabledCheckedContentColor
        !enabled -> colors.disabledContentColor
        checked -> colors.checkedContentColor
        else -> colors.contentColor
    }

    OutlinedButton(
        onClick = { onCheckedChange(!checked) },
        modifier = modifier,
        enabled = enabled,
        shape = shape,
        colors = ButtonDefaults.outlinedButtonColors(
            containerColor = containerColor,
            contentColor = contentColor,
            disabledContainerColor = colors.disabledContainerColor,
            disabledContentColor = colors.disabledContentColor,
        ),
        border = border,
        contentPadding = contentPadding,
        content = content,
    )
}

private const val DisabledAlpha = 0.38f
