package com.xiaozhi.admin

/**
 * macOS（本宿主走 iOS 渲染层的 Mac Catalyst）：`Menlo` 同样是系统自带等宽字体。
 * 与 `iosMain` 分开各写一份，是因为 `macosArm64Main` 不依赖 `iosMain`（层级模板里两者
 * 各自挂在 nativeMain 下，`AdminPlatform.kt` 的 actual 也是分开的）。
 */
actual val ADMIN_MONO_FONT: String = "Menlo"
