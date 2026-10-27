package com.alorma.compose.settings.ui.expressive

import androidx.compose.runtime.Composable
import com.tencent.kuikly.compose.foundation.layout.fillMaxWidth
import com.tencent.kuikly.compose.material3.CheckboxColors
import com.tencent.kuikly.compose.material3.CheckboxDefaults
import com.tencent.kuikly.compose.material3.ExperimentalMaterial3ExpressiveApi
import com.tencent.kuikly.compose.material3.ListItemColors
import com.tencent.kuikly.compose.material3.ListItemElevation
import com.tencent.kuikly.compose.material3.ListItemShapes
import com.tencent.kuikly.compose.material3.SegmentedListItem
import com.tencent.kuikly.compose.material3.TriStateCheckbox
import com.tencent.kuikly.compose.ui.Modifier
import com.tencent.kuikly.compose.ui.semantics.SemanticsPropertyReceiver
import com.tencent.kuikly.compose.ui.semantics.clearAndSetSemantics
import com.tencent.kuikly.compose.ui.semantics.semantics
import com.tencent.kuikly.compose.ui.state.ToggleableState
import com.alorma.compose.settings.ui.core.LocalSettingsGroupEnabled

@OptIn(ExperimentalMaterial3ExpressiveApi::class)
@Composable
fun SettingsTriStateCheckbox(
  state: ToggleableState,
  title: @Composable () -> Unit,
  modifier: Modifier = Modifier,
  enabled: Boolean = LocalSettingsGroupEnabled.current,
  icon: @Composable (() -> Unit)? = null,
  subtitle: @Composable (() -> Unit)? = null,
  colors: ListItemColors = SettingsTileDefaults.colors(),
  checkboxColors: CheckboxColors = CheckboxDefaults.colors(),
  shapes: ListItemShapes = SettingsTileDefaults.shapes(),
  elevation: ListItemElevation = SettingsTileDefaults.elevation(),
  semanticProperties: (SemanticsPropertyReceiver.() -> Unit) = {},
  onCheckedChange: (ToggleableState) -> Unit = {},
) {
  val update: () -> Unit = {
    val newState = when (state) {
      ToggleableState.On -> ToggleableState.Off
      ToggleableState.Off -> ToggleableState.On
      ToggleableState.Indeterminate -> ToggleableState.On
    }
    onCheckedChange(newState)
  }

  SegmentedListItem(
    modifier = Modifier
      .fillMaxWidth()
      .semantics(properties = semanticProperties)
      .then(modifier),
    checked = when(state) {
      ToggleableState.On -> true
      ToggleableState.Indeterminate -> true
      ToggleableState.Off -> false
    },
    onCheckedChange = { update() },
    shapes = shapes,
    enabled = enabled,
    content = title,
    leadingContent = icon,
    supportingContent = subtitle,
    colors = colors,
    elevation = elevation,
    trailingContent = {
      TriStateCheckbox(
        modifier = Modifier.clearAndSetSemantics { },
        enabled = enabled,
        state = state,
        onClick = update,
        colors = checkboxColors,
      )
    },
  )
}
