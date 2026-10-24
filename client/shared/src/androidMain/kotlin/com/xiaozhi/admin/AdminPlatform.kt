package com.xiaozhi.admin

import com.xiaozhi.admin.ui.*

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
    onOpened: () -> Unit,
    onClosed: () -> Unit,
    onApplied: () -> Unit,
) {
}

actual fun pagerBaseUrl(pager: Pager): String = ""

actual fun formatEpochSeconds(secs: Long): String {
    if (secs <= 0) return ""
    return java.text.SimpleDateFormat("yyyy-MM-dd HH:mm:ss", java.util.Locale.getDefault())
        .format(java.util.Date(secs * 1000L))
}

/** 系统默认时区当前 UTC 偏移（毫秒 → 秒）。 */
actual fun localZoneOffsetSeconds(): Int =
    java.util.TimeZone.getDefault().getOffset(System.currentTimeMillis()) / 1000

/** Android 宿主未接 Context 桥，暂不支持一键复制（调用方 toast 提示）。 */
actual fun copyToClipboard(text: String): Boolean = false
