package com.alorma.compose.settings.ui

import androidx.compose.runtime.Composable
import com.tencent.kuikly.compose.foundation.layout.Arrangement
import com.tencent.kuikly.compose.foundation.layout.Column
import com.tencent.kuikly.compose.foundation.layout.padding
import com.tencent.kuikly.compose.material3.ListItemColors
import com.tencent.kuikly.compose.material3.Slider
import com.tencent.kuikly.compose.material3.SliderColors
import com.tencent.kuikly.compose.material3.SliderDefaults
import com.tencent.kuikly.compose.ui.Modifier
import com.tencent.kuikly.compose.ui.graphics.Shape
import com.tencent.kuikly.compose.ui.unit.Dp
import com.tencent.kuikly.compose.ui.unit.dp
import com.alorma.compose.settings.ui.core.LocalSettingsGroupEnabled

@Composable
@Suppress("LongParameterList")
fun SettingsSlider(
  title: @Composable () -> Unit,
  value: Float,
  onValueChange: (Float) -> Unit,
  modifier: Modifier = Modifier,
  subtitle: @Composable (() -> Unit)? = null,
  icon: @Composable (() -> Unit)? = null,
  enabled: Boolean = LocalSettingsGroupEnabled.current,
  valueRange: ClosedFloatingPointRange<Float> = 0f..1f,
  steps: Int = 0,
  onValueChangeFinished: (() -> Unit)? = null,
  colors: ListItemColors = SettingsTileDefaults.colors(),
  sliderColors: SliderColors = SliderDefaults.colors(),
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
        Slider(
          value = value,
          onValueChange = { value -> onValueChange(value) },
          modifier = Modifier.padding(end = 16.dp),
          enabled = enabled,
          valueRange = valueRange,
          steps = steps,
          onValueChangeFinished = onValueChangeFinished,
          colors = sliderColors,
        )
      }
    },
    icon = icon,
    colors = colors,
    shape = shape,
    tonalElevation = tonalElevation,
    shadowElevation = shadowElevation,
  )
}
