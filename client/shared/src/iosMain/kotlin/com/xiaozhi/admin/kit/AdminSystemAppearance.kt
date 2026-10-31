package com.xiaozhi.admin.kit

import androidx.compose.runtime.State
import androidx.compose.runtime.mutableStateOf
import platform.UIKit.UIScreen

actual fun systemDarkModeState(): State<Boolean> {
    // iOS：读取主屏 trait collection 的 userInterfaceStyle（Dark 的原始值为 2）。
    // 实时切换监听待补（需 override traitCollectionDidChange / iOS17+ registerForTraitCollectionChanges），
    // 当前先返回当前值。
    val isDark = UIScreen.mainScreen.traitCollection.userInterfaceStyle.value == 2L
    return mutableStateOf(isDark)
}
