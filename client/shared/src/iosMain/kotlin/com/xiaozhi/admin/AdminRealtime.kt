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

/**
 * iOS 宿主**关闭**实时通道：Kuikly Kotlin 侧只允许在 context 线程碰 observable/`setTimeout`，
 * 而 iOS 宿主（iosApp 模板）没有 XiaoZhiModule 原生桥，无法像 mac 那样借模块回调通道
 * （框架 `performOnContextQueue` 兜底切线程）把 WS 事件送回 context 线程——自建
 * NSURLSession 无论回调投到哪个队列都违反线程契约（mac 侧 2026-10 `assertContextQueue`
 * 卡死事故同根因，见 macosArm64Main/AdminRealtime.kt）。关闭后退化为「进页刷新一次」，
 * 不再让带病实现上线；iOS 宿主接入原生桥后可按 mac 的 AdminWs 方案重开。
 */
actual val PLATFORM_WS_SUPPORTED: Boolean = false

/**
 * WS 端点：`base` 换 ws(s) scheme 拼 /api/admin/ws。iOS 当前无宿主接管理台
 * （pagerBaseUrl 为空串），空 base 兜底本地默认地址（尽力值）。
 * `token` 非空编码进查询参数（服务端 auth_ok 的 `?token=` 兜底口径）。
 */
actual fun adminRealtimeUrl(base: String, token: String, path: String): String {
    val b = base.ifEmpty { "http://127.0.0.1:8000" }
    val scheme = if (b.startsWith("https://", ignoreCase = true)) "wss" else "ws"
    val url = "$scheme://${b.substringAfter("://")}$path"
    return if (token.isEmpty()) url else "$url?token=${echoMuseUrlComponent(token)}"
}

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
class IosWsSocket(
    url: String,
    onOpen: () -> Unit,
    onText: (String) -> Unit,
    onClosed: () -> Unit,
) : PlatformWsSocket {
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

    private val task: NSURLSessionWebSocketTask

    init {
        // cinterop 下 NSURL(string:) 返回非空类型（无效串由 task 自身连接失败兜底 → onClosed）。
        val t = session.webSocketTaskWithURL(NSURL(string = url))
        task = t
        receiveLoop(t)
        t.resume()
    }

    override fun send(text: String) {
        // completionHandler 为必填参数（Kotlin/Native 无单参重载）；结果不影响共享层语义。
        task?.sendMessage(NSURLSessionWebSocketMessage(text)) { }
    }

    override fun close() {
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

/** iOS 平台工厂（当前 PLATFORM_WS_SUPPORTED=false，共享层短路后不会被构造）。 */
actual fun platformWsSocket(
    url: String,
    onOpen: () -> Unit,
    onText: (String) -> Unit,
    onClosed: () -> Unit,
): PlatformWsSocket = IosWsSocket(url, onOpen, onText, onClosed)
