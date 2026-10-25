package com.xiaozhi.admin

import com.xiaozhi.admin.ui.*

/** macOS 客户端：经原生桥直连系统 WebSocket，支持。 */
actual val PLATFORM_WS_SUPPORTED: Boolean = true

/**
 * WS 端点：`base` 换 ws(s) scheme 拼 /api/admin/ws。mac 的 base 恒非空
 * （pagerBaseUrl 读 SP 记忆地址，空则回退本地默认）；仍空时兜底本地默认地址。
 * `token` 非空编码进查询参数（服务端 auth_ok 的 `?token=` 兜底口径）。
 */
actual fun adminRealtimeUrl(base: String, token: String, path: String): String {
    val b = base.ifEmpty { "http://127.0.0.1:8000" }
    val scheme = if (b.startsWith("https://", ignoreCase = true)) "wss" else "ws"
    val url = "$scheme://${b.substringAfter("://")}$path"
    return if (token.isEmpty()) url else "$url?token=${echoMuseUrlComponent(token)}"
}

/**
 * 为什么经 XiaoZhiModule 原生桥（而不再自建 NSURLSessionWebSocketTask）：
 *
 * Kuikly 的 Kotlin 侧运行在**独立 context 线程**（非主线程）——observable、
 * setTimeout 等 UI 侧 API 只允许在该线程调用。自建 NSURLSession 的回调无论切到
 * 哪个 GCD 队列都不是 context 线程，断线重连一路调用共享层 `ctx.setTimeout` 就会
 * 触发渲染器 `assertContextQueue` 断言（实测表现为管理台整体卡死）。
 * 而模块回调通道由框架兜底：原生在任意线程 invoke 回调 block，框架内部
 * `performOnContextQueueWithBlock` 切回 context 线程再进 Kotlin。因此 WS 的
 * 建连/收发全部下沉到原生侧（XiaoZhiModule+AdminWs.m），本类只保留状态机。
 *
 * 契约：`onOpen`/`onText`/`onClosed` 回调到达时已在 Kuikly context 线程；
 * `onClosed` 至多回调一次（正常关闭/错误/底层断开都算），之后共享层自行管理重连。
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

    /** 原生侧按 key 归属连接（断旧连新由共享层管理，key 恒定不冲突）。 */
    private val key = "admin-ws-${nextId++}"

    private val module = sharedBridge()

    init {
        if (module == null) {
            // 桥不在（理论不可达：本 actual 只在 mac App 编译，模块随壳注册）。
            // 按构造失败处理：立即 onClosed 一次，共享层走重连退避或停订。
            reportClosed()
        } else {
            module.adminWsConnect(key, url) { result ->
                when (result?.optString("event", "")) {
                    "open" -> onOpenCb()
                    "text" -> onTextCb(result.optString("data", ""))
                    "closed" -> reportClosed()
                }
            }
        }
    }

    actual fun send(text: String) {
        module?.adminWsSend(key, text)
    }

    actual fun close() {
        module?.adminWsClose(key)
    }

    private fun reportClosed() {
        if (closedReported) return
        closedReported = true
        onClosedCb()
    }

    companion object {
        private var nextId = 0
    }
}

/**
 * 取 XiaoZhiModule 桥：PlatformWsSocket 构造时没有 Pager（expect 契约固定），
 * 经 [sharedBridgeRef] 拿壳注册的模块实例（AdminRealtime 与模块同壳生命周期，
 * 且构造发生在 Kuikly 线程，读写无竞争）。桥缺失时按「连接立即失败」降级。
 * 强引用持有（与 benchStates 常驻 Pager 同规模）：Kotlin/Native WeakReference 属
 * ExperimentalNativeApi，为一个判空引用不值得开 opt-in。
 */
internal fun sharedBridge(): XiaoZhiModule? = sharedBridgeRef

private var sharedBridgeRef: XiaoZhiModule? = null

internal fun registerSharedBridge(module: XiaoZhiModule) {
    sharedBridgeRef = module
}
