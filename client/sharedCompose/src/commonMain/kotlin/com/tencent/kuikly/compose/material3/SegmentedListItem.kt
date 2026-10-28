package com.tencent.kuikly.compose.material3

import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.Immutable
import com.tencent.kuikly.compose.foundation.clickable
import com.tencent.kuikly.compose.foundation.layout.Arrangement
import com.tencent.kuikly.compose.foundation.layout.Box
import com.tencent.kuikly.compose.foundation.layout.Column
import com.tencent.kuikly.compose.foundation.layout.PaddingValues
import com.tencent.kuikly.compose.foundation.layout.Row
import com.tencent.kuikly.compose.foundation.layout.fillMaxWidth
import com.tencent.kuikly.compose.foundation.layout.padding
import com.tencent.kuikly.compose.foundation.selection.toggleable
import com.tencent.kuikly.compose.ui.Alignment
import com.tencent.kuikly.compose.ui.Modifier
import com.tencent.kuikly.compose.ui.graphics.Color
import com.tencent.kuikly.compose.ui.graphics.RectangleShape
import com.tencent.kuikly.compose.ui.graphics.Shape
import com.tencent.kuikly.compose.ui.semantics.Role
import com.tencent.kuikly.compose.ui.unit.Dp
import com.tencent.kuikly.compose.ui.unit.dp

@Immutable
class ListItemShapes(
    val containerShape: Shape,
)

@Immutable
class ListItemElevation(
    val tonalElevation: Dp,
    val shadowElevation: Dp,
)

@OptIn(ExperimentalMaterial3ExpressiveApi::class)
@Composable
fun SegmentedListItem(
    shapes: ListItemShapes,
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
    selected: Boolean = false,
    onClick: () -> Unit = {},
    leadingContent: (@Composable () -> Unit)? = null,
    trailingContent: (@Composable () -> Unit)? = null,
    supportingContent: (@Composable () -> Unit)? = null,
    colors: ListItemColors = ListItemDefaults.segmentedColors(),
    elevation: ListItemElevation = ListItemDefaults.elevation(),
    contentPadding: PaddingValues = ListItemDefaults.ContentPadding,
    content: @Composable () -> Unit,
) {
    SegmentedListItemLayout(
        shapes = shapes,
        modifier = modifier.clickable(enabled = enabled, onClick = onClick),
        enabled = enabled,
        selected = selected,
        leadingContent = leadingContent,
        trailingContent = trailingContent,
        supportingContent = supportingContent,
        colors = colors,
        elevation = elevation,
        contentPadding = contentPadding,
        content = content,
    )
}

@OptIn(ExperimentalMaterial3ExpressiveApi::class)
@Composable
fun SegmentedListItem(
    modifier: Modifier = Modifier,
    checked: Boolean = false,
    onCheckedChange: (Boolean) -> Unit = {},
    shapes: ListItemShapes = ListItemDefaults.shapes(),
    enabled: Boolean = true,
    content: @Composable () -> Unit,
    leadingContent: (@Composable () -> Unit)? = null,
    supportingContent: (@Composable () -> Unit)? = null,
    colors: ListItemColors = ListItemDefaults.segmentedColors(),
    elevation: ListItemElevation = ListItemDefaults.elevation(),
    trailingContent: (@Composable () -> Unit)? = null,
    contentPadding: PaddingValues = ListItemDefaults.ContentPadding,
) {
    SegmentedListItemLayout(
        shapes = shapes,
        modifier = modifier.toggleable(
            value = checked,
            enabled = enabled,
            role = Role.Checkbox,
            onValueChange = onCheckedChange,
        ),
        enabled = enabled,
        selected = checked,
        leadingContent = leadingContent,
        trailingContent = trailingContent,
        supportingContent = supportingContent,
        colors = colors,
        elevation = elevation,
        contentPadding = contentPadding,
        content = content,
    )
}

@OptIn(ExperimentalMaterial3ExpressiveApi::class)
@Composable
private fun SegmentedListItemLayout(
    shapes: ListItemShapes,
    modifier: Modifier,
    enabled: Boolean,
    selected: Boolean,
    leadingContent: (@Composable () -> Unit)?,
    trailingContent: (@Composable () -> Unit)?,
    supportingContent: (@Composable () -> Unit)?,
    colors: ListItemColors,
    elevation: ListItemElevation,
    contentPadding: PaddingValues,
    content: @Composable () -> Unit,
) {
    val containerColor = if (enabled) {
        if (selected) colors.containerColor else colors.containerColor
    } else {
        colors.containerColor
    }
    val headlineColor = if (enabled) colors.headlineColor else colors.disabledHeadlineColor
    val supportingColor = if (enabled) colors.supportingColor else colors.disabledSupportingColor
    val leadingIconColor = if (enabled) colors.leadingIconColor else colors.disabledLeadingIconColor
    val trailingIconColor = if (enabled) colors.trailingIconColor else colors.disabledTrailingIconColor

    Surface(
        modifier = modifier.fillMaxWidth(),
        shape = shapes.containerShape,
        color = containerColor,
        contentColor = headlineColor,
        tonalElevation = elevation.tonalElevation,
        shadowElevation = elevation.shadowElevation,
    ) {
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .padding(contentPadding),
            horizontalArrangement = Arrangement.spacedBy(16.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            if (leadingContent != null) {
                CompositionLocalProvider(LocalContentColor provides leadingIconColor) {
                    Box { leadingContent() }
                }
            }
            Column(
                modifier = Modifier.weight(1f),
                verticalArrangement = Arrangement.spacedBy(2.dp),
            ) {
                CompositionLocalProvider(LocalContentColor provides headlineColor) {
                    content()
                }
                if (supportingContent != null) {
                    CompositionLocalProvider(LocalContentColor provides supportingColor) {
                        supportingContent()
                    }
                }
            }
            if (trailingContent != null) {
                CompositionLocalProvider(LocalContentColor provides trailingIconColor) {
                    Box { trailingContent() }
                }
            }
        }
    }
}
