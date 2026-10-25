package com.xiaozhi.admin

import com.xiaozhi.admin.ui.*

/** Android：宿主未接 WS 能力。PLATFORM_WS_SUPPORTED=false → 共享层 subscribe 直接短路。 */
actual val PLATFORM_WS_SUPPORTED: Boolean = false

/** 尽力值：base 换 scheme 拼固定路径（实际不会被用到——subscribe 已被短路）。 */
actual fun adminRealtimeUrl(base: String, token: String, path: String): String {
    val b = base.ifEmpty { "http://127.0.0.1:8000" }
    val scheme = if (b.startsWith("https://", ignoreCase = true)) "wss" else "ws"
    val url = "$scheme://${b.substringAfter("://")}$path"
    return if (token.isEmpty()) url else "$url?token=${echoMuseUrlComponent(token)}"
}

/**
 * 不会被共享层构造（SUPPORTED=false 先短路）；构造即回调 onClosed 一次兜底，
 * 保证若被误用也不会挂死在「永远连接中」。
 */
actual class PlatformWsSocket actual constructor(
    url: String,
    onOpen: () -> Unit,
    onText: (String) -> Unit,
    onClosed: () -> Unit,
) {
    init {
        onClosed()
    }

    actual fun send(text: String) {
        // 无 WS 能力：静默丢弃
    }

    actual fun close() {
        // 无 WS 能力：空实现
    }
}
