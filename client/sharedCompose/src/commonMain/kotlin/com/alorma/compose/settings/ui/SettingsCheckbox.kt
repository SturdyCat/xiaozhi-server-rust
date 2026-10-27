package com.alorma.compose.settings.ui

import androidx.compose.runtime.Composable
import com.tencent.kuikly.compose.foundation.selection.toggleable
import com.tencent.kuikly.compose.material3.Checkbox
import com.tencent.kuikly.compose.material3.CheckboxColors
import com.tencent.kuikly.compose.material3.CheckboxDefaults
import com.tencent.kuikly.compose.material3.ListItemColors
import com.tencent.kuikly.compose.ui.Modifier
import com.tencent.kuikly.compose.ui.graphics.Shape
import com.tencent.kuikly.compose.ui.semantics.Role
import com.tencent.kuikly.compose.ui.semantics.SemanticsPropertyReceiver
import com.tencent.kuikly.compose.ui.semantics.clearAndSetSemantics
import com.tencent.kuikly.compose.ui.semantics.semantics
import com.tencent.kuikly.compose.ui.unit.Dp
import com.alorma.compose.settings.ui.core.LocalSettingsGroupEnabled

@Composable
fun SettingsCheckbox(
  state: Boolean,
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
  onCheckedChange: (Boolean) -> Unit,
) {
  val update: (Boolean) -> Unit = { boolean -> onCheckedChange(boolean) }
  SettingsTileScaffold(
    modifier =
      Modifier
        .toggleable(
          enabled = enabled,
          value = state,
          role = Role.Checkbox,
          onValueChange = { update(!state) },
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
    Checkbox(
      modifier = Modifier.clearAndSetSemantics { },
      enabled = enabled,
      checked = state,
      onCheckedChange = update,
      colors = checkboxColors,
    )
  }
}
