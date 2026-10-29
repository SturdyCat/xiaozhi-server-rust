// 新增：macOS actual，非上游代码（Apache-2.0 §4(b) 改动标注见本模块 THIRD_PARTY_NOTICES.md）。
// 理由：该函数的唯一作用是让**系统绘制的部件**（iOS 软键盘、系统弹窗）跟随 App 外观，
// macOS 上无此概念；且本项目主题固定为 Light（themeMode 恒为 Light），
// 即便实现也不会产生任何可见差异。不选 AppKit 路线（NSApp.appearance）：
// 本产物是 Mac Catalyst（UIKit 进程），AppKit 符号能否在该进程安全调用未经证实，收益为零。
package com.gearui.system

import com.gearui.theme.ThemeMode

internal actual fun applyPlatformInterfaceStyle(mode: ThemeMode) {
    // macOS 无系统级界面风格跟随需求：no-op。
}
