package com.xiaozhi.admin.kit

import androidx.compose.runtime.State

/**
 * 当前系统深色模式状态。
 *
 * 由平台层提供：web 用 `prefers-color-scheme` 媒体查询，macOS/iOS 用 UIKit
 * `UITraitCollection`，Android 用 `UiModeManager`（此处先占位）。在 admin 根 `setContent`
 * 内读取 `.value` 传给 [com.gearui.App] 的 `isSystemDark`，由 `ThemeMode.System` 消费。
 */
expect fun systemDarkModeState(): State<Boolean>
