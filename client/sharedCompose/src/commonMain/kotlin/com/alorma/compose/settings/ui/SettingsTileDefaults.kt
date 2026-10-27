package com.alorma.compose.settings.ui

import androidx.compose.runtime.Composable
import com.tencent.kuikly.compose.material3.ListItemColors
import com.tencent.kuikly.compose.material3.ListItemDefaults
import com.tencent.kuikly.compose.ui.graphics.Shape
import com.tencent.kuikly.compose.ui.unit.Dp
import com.alorma.compose.settings.ui.core.SettingsTileCoreDefaults

object SettingsTileDefaults : SettingsTileCoreDefaults() {
  override val Elevation: Dp = ListItemDefaults.Elevation

  @Composable
  fun shape(): Shape = ListItemDefaults.shape

  @Composable
  override fun colors(): ListItemColors = ListItemDefaults.colors()
}
