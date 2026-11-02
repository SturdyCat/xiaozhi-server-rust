package com.xiaozhi.admin.kit

import androidx.compose.runtime.MutableState
import androidx.compose.runtime.State
import androidx.compose.runtime.mutableStateOf
import kotlinx.browser.window

private var darkState: MutableState<Boolean>? = null
private var darkListenerAttached = false

private fun ensureDarkState(): MutableState<Boolean> {
    if (darkState == null) {
        val mql = window.matchMedia("(prefers-color-scheme: dark)")
        val state = mutableStateOf(mql.matches)
        if (!darkListenerAttached) {
            darkListenerAttached = true
            // 实时跟随系统配色切换：读取当前 mql.matches
            mql.addListener { state.value = mql.matches }
        }
        darkState = state
    }
    return darkState!!
}

actual fun systemDarkModeState(): State<Boolean> = ensureDarkState()
