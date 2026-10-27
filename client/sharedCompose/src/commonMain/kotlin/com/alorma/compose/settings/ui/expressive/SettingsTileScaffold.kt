package com.alorma.compose.settings.ui.expressive

import androidx.compose.runtime.Composable
import com.tencent.kuikly.compose.foundation.layout.fillMaxWidth
import com.tencent.kuikly.compose.material3.ExperimentalMaterial3ExpressiveApi
import com.tencent.kuikly.compose.material3.ListItemColors
import com.tencent.kuikly.compose.material3.ListItemElevation
import com.tencent.kuikly.compose.material3.ListItemShapes
import com.tencent.kuikly.compose.material3.SegmentedListItem
import com.tencent.kuikly.compose.ui.Modifier

@OptIn(ExperimentalMaterial3ExpressiveApi::class)
@Composable
fun SettingsTileScaffold(
  title: @Composable () -> Unit,
  modifier: Modifier = Modifier,
  enabled: Boolean = true,
  onClick: () -> Unit = {},
  supportingContent: @Composable (() -> Unit)? = null,
  leadingContent: @Composable (() -> Unit)? = null,
  colors: ListItemColors = SettingsTileDefaults.colors(),
  shapes: ListItemShapes = SettingsTileDefaults.shapes(),
  elevation: ListItemElevation = SettingsTileDefaults.elevation(),
  trailingContent: @Composable (() -> Unit)? = null,
) {
  SegmentedListItem(
    selected = false,
    onClick = onClick,
    shapes = shapes,
    modifier = Modifier.fillMaxWidth().then(modifier),
    enabled = enabled,
    leadingContent = leadingContent,
    trailingContent = trailingContent,
    supportingContent = supportingContent,
    colors = colors,
    elevation = elevation,
  ) {
    title()
  }
}
