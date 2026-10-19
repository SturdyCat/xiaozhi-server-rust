package com.xiaozhi.admin

import com.tencent.kuikly.core.base.ViewContainer
import com.tencent.kuikly.core.module.Module
import com.tencent.kuikly.core.pager.Pager

/** Android：无 macOS 专属能力（测试台与「切换服务器」为 mac-only）。 */
actual val IS_MACOS: Boolean = false

/** 原生渲染器由 textInputState 通道显示，无需 text()。 */
actual val INPUT_NEEDS_TEXT_PROP: Boolean = false

actual fun Pager.platformExternalModules(): Map<String, Module> = emptyMap()

actual fun ViewContainer<*, *>.renderTestbenchSection(
    pager: Pager,
    form: ConfigFormState,
    twoColumn: () -> Boolean,
    pageWidth: () -> Float,
    pageHeight: () -> Float,
) {
    // 防御分支：Android 侧边栏不渲染测试台入口
}

actual fun ViewContainer<*, *>.macServerSwitchEntry(
    pager: Pager,
    open: () -> Boolean,
    onClosed: () -> Unit,
    onApplied: () -> Unit,
) {
}

actual fun pagerBaseUrl(pager: Pager): String = ""
