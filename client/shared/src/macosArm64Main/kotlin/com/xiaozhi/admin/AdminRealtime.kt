package com.xiaozhi.admin

import platform.Foundation.NSData
import platform.Foundation.NSURL
import platform.Foundation.NSURLSession
import platform.Foundation.NSURLSessionConfiguration
import platform.Foundation.NSURLSessionWebSocketCloseCodeNormalClosure
import platform.Foundation.NSURLSessionWebSocketDelegateProtocol
import platform.Foundation.NSURLSessionWebSocketMessage
import platform.Foundation.NSURLSessionWebSocketTask
import platform.Foundation.create
import platform.darwin.NSObject
import platform.darwin.dispatch_async
import platform.darwin.dispatch_get_main_queue

/** macOS 客户端：经 Kotlin/Native ObjC 互操作直连系统 WebSocket，支持。 */
actual val PLATFORM_WS_SUPPORTED: Boolean = true

/**
 * WS 端点：`base` 换 ws(s) scheme 拼 /api/admin/ws。mac 的 base 恒非空
 * （pagerBaseUrl 读 SP 记忆地址，空则回退本地默认）；仍空时兜底本地默认地址。
 * `token` 非空编码进查询参数（服务端 auth_ok 的 `?token=` 兜底口径）。
 */
actual fun adminRealtimeUrl(base: String, token: String): String {
    val b = base.ifEmpty { "http://127.0.0.1:8000" }
    val scheme = if (b.startsWith("https://", ignoreCase = true)) "wss" else "ws"
    val url = "$scheme://${b.substringAfter("://")}/api/admin/ws"
    return if (token.isEmpty()) url else "$url?token=${echoMuseUrlComponent(token)}"
}

/**
 * 为什么不经 XiaoZhiModule 原生桥：该桥是 macOS **测试台**专用通道（测试台 WS/麦克风/
 * Opus 都挂在 macApp 的 ObjC 侧实现上，web 端没有对应桥）。管理台实时通道要求 web 与
 * mac 行为一致且互不拖累，故 mac 侧直接用 Kotlin/Native 互操作持有自己的
 * NSURLSessionWebSocketTask，不引入任何宿主依赖。
 */

/** NSURLSession 回调在自建后台队列：统一切回主队列（Kuikly 线程）再进共享层。 */
private fun notifyMain(block: () -> Unit) {
    dispatch_async(dispatch_get_main_queue(), block)
}

/**
 * WebSocket 生命周期代理：didOpen → onOpen；didClose → onClosed。
 * delegate 由 NSURLSession 持强引用（session 构造传入），无提前释放问题。
 */
private class WsDelegate(
    private val onOpen: () -> Unit,
    private val onClosed: () -> Unit,
) : NSObject(), NSURLSessionWebSocketDelegateProtocol {

    override fun URLSession(
        session: NSURLSession,
        webSocketTask: NSURLSessionWebSocketTask,
        didOpenWithProtocol: String?,
    ) {
        notifyMain(onOpen)
    }

    override fun URLSession(
        session: NSURLSession,
        webSocketTask: NSURLSessionWebSocketTask,
        didCloseWithCode: Long,
        reason: NSData?,
    ) {
        notifyMain(onClosed)
    }
}

/**
 * NSURLSessionWebSocketTask 原语：收帧走 receiveMessageWithCompletionHandler 循环
 * （nil 回包 = 关闭/出错，统一归并为一次 onClosed）；发帧 sendMessage；关闭
 * cancelWithCloseCode（挂起的 receive 随之回调 error → onClosed，once 标记兜底）。
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

    private val session = NSURLSession.sessionWithConfiguration(
        NSURLSessionConfiguration.defaultSessionConfiguration,
        delegate = WsDelegate(
            onOpen = { notifyMain(onOpenCb) },
            onClosed = { reportClosed() },
        ),
        delegateQueue = null, // session 自建串行队列，回调统一经 notifyMain 切主线程
    )

    private val task: NSURLSessionWebSocketTask?

    init {
        val nsUrl = NSURL(string = url)
        if (nsUrl == null) {
            // URL 非法：构造即失败，onClosed 一次（共享层据此走重连退避）
            task = null
            reportClosed()
        } else {
            val t = session.webSocketTaskWithURL(nsUrl)
            task = t
            receiveLoop(t)
            t.resume()
        }
    }

    actual fun send(text: String) {
        // completionHandler 为必填参数（Kotlin/Native 无单参重载）；结果不影响共享层语义。
        task?.sendMessage(NSURLSessionWebSocketMessage(text)) { }
    }

    actual fun close() {
        task?.cancelWithCloseCode(NSURLSessionWebSocketCloseCodeNormalClosure, null)
    }

    /**
     * 挂收一帧；文本帧回调后继续挂收，nil 回包（关闭/出错）归并为一次 onClosed。
     * 文本帧以 `string` 非空判定（服务端契约全为 JSON 文本；ping/data 帧不进该字段）。
     */
    private fun receiveLoop(t: NSURLSessionWebSocketTask) {
        t.receiveMessageWithCompletionHandler { message, _ ->
            val text = message?.string
            if (text != null) {
                notifyMain { onTextCb(text) }
                receiveLoop(t)
            } else {
                reportClosed()
            }
        }
    }

    private fun reportClosed() {
        if (closedReported) return
        closedReported = true
        notifyMain(onClosedCb)
    }
}
