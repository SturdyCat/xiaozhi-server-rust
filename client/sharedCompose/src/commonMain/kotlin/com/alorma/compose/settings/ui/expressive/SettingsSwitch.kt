package com.alorma.compose.settings.ui.expressive

import androidx.compose.runtime.Composable
import com.tencent.kuikly.compose.foundation.layout.fillMaxWidth
import com.tencent.kuikly.compose.material3.ExperimentalMaterial3ExpressiveApi
import com.tencent.kuikly.compose.material3.ListItemColors
import com.tencent.kuikly.compose.material3.ListItemElevation
import com.tencent.kuikly.compose.material3.ListItemShapes
import com.tencent.kuikly.compose.material3.SegmentedListItem
import com.tencent.kuikly.compose.material3.Switch
import com.tencent.kuikly.compose.material3.SwitchColors
import com.tencent.kuikly.compose.material3.SwitchDefaults
import com.tencent.kuikly.compose.ui.Modifier
import com.tencent.kuikly.compose.ui.semantics.SemanticsPropertyReceiver
import com.tencent.kuikly.compose.ui.semantics.clearAndSetSemantics
import com.tencent.kuikly.compose.ui.semantics.semantics
import com.alorma.compose.settings.ui.core.LocalSettingsGroupEnabled

@OptIn(ExperimentalMaterial3ExpressiveApi::class)
@Composable
fun SettingsSwitch(
  state: Boolean,
  title: @Composable () -> Unit,
  modifier: Modifier = Modifier,
  enabled: Boolean = LocalSettingsGroupEnabled.current,
  icon: @Composable (() -> Unit)? = null,
  subtitle: @Composable (() -> Unit)? = null,
  colors: ListItemColors = SettingsTileDefaults.colors(),
  switchColors: SwitchColors = SwitchDefaults.colors(),
  shapes: ListItemShapes = SettingsTileDefaults.shapes(),
  elevation: ListItemElevation = SettingsTileDefaults.elevation(),
  semanticProperties: (SemanticsPropertyReceiver.() -> Unit) = {},
  onCheckedChange: (Boolean) -> Unit,
) {
  val update: (Boolean) -> Unit = { boolean -> onCheckedChange(boolean) }

  SegmentedListItem(
    modifier = Modifier
      .fillMaxWidth()
      .semantics(properties = semanticProperties)
      .then(modifier),
    checked = state,
    onCheckedChange = { update(!state) },
    shapes = shapes,
    enabled = enabled,
    content = title,
    leadingContent = icon,
    supportingContent = subtitle,
    colors = colors,
    elevation = elevation,
    trailingContent = {
      Switch(
        modifier = Modifier.clearAndSetSemantics { },
        enabled = enabled,
        checked = state,
        onCheckedChange = update,
        colors = switchColors,
      )
    },
  )
}
