package com.alorma.compose.settings.ui

import androidx.compose.runtime.Composable
import com.tencent.kuikly.compose.foundation.selection.triStateToggleable
import com.tencent.kuikly.compose.material3.CheckboxColors
import com.tencent.kuikly.compose.material3.CheckboxDefaults
import com.tencent.kuikly.compose.material3.ListItemColors
import com.tencent.kuikly.compose.material3.TriStateCheckbox
import com.tencent.kuikly.compose.ui.Modifier
import com.tencent.kuikly.compose.ui.graphics.Shape
import com.tencent.kuikly.compose.ui.semantics.Role
import com.tencent.kuikly.compose.ui.semantics.SemanticsPropertyReceiver
import com.tencent.kuikly.compose.ui.semantics.clearAndSetSemantics
import com.tencent.kuikly.compose.ui.semantics.semantics
import com.tencent.kuikly.compose.ui.state.ToggleableState
import com.tencent.kuikly.compose.ui.unit.Dp
import com.alorma.compose.settings.ui.core.LocalSettingsGroupEnabled

@Composable
fun SettingsTriStateCheckbox(
  state: Boolean?,
  title: @Composable () -> Unit,
  modifier: Modifier = Modifier,
  enabled: Boolean = LocalSettingsGroupEnabled.current,
  icon: @Composable (() -> Unit)? = null,
  subtitle: @Composable (() -> Unit)? = null,
  colors: ListItemColors = SettingsTileDefaults.colors(),
  checkboxColors: CheckboxColors = CheckboxDefaults.colors(),
  shape: Shape = SettingsTileDefaults.shape(),
  tonalElevation: Dp = SettingsTileDefaults.Elevation,
  shadowElevation: Dp = SettingsTileDefaults.Elevation,
  semanticProperties: (SemanticsPropertyReceiver.() -> Unit) = {},
  onCheckedChange: (Boolean) -> Unit = {},
) {
  val update: () -> Unit = { onCheckedChange(state?.not() ?: true) }
  SettingsTileScaffold(
    modifier =
      Modifier
        .triStateToggleable(
          state = mapNullableBooleanToToggleableState(state),
          enabled = enabled,
          role = Role.Checkbox,
          onClick = update,
        ).semantics(properties = semanticProperties)
        .then(modifier),
    title = title,
    subtitle = subtitle,
    icon = icon,
    colors = colors,
    shape = shape,
    tonalElevation = tonalElevation,
    shadowElevation = shadowElevation,
  ) {
    TriStateCheckbox(
      modifier = Modifier.clearAndSetSemantics { },
      enabled = enabled,
      state = mapNullableBooleanToToggleableState(state),
      onClick = update,
      colors = checkboxColors,
    )
  }
}

private fun mapNullableBooleanToToggleableState(state: Boolean?) =
  when (state) {
    true -> ToggleableState.On
    false -> ToggleableState.Off
    null -> ToggleableState.Indeterminate
  }
