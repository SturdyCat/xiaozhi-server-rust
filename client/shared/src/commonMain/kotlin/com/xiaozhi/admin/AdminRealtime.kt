package com.xiaozhi.admin

import com.xiaozhi.admin.ui.*

import com.tencent.kuikly.core.nvi.serialization.json.JSONObject
import com.tencent.kuikly.core.pager.Pager
import com.tencent.kuikly.core.timer.clearTimeout
import com.tencent.kuikly.core.timer.setTimeout

/** 断线重连退避起点（毫秒）：3s 起步，逐次 ×2，封顶 [RT_BACKOFF_MAX_MS]。 */
private const val RT_BACKOFF_INIT_MS = 3_000L

/** 断线重连退避上限（毫秒）。 */
private const val RT_BACKOFF_MAX_MS = 30_000L

/** 应用层心跳间隔（毫秒）：连接期间每 25s 发一帧 ping（浏览器 WS 无协议层 ping）。 */
private const val RT_HEARTBEAT_MS = 25_000

/** 心跳帧。服务端契约：`{"type":"ping"}` → 回 `{"type":"pong"}`，未知/非法帧不绊断连接。 */
private const val RT_PING_FRAME = "{\"type\":\"ping\"}"

/** 管理台 WS 路径（服务端契约：连接建立即推 hello，设备面变化推 echomuse_devices_changed）。 */
private const val RT_WS_PATH = "/api/admin/ws"

/**
 * 当前平台是否具备直连 WebSocket 的能力（[AdminRealtime] 据此短路）：
 * Android 宿主未接 WS（false，订阅退化为「进页刷新一次」），其余平台 true。
 */
expect val PLATFORM_WS_SUPPORTED: Boolean

/**
 * 管理台 WS 端点：`base`（`http(s)://host[:port]`）换 ws(s) scheme 后拼 [RT_WS_PATH]；
 * `base` 为空串（web 由 server 同域托管）→ 平台从当前页面 origin 推导。
 * `token` 非空时编码进 `?token=` 查询参数——浏览器 WS 无法自定义请求头，与服务端
 * `auth_ok` 的查询参数兜底口径一致（token 为空 = 服务端未设管理令牌）。
 */
expect fun adminRealtimeUrl(base: String, token: String): String

/**
 * 平台 WebSocket 原语：各平台 actual 直连系统 API（web=浏览器 WebSocket、
 * mac/iOS=NSURLSessionWebSocketTask、Android 短路），**不经 XiaoZhiModule 原生桥**
 * （那是 macOS 测试台专用通道，管理台实时通道自持）。
 *
 * 契约：`onOpen`/`onText`/`onClosed` 必须投递到平台主线程（Kuikly 线程）回调；
 * `onClosed` 至多回调一次（正常关闭/错误/底层断开都算），之后共享层自行管理重连。
 */
expect class PlatformWsSocket(
    url: String,
    onOpen: () -> Unit,
    onText: (String) -> Unit,
    onClosed: () -> Unit,
) {
    /** 发一帧文本（心跳用）。未连接时由平台实现静默丢弃。 */
    fun send(text: String)

    /** 主动关闭。重连与否由共享层（[AdminRealtime]）管理，实现方不做重连语义。 */
    fun close()
}

/**
 * 管理台**实时通道**共享层：把各平台 WebSocket actual 收敛成「订阅制」事件分发，
 * 页面只管 subscribe / close，不感知连接生命周期。
 *
 * - **多订阅者共享连接**：按 base+token 分组分 socket，同组复用同一条连接，引用计数
 *   归零才真正断开；base 或 token 变了旧组归零自动断开、新组另起连接（断旧连新）；
 * - **分发**：文本帧按 `type` 分发给该连接上的所有订阅者（hello/pong 也分发，
 *   订阅者自行忽略不关心的类型）；非法帧静默丢弃（服务端会回 error 并保持连接）；
 * - **断线自动重连**：3s 起步 ×2 封顶 30s，仅在仍有订阅者时；成功 open 后重置退避；
 * - **代际保护**：每次断开/关停递增代际计数器，残留的 setTimeout 回调（重连/心跳）
 *   携带的代际过期即放弃，杜绝 close 后幽灵重连；
 * - **心跳**：连接期间每 25s 发一帧 ping（Kuikly setTimeout 链，同样受代际保护）；
 * - `PLATFORM_WS_SUPPORTED=false` 的平台 subscribe 返回空订阅，行为退化为
 *   「进页刷新一次」，不循环重试。
 *
 * 管理台与页面同生命周期，实例由页面 State 持有一份即可。
 */
class AdminRealtime {

    /** 一次事件订阅。所有 type 都会分发，订阅者自行忽略不关心的类型。 */
    interface Subscription {
        /** 退订（幂等）。最后一个订阅者退订时真正断开共享连接。 */
        fun close()
    }

