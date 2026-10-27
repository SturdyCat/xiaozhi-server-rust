package com.alorma.compose.settings.ui

import androidx.compose.runtime.Composable
import com.tencent.kuikly.compose.foundation.layout.Arrangement
import com.tencent.kuikly.compose.foundation.layout.Column
import com.tencent.kuikly.compose.foundation.layout.fillMaxWidth
import com.tencent.kuikly.compose.material3.ListItemColors
import com.tencent.kuikly.compose.material3.SegmentedButton
import com.tencent.kuikly.compose.material3.SegmentedButtonColors
import com.tencent.kuikly.compose.material3.SegmentedButtonDefaults
import com.tencent.kuikly.compose.material3.SingleChoiceSegmentedButtonRow
import com.tencent.kuikly.compose.material3.Text
import com.tencent.kuikly.compose.ui.Modifier
import com.tencent.kuikly.compose.ui.graphics.Shape
import com.tencent.kuikly.compose.ui.text.AnnotatedString
import com.tencent.kuikly.compose.ui.unit.Dp
import com.tencent.kuikly.compose.ui.unit.dp
import com.alorma.compose.settings.ui.core.LocalSettingsGroupEnabled

@Composable
@Suppress("LongParameterList")
fun <T> SettingsSegmented(
  title: @Composable () -> Unit,
  items: List<T>,
  selectedItem: T?,
  onItemSelected: (T) -> Unit,
  itemTitleMap: (T) -> CharSequence,
  modifier: Modifier = Modifier,
  enabled: Boolean = LocalSettingsGroupEnabled.current,
  colors: ListItemColors = SettingsTileDefaults.colors(),
  buttonSpace: Dp = SegmentedButtonDefaults.BorderWidth,
  buttonShape: @Composable (Int) -> Shape = { index ->
    SegmentedButtonDefaults.itemShape(
      index = index,
      count = items.size,
    )
  },
  buttonColors: SegmentedButtonColors = SegmentedButtonDefaults.colors(),
  buttonIcon: @Composable (Boolean) -> Unit = { selected -> SegmentedButtonDefaults.Icon(selected) },
  subtitle: @Composable (() -> Unit)? = null,
  icon: @Composable (() -> Unit)? = null,
  shape: Shape = SettingsTileDefaults.shape(),
  tonalElevation: Dp = SettingsTileDefaults.Elevation,
  shadowElevation: Dp = SettingsTileDefaults.Elevation,
) {
  SettingsTileScaffold(
    modifier = modifier,
    title = title,
    subtitle = {
      Column(
        verticalArrangement = Arrangement.spacedBy(4.dp),
      ) {
        subtitle?.invoke()
        SingleChoiceSegmentedButtonRow(
          modifier = Modifier.fillMaxWidth(),
          space = buttonSpace,
        ) {
          items.forEachIndexed { index, item ->
            val buttonItemShape = buttonShape(index)

            SegmentedButton(
              enabled = enabled,
              shape = buttonItemShape,
              label = {
                when (val text = itemTitleMap(item)) {
                  is String -> Text(text = text)
                  is AnnotatedString -> Text(text = text)
                  else -> Text(text = text.toString())
                }
              },
              icon = { buttonIcon(item == selectedItem) },
              selected = item == selectedItem,
              onClick = { onItemSelected(item) },
              colors = buttonColors,
            )
          }
        }
      }
    },
    icon = icon,
    colors = colors,
    shape = shape,
    tonalElevation = tonalElevation,
    shadowElevation = shadowElevation,
  )
}
