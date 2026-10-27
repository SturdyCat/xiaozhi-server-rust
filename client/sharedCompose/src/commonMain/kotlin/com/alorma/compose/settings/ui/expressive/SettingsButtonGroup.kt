package com.alorma.compose.settings.ui.expressive

import androidx.compose.runtime.Composable
import com.tencent.kuikly.compose.foundation.layout.Arrangement
import com.tencent.kuikly.compose.foundation.layout.Column
import com.tencent.kuikly.compose.foundation.layout.Row
import com.tencent.kuikly.compose.foundation.layout.fillMaxWidth
import com.tencent.kuikly.compose.material3.ButtonGroupDefaults
import com.tencent.kuikly.compose.material3.ExperimentalMaterial3ExpressiveApi
import com.tencent.kuikly.compose.material3.ListItemColors
import com.tencent.kuikly.compose.material3.ListItemElevation
import com.tencent.kuikly.compose.material3.ListItemShapes
import com.tencent.kuikly.compose.material3.MaterialTheme
import com.tencent.kuikly.compose.material3.OutlinedToggleButton
import com.tencent.kuikly.compose.material3.Text
import com.tencent.kuikly.compose.material3.ToggleButtonDefaults
import com.tencent.kuikly.compose.ui.Modifier
import com.tencent.kuikly.compose.ui.unit.dp
import com.alorma.compose.settings.ui.core.LocalSettingsGroupEnabled

@OptIn(ExperimentalMaterial3ExpressiveApi::class)
@Composable
@Suppress("LongParameterList")
fun <T> SettingsButtonGroup(
  title: @Composable () -> Unit,
  items: List<T>,
  selectedItem: T?,
  onItemSelected: (T) -> Unit,
  itemTitleMap: (T) -> CharSequence,
  modifier: Modifier = Modifier,
  enabled: Boolean = LocalSettingsGroupEnabled.current,
  colors: ListItemColors = SettingsTileDefaults.colors(),
  subtitle: @Composable (() -> Unit)? = null,
  icon: @Composable (() -> Unit)? = null,
  shapes: ListItemShapes = SettingsTileDefaults.shapes(),
  elevation: ListItemElevation = SettingsTileDefaults.elevation(),
) {
  SettingsTileScaffold(
    modifier = modifier,
    enabled = enabled,
    title = title,
    supportingContent = {
      Column(
        verticalArrangement = Arrangement.spacedBy(4.dp),
      ) {
        subtitle?.invoke()

        Row(
          modifier = Modifier.fillMaxWidth(),
          horizontalArrangement = Arrangement.spacedBy(ButtonGroupDefaults.ConnectedSpaceBetween),
        ) {
          items.forEachIndexed { _, item ->
            OutlinedToggleButton(
              modifier = Modifier.weight(1f),
              checked = item == selectedItem,
              onCheckedChange = { onItemSelected(item) },
              enabled = enabled,
              colors = ToggleButtonDefaults.toggleButtonColors(containerColor = MaterialTheme.colorScheme.surfaceContainerLow),
            ) {
              Text(text = itemTitleMap(item).toString())
            }
          }
        }
      }
    },
    leadingContent = icon,
    colors = colors,
    shapes = shapes,
    elevation = elevation,
  )
}
