package com.alorma.compose.settings.ui.core

import androidx.compose.runtime.ProvidableCompositionLocal
import androidx.compose.runtime.compositionLocalOf
import com.tencent.kuikly.compose.material3.ListItemColors

val LocalSettingsGroupEnabled: ProvidableCompositionLocal<Boolean> = compositionLocalOf { true }

val LocalSettingsTileColors: ProvidableCompositionLocal<ListItemColors?> = compositionLocalOf { null }
