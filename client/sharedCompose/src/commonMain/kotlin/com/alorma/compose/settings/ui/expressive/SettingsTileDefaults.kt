package com.alorma.compose.settings.ui.expressive

import androidx.compose.runtime.Composable
import com.tencent.kuikly.compose.material3.ExperimentalMaterial3ExpressiveApi
import com.tencent.kuikly.compose.material3.ListItemColors
import com.tencent.kuikly.compose.material3.ListItemDefaults
import com.tencent.kuikly.compose.material3.ListItemElevation
import com.tencent.kuikly.compose.material3.ListItemShapes
import com.tencent.kuikly.compose.ui.unit.Dp
import com.alorma.compose.settings.ui.core.SettingsTileCoreDefaults

object SettingsTileDefaults : SettingsTileCoreDefaults() {
  override val Elevation: Dp = ListItemDefaults.Elevation

  @OptIn(ExperimentalMaterial3ExpressiveApi::class)
  @Composable
  fun shapes(): ListItemShapes = ListItemDefaults.shapes()

  @OptIn(ExperimentalMaterial3ExpressiveApi::class)
  @Composable
  fun elevation(): ListItemElevation = ListItemDefaults.elevation()

  @Composable
  override fun colors(): ListItemColors = ListItemDefaults.colors()
}
