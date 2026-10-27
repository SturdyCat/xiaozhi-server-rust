package com.alorma.compose.settings.ui.core

import androidx.compose.runtime.Composable
import com.tencent.kuikly.compose.material3.ListItemColors
import com.tencent.kuikly.compose.material3.ListItemDefaults
import com.tencent.kuikly.compose.ui.unit.Dp

/**
 * Shared defaults for creating SettingsTileColors and SettingsTextStyles.
 * These are framework-agnostic and work with both standard and expressive Material 3.
 */
abstract class SettingsTileCoreDefaults {

  abstract val Elevation: Dp

  @Composable
  abstract fun colors(): ListItemColors
}
