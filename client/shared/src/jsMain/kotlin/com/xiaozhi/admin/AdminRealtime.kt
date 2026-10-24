package com.xiaozhi.admin

import com.xiaozhi.admin.ui.*

import kotlinx.browser.window
import org.w3c.dom.WebSocket

/** Web：浏览器原生 WebSocket（kotlinx.browser 直连，不经任何原生桥）。 */
actual val PLATFORM_WS_SUPPORTED: Boolean = true

/**
 * WS 端点：`base` 非空 → 换 ws(s) scheme 拼 /api/admin/ws；
 * 空串（server 同域托管）→ 从当前页面 location 推导（http→ws、https→wss）。
 * `token` 非空编码进查询参数（浏览器 WS 无法自定义请求头）。
 */
actual fun adminRealtimeUrl(base: String, token: String): String {
    val url = if (base.isEmpty()) {
        val scheme = if (window.location.protocol == "https:") "wss" else "ws"
        "$scheme://${window.location.host}/api/admin/ws"
    } else {
        httpToWsAuthority(base) + "/api/admin/ws"
    }
    return if (token.isEmpty()) url else "$url?token=${echoMuseUrlComponent(token)}"
}

/** `http(s)://host[:port]` → `ws(s)://host[:port]`（与 AdminNav/测试台统一地址口径一致）。 */
private fun httpToWsAuthority(base: String): String {
    val scheme = if (base.startsWith("https://", ignoreCase = true)) "wss" else "ws"
    return "$scheme://${base.substringAfter("://")}"
}

/**
 * 浏览器 WebSocket 原语：回调本就在 JS 主线程（Kuikly 线程），无需换线程；
 * onclose/onerror 都归并为一次 onClosed（浏览器在 error 后通常还会补发 close，
 * [reportClosed] 的 once 标记保证至多回调一次）。
 */
actual class PlatformWsSocket actual constructor(
    url: String,
    onOpen: () -> Unit,
    onText: (String) -> Unit,
    onClosed: () -> Unit,
) {
    private val onOpenCb = onOpen
    private val onTextCb = onText
    private val onClosedCb = onClosed
    private var closedReported = false

    private val ws = WebSocket(url)

    init {
        // 只消费文本帧（服务端契约全为 JSON 文本；二进制帧不进共享层）
        ws.onopen = { onOpenCb() }
        ws.onmessage = { event -> (event.data as? String)?.let(onTextCb) }
        ws.onclose = { reportClosed() }
        ws.onerror = { reportClosed() }
    }

    actual fun send(text: String) {
        ws.send(text)
    }

    actual fun close() {
        ws.close() // 浏览器随后触发 onclose → onClosed 一次（共享层据此收尾）
    }

    private fun reportClosed() {
        if (closedReported) return
        closedReported = true
        onClosedCb()
    }
}
