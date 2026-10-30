package com.xiaozhi.admin

import androidx.compose.runtime.Composable
import com.tencent.kuikly.core.module.Module
import com.tencent.kuikly.core.nvi.serialization.json.JSONObject
import com.tencent.kuikly.core.pager.Pager
import kotlinx.browser.window

/** Web：无 macOS 专属能力（测试台菜单已隐藏，服务器地址走同域相对路径）。 */
actual val IS_MACOS: Boolean = false

actual fun Pager.platformExternalModules(): Map<String, Module> = emptyMap()

@Composable
actual fun TestbenchSection(page: AdminPage) {
    // 防御分支：web 菜单已隐藏（AdminNav.visible），正常不会走到
}

actual fun Pager.startAdminServerDiscovery(onResult: (JSONObject?) -> Unit) {
    // web 无 mDNS 能力（「服务器设置」入口只在 macOS 渲染）
}

actual fun Pager.stopAdminServerDiscovery() {
}

actual fun pagerBaseUrl(pager: Pager): String = ""

/**
 * `window.open` 打开同源静态页（官方设备管理页 `/em/`）。
 * 被浏览器弹窗拦截时 `window.open` 返回 null；它也可能"成功但返回 null"，
 * 所以 null 只作为失败信号（调用方提示改用直接访问 URL）。
 */
actual fun openAdminUrl(url: String, newTab: Boolean): Boolean = try {
    val opened = window.open(url, if (newTab) "_blank" else "_self")
    opened != null
} catch (_: Throwable) {
    false
}

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
