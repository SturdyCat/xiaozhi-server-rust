// 新增：macOS actual，非上游代码（Apache-2.0 §4(b) 改动标注见本模块 THIRD_PARTY_NOTICES.md）。
// 理由：macosArm64 的 Kotlin/Native 平台库不含 UIKit（UIKit 只在 ios/tvos/watchos 平台库中），
// 而 macOS 也没有 UIKeyboardWillChangeFrameNotification 这一概念；Mac Catalyst 运行时虽带 UIKit，
// 但本产物链接的是 macOS 平台库，强行引 UIKit 会在编译期找不到符号。
// macOS 无软键盘，故本 actual 为空实现：永不回调。
package com.gearui.foundation.keyboard

import com.tencent.kuikly.compose.ui.unit.Dp

internal actual fun observePlatformKeyboard(onHeight: (Dp) -> Unit) {
    // macOS 无软键盘：不注册任何监听，也不回调。
}
