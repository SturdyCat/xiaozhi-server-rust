package com.xiaozhi.admin.kit

import androidx.compose.runtime.State
import androidx.compose.runtime.mutableStateOf

actual fun systemDarkModeState(): State<Boolean> {
    // TODO: 接入 UiModeManager / Configuration.uiMode 读取系统深色模式并监听变化。
    // 当前占位返回 false（不跟随系统深色），避免未实现崩溃；不影响 web / macOS / iOS 主路径。
    return mutableStateOf(false)
}
