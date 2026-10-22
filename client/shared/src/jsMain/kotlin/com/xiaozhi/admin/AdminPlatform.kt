package com.xiaozhi.admin

import com.tencent.kuikly.core.base.ViewContainer
import com.tencent.kuikly.core.module.Module
import com.tencent.kuikly.core.pager.Pager
import kotlinx.browser.window

/** Web：无 macOS 专属能力（测试台菜单已隐藏，服务器地址走同域相对路径）。 */
actual val IS_MACOS: Boolean = false

/** web 渲染器 setProp 不处理 textInputState attr，必须绑 text() 才显示已存值。 */
actual val INPUT_NEEDS_TEXT_PROP: Boolean = true

actual fun Pager.platformExternalModules(): Map<String, Module> = emptyMap()

actual fun ViewContainer<*, *>.renderTestbenchSection(
    pager: Pager,
    form: ConfigFormState,
    twoColumn: () -> Boolean,
    pageWidth: () -> Float,
    pageHeight: () -> Float,
) {
    // 防御分支：web 菜单已隐藏（AdminNav.visible），正常不会走到
}

actual fun ViewContainer<*, *>.macServerSwitchEntry(
    pager: Pager,
    open: () -> Boolean,
    onOpened: () -> Unit,
    onClosed: () -> Unit,
    onApplied: () -> Unit,
) {
    // web 由 server 同域托管，无「切换服务器」概念
}

actual fun pagerBaseUrl(pager: Pager): String = ""

/** 浏览器本地时区换算（Date 各 getter 即本地值），补零成固定列宽便于扫读。 */
actual fun formatEpochSeconds(secs: Long): String {
    if (secs <= 0) return ""
    val d = kotlin.js.Date(secs.toDouble() * 1000.0)
    fun p(n: Int): String = n.toString().padStart(2, '0')
    return "${d.getFullYear().toInt()}-${p(d.getMonth().toInt() + 1)}-${p(d.getDate().toInt())} " +
        "${p(d.getHours().toInt())}:${p(d.getMinutes().toInt())}:${p(d.getSeconds().toInt())}"
}

/** 浏览器本地时区偏移（getTimezoneOffset 返回 UTC−当地的分钟数，取负转成秒）。 */
actual fun localZoneOffsetSeconds(): Int = -kotlin.js.Date().getTimezoneOffset().toInt() * 60

/**
 * 优先 async Clipboard API（https/localhost）；管理台常以 `http://LAN-IP` 访问，
 * 非 secure context 下 API 不存在，回落隐藏 textarea + `execCommand('copy')`。
 */
actual fun copyToClipboard(text: String): Boolean = try {
    val clip = window.navigator.asDynamic().clipboard
    if (window.asDynamic().isSecureContext == true && clip != null) {
        clip.writeText(text) // Promise 不等待：写失败极罕见且无更优兜底
        true
    } else {
        // http://LAN-IP 部署不是 secure context：隐藏 textarea + execCommand 兜底
        val doc = window.document
        val body = doc.body ?: return false
        val ta = doc.createElement("textarea").asDynamic()
        ta.value = text
        ta.style.position = "fixed"
        ta.style.opacity = "0"
        body.appendChild(ta)
        ta.select()
        val ok = doc.asDynamic().execCommand("copy")
        body.removeChild(ta)
        ok == true
    }
} catch (_: Throwable) {
    false
}
