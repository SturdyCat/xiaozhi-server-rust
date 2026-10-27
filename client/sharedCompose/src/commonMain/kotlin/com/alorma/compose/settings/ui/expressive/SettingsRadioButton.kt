package com.alorma.compose.settings.ui.expressive

import androidx.compose.runtime.Composable
import com.tencent.kuikly.compose.foundation.layout.fillMaxWidth
import com.tencent.kuikly.compose.material3.ExperimentalMaterial3ExpressiveApi
import com.tencent.kuikly.compose.material3.ListItemColors
import com.tencent.kuikly.compose.material3.ListItemElevation
import com.tencent.kuikly.compose.material3.ListItemShapes
import com.tencent.kuikly.compose.material3.RadioButton
import com.tencent.kuikly.compose.material3.RadioButtonColors
import com.tencent.kuikly.compose.material3.RadioButtonDefaults
import com.tencent.kuikly.compose.material3.SegmentedListItem
import com.tencent.kuikly.compose.ui.Modifier
import com.tencent.kuikly.compose.ui.semantics.SemanticsPropertyReceiver
import com.tencent.kuikly.compose.ui.semantics.clearAndSetSemantics
import com.tencent.kuikly.compose.ui.semantics.semantics
import com.alorma.compose.settings.ui.core.LocalSettingsGroupEnabled

@OptIn(ExperimentalMaterial3ExpressiveApi::class)
@Composable
fun SettingsRadioButton(
  state: Boolean,
  title: @Composable () -> Unit,
  modifier: Modifier = Modifier,
  enabled: Boolean = LocalSettingsGroupEnabled.current,
  icon: @Composable (() -> Unit)? = null,
  subtitle: @Composable (() -> Unit)? = null,
  colors: ListItemColors = SettingsTileDefaults.colors(),
  checkboxColors: RadioButtonColors = RadioButtonDefaults.colors(),
  shapes: ListItemShapes = SettingsTileDefaults.shapes(),
  elevation: ListItemElevation = SettingsTileDefaults.elevation(),
  semanticProperties: (SemanticsPropertyReceiver.() -> Unit) = {},
  onClick: () -> Unit,
) {
  SegmentedListItem(
    modifier = Modifier
      .fillMaxWidth()
      .semantics(properties = semanticProperties)
      .then(modifier),
    checked = state,
    onCheckedChange = { onClick() },
    shapes = shapes,
    enabled = enabled,
    content = title,
    leadingContent = icon,
    supportingContent = subtitle,
    colors = colors,
    elevation = elevation,
    trailingContent = {
      RadioButton(
        modifier = Modifier.clearAndSetSemantics { },
        enabled = enabled,
        selected = state,
        onClick = onClick,
        colors = checkboxColors,
      )
    },
  )
}