    /** 平台不支持 WS 时的空订阅。 */
    private object Unsupported : Subscription {
        override fun close() {}
    }

    /** 按 base+token 分组的共享连接。 */
    private val groups = mutableMapOf<String, Group>()

    /**
     * 订阅管理台 WS 事件。同 base+token 的订阅共享同一条 socket（引用计数）；
     * 关闭订阅用返回值的 [Subscription.close]。token 变了 = 另起连接组（旧组
     * 引用计数归零自动断开），换令牌后旧连接不会带着过期凭据继续收事件。
     */
    fun subscribe(
        ctx: Pager,
        base: String,
        token: String,
        onEvent: (type: String, data: JSONObject) -> Unit,
    ): Subscription {
        if (!PLATFORM_WS_SUPPORTED) return Unsupported
        val key = "$base|$token"
        val sub = SubscriptionImpl(key, onEvent)
        val group = groups.getOrPut(key) { Group(ctx, base, token) }
        group.subs += sub
        group.ensureConnected()
        return sub
    }

    private inner class SubscriptionImpl(
        private val key: String,
        private val onEvent: (type: String, data: JSONObject) -> Unit,
    ) : Subscription {
        private var closed = false

        /** 共享层分发入口（内部）：已退订的订阅不再回调。 */
        fun dispatch(type: String, data: JSONObject) {
            if (!closed) onEvent(type, data)
        }

        override fun close() {
            if (closed) return
            closed = true
            val group = groups[key] ?: return
            group.subs.remove(this)
            if (group.subs.isEmpty()) {
                groups.remove(key)
                group.shutdown() // 引用计数归零才真正断开
            }
        }
    }

    private inner class Group(
        private val ctx: Pager,
        private val base: String,
        private val token: String,
    ) {
        val subs = mutableListOf<SubscriptionImpl>()

        /** 代际计数器：每次断开/关停递增；残留回调携带的代际过期即放弃。 */
        private var generation = 0
        private var socket: PlatformWsSocket? = null

        /** 下一次重连的退避间隔（成功 open 后重置）。 */
        private var backoffMs = RT_BACKOFF_INIT_MS
        private var reconnectRef: String? = null
        private var heartbeatRef: String? = null

        fun ensureConnected() {
            if (socket != null) return
            val gen = generation
            socket = PlatformWsSocket(
                adminRealtimeUrl(base, token),
                onOpen = {
                    if (gen == generation) {
                        backoffMs = RT_BACKOFF_INIT_MS
                        armHeartbeat(gen)
                    }
                },
                onText = { raw ->
                    if (gen == generation) dispatchFrame(raw)
                },
                onClosed = {
                    // socket==null 兜底：平台 once 保证失败时此处也只进一次
                    if (gen == generation && socket != null) {
                        socket = null
                        disarmHeartbeat()
                        scheduleReconnect(gen)
                    }
                },
            )
        }

        fun shutdown() {
            generation++
            cancel(reconnectRef)
            reconnectRef = null
            disarmHeartbeat()
            val s = socket
            socket = null
            s?.close() // close 后残留回调因代际过期而放弃，不会幽灵重连
        }

        private fun scheduleReconnect(gen: Int) {
            if (subs.isEmpty()) return
            val delay = backoffMs
            backoffMs = (backoffMs * 2).coerceAtMost(RT_BACKOFF_MAX_MS)
            reconnectRef = ctx.setTimeout(delay.toInt()) {
                reconnectRef = null
                if (gen != generation || subs.isEmpty()) return@setTimeout
                ensureConnected()
            }
        }

        /** 心跳链：到期发 ping 后自续期；断线/关停 disarm，代际过期即放弃。 */
        private fun armHeartbeat(gen: Int) {
            disarmHeartbeat()
            heartbeatRef = ctx.setTimeout(RT_HEARTBEAT_MS) {
                heartbeatRef = null
                if (gen != generation) return@setTimeout
                socket?.send(RT_PING_FRAME)
                armHeartbeat(gen)
            }
        }

        private fun disarmHeartbeat() {
            cancel(heartbeatRef)
            heartbeatRef = null
        }

        private fun dispatchFrame(raw: String) {
            val data = try {
                JSONObject(raw)
            } catch (_: Throwable) {
                null // 非法帧：服务端会回 error 并保持连接，这里静默忽略
            } ?: return
            val type = data.optString("type", "")
            if (type.isEmpty()) return
            // 快照迭代：订阅者在回调里 close 自己（或他人）不撕坏遍历
            for (sub in subs.toList()) sub.dispatch(type, data)
        }

        private fun cancel(ref: String?) {
            if (ref != null) ctx.clearTimeout(ref)
        }
    }
}
