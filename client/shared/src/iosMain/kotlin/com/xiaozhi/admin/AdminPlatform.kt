package com.xiaozhi.admin

import com.tencent.kuikly.core.base.ViewContainer
import com.tencent.kuikly.core.module.Module
import com.tencent.kuikly.core.pager.Pager
import platform.Foundation.NSDate
import platform.Foundation.NSDateFormatter
import platform.Foundation.NSLocale
import platform.Foundation.dateWithTimeIntervalSince1970
import platform.Foundation.localeWithLocaleIdentifier
import platform.UIKit.UIPasteboard

/** iOS（真机 + Apple Silicon 模拟器共用）：无 macOS 专属能力（测试台与「切换服务器」为 mac-only）。 */
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
    // 防御分支：iOS 侧边栏不渲染测试台入口
}

actual fun ViewContainer<*, *>.macServerSwitchEntry(
    pager: Pager,
    open: () -> Boolean,
    onClosed: () -> Unit,
    onApplied: () -> Unit,
) {
}

actual fun pagerBaseUrl(pager: Pager): String = ""

/** 本地时区（NSDateFormatter 默认系统时区）；en_US_POSIX 锁死数字格式防区域变体。 */
actual fun formatEpochSeconds(secs: Long): String {
    if (secs <= 0) return ""
    val fmt = NSDateFormatter().apply {
        dateFormat = "yyyy-MM-dd HH:mm:ss"
        locale = NSLocale.localeWithLocaleIdentifier("en_US_POSIX")
    }
    return fmt.stringFromDate(NSDate.dateWithTimeIntervalSince1970(secs.toDouble()))
}

actual fun copyToClipboard(text: String): Boolean = try {
    UIPasteboard.generalPasteboard.string = text
    true
} catch (_: Exception) {
    false
}
