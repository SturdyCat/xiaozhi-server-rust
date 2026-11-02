package com.xiaozhi.admin.kit

import androidx.compose.runtime.State
import androidx.compose.runtime.mutableStateOf
import platform.Foundation.NSUserDefaults

actual fun systemDarkModeState(): State<Boolean> {
    // macOS：读取 AppleInterfaceStyle 用户默认（Dark 时为 "Dark"）。
    // 实时切换监听待补（需监听 appearance 变化通知 / 轮询），当前先返回当前值。
    val isDark = NSUserDefaults.standardUserDefaults.stringForKey("AppleInterfaceStyle") == "Dark"
    return mutableStateOf(isDark)
}
