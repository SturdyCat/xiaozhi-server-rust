package com.xiaozhi.admin

import androidx.compose.runtime.Composable
import com.xiaozhi.admin.kit.labeledTextArea

/** Device-only form content; drafts, validation, save, 409 and reload remain in the shared config shell. */
@Composable
internal fun EchoMuseCompatibilitySettings(state: EchoMuseState) {
    val drafts = state.drafts
    EmGroupLabel("设备专属兼容设置 · 非官方功能")
    labeledTextArea("私听动画", drafts.listeningAnimDraft, { drafts.listeningAnimDraft = it },
        help = "设备本地动画 JSON 对象或数组；留空不修改。不等同于官方控制器的灯环场景。",
        placeholder = "留空不修改", enabled = !state.actionBusy)
}
