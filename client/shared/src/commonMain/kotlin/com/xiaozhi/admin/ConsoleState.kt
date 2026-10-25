package com.xiaozhi.admin

import com.xiaozhi.admin.ui.*
import com.tencent.kuikly.core.nvi.serialization.json.JSONObject
import com.tencent.kuikly.core.pager.Pager
import com.tencent.kuikly.core.reactive.handler.observable
import com.tencent.kuikly.core.reactive.handler.observableList
import com.tencent.kuikly.core.timer.clearTimeout
import com.tencent.kuikly.core.timer.setTimeout

/**
 * 管理台「控制台」数据源（官方 dashboard 的 CONSOLE / 命令行）：连服务端全局实时日志流
 * `GET /api/logs`（`server/src/gateway/ws/log_stream.rs` 的 tracing 镜像流），连接后先收
 * `backlog` 补最近历史、再逐条收 `log` 帧实时追加。本类只管连 / 收 / 缓冲 / 筛选，渲染见
 * [renderConsole]。
 *
 * 与 [AdminRealtime]（管理台事件通道 `/api/admin/ws`）是**两条独立 WS**：后者推设备面版本号
 * 等结构化事件，本类推的是扁平的日志文本行，帧协议不同，故不复用。
 *
 * 平台差异：`PLATFORM_WS_SUPPORTED=false`（Android / iOS 宿主未接原生桥）时 [connect] 直接置
 * 「不支持」状态、不发连接——与 [AdminRealtime] 的退化口径一致。
 */
class ConsoleState(private val ctx: Pager) {
    /** 当前可见文本（已按级别筛选、按上限截断）。改它即触发重渲染。 */
    var display by observable("")

    /** 连接状态文案：未连接 / 连接中… / 已连接 / 已断开 · 重连中… / 不支持。 */
    var status by observable("未连接")

    /** 是否连上（影响「复制 / 清空 / 暂停」可用性与状态字色）。 */
    var connected by observable(false)

    /** 暂停：冻结画面（连接保持，新行进暂停缓冲，继续后一次性补上）。便于盯着某段排查。 */
    var paused by observable(false)

    /** 级别筛选：all / warn / error。 */
    var levelFilter by observable("all")

    /** 级别筛选下拉展开态（[dropdownField] 需要）。 */
    var levelOpen by observable(false)

    /** 连接地址与令牌（[connect] 时记下，供「重连」按钮复用）。 */
    internal var apiBase = ""
    internal var apiToken = ""

    /** 级别筛选下拉选项（[dropdownField] 的 `options` 要求 ObservableList）。 */
    var levelOptions by observableList<Pair<String, String>>()
    init {
        levelOptions.add("all" to "全部级别")
        levelOptions.add("warn" to "警告及以上")
        levelOptions.add("error" to "仅错误")
    }

    private var socket: PlatformWsSocket? = null
    private val lines = mutableListOf<String>()       // 已显示（受上限约束）
    private val pausedBuffer = mutableListOf<String>() // 暂停期间累积
    private val pending = mutableListOf<String>()      // 待刷入（节流）
    private var flushRef: String? = null
    private var reconnectRef: String? = null
    private var backoff = CONSOLE_BACKOFF_INIT
    private var stopped = false
    private var generation = 0

    /** 连上日志流；重复调用 = 重连（关旧连、清计时器、开新连）。 */
    fun connect(base: String, token: String) {
        apiBase = base
        apiToken = token
        stopped = false
        generation++
        socket?.close()
        socket = null
        cancel(reconnectRef); reconnectRef = null
        openOnce()
    }

    private fun openOnce() {
        if (stopped) return
        if (!PLATFORM_WS_SUPPORTED) {
            status = "当前平台暂不支持实时日志（Android / iOS 客户端未接入 WebSocket）"
            return
        }
        if (!connected) status = "连接中…"
        val gen = generation
        val url = adminRealtimeUrl(apiBase, apiToken, "/api/logs")
        socket = PlatformWsSocket(
            url,
            onOpen = {
                if (gen == generation) {
                    connected = true
                    backoff = CONSOLE_BACKOFF_INIT
                    status = "已连接"
                }
            },
            onText = { raw -> if (gen == generation) onFrame(raw) },
            onClosed = {
                if (gen == generation) {
                    connected = false
                    socket = null
                    if (!stopped) {
                        status = "已断开 · 重连中…"
                        scheduleReconnect(gen)
                    }
                }
            },
        )
    }

    private fun scheduleReconnect(gen: Int) {
        if (stopped || gen != generation) return
        val delay = backoff
        backoff = (backoff * 2).coerceAtMost(CONSOLE_BACKOFF_MAX)
        reconnectRef = ctx.setTimeout(delay.toInt()) {
            reconnectRef = null
            if (gen == generation && !stopped) openOnce()
        }
    }

    private fun onFrame(raw: String) {
        val data = try {
            JSONObject(raw)
        } catch (_: Throwable) {
            null
        } ?: return
        when (data.optString("type", "")) {
            "backlog" -> {
                val arr = data.optJSONArray("lines") ?: return
                val incoming = (0 until arr.length()).mapNotNull { arr.optString(it) }
                applyLines(incoming)
            }
            "log" -> enqueue(data.optString("line", ""))
        }
    }

    private fun applyLines(list: List<String>) {
        lines.clear()
        lines.addAll(list.takeLast(CONSOLE_MAX_LINES))
        rebuild()
    }

    private fun enqueue(line: String) {
        if (line.isEmpty()) return
        pending.add(line)
        if (flushRef == null) {
            flushRef = ctx.setTimeout(CONSOLE_FLUSH_MS.toInt()) {
                flushRef = null
                flush()
            }
        }
    }

    /** 节流刷入：每 [CONSOLE_FLUSH_MS] 把累积的新行并入可见缓冲，避免高频日志逐帧重渲染。 */
    private fun flush() {
        if (pending.isEmpty()) return
        val batch = pending.toList()
        pending.clear()
        if (paused) {
            pausedBuffer.addAll(batch)
            if (pausedBuffer.size > CONSOLE_MAX_LINES * 2) {
                pausedBuffer.subList(0, pausedBuffer.size - CONSOLE_MAX_LINES * 2).clear()
            }
            return
        }
        appendBatch(batch)
    }

    private fun appendBatch(batch: List<String>) {
        lines.addAll(batch)
        if (lines.size > CONSOLE_MAX_LINES) {
            lines.subList(0, lines.size - CONSOLE_MAX_LINES).clear()
        }
        rebuild()
    }

    private fun rebuild() {
        val src = if (paused) pausedBuffer else lines
        display = src.filter { matchesLevel(it) }.joinToString("\n")
    }

    private fun matchesLevel(line: String): Boolean {
        if (levelFilter == "all") return true
        val lvl = extractLevel(line)
        return when (levelFilter) {
            "warn" -> lvl in setOf("WARN", "ERROR")
            "error" -> lvl == "ERROR"
            else -> true
        }
    }

    /** 行格式 `YYYY-MM-DD HH:MM:SS LEVEL [target] msg` → 取 LEVEL 词（第二个空白分隔段起、[target] 之前的词）。 */
    private fun extractLevel(line: String): String {
        val head = if (line.contains('[')) line.substringBefore('[') else line
        for (t in head.split(Regex("\\s+"))) {
            val u = t.uppercase()
            if (u in setOf("ERROR", "WARN", "INFO", "DEBUG", "TRACE")) return u
        }
        return ""
    }

    fun setLevel(level: String) {
        levelFilter = level
        rebuild()
    }

    fun togglePause() {
        paused = !paused
        if (!paused && pausedBuffer.isNotEmpty()) {
            // 继续：把暂停期间累积的行并入主缓冲（受上限约束），再重建
            val batch = pausedBuffer.toList()
            pausedBuffer.clear()
            appendBatch(batch)
        }
    }

    fun clear() {
        lines.clear()
        pausedBuffer.clear()
        pending.clear()
        display = ""
    }

    /** 断开并停重连（页面销毁时调用）。 */
    fun disconnect() {
        stopped = true
        generation++
        cancel(reconnectRef); reconnectRef = null
        cancel(flushRef); flushRef = null
        socket?.close()
        socket = null
        connected = false
    }

    private fun cancel(ref: String?) {
        if (ref != null) ctx.clearTimeout(ref)
    }
}

/** 可见行上限：超出丢弃最旧，避免内存与重渲染无限增长。 */
private const val CONSOLE_MAX_LINES = 2000

/** 刷入节流间隔（毫秒）：高频日志下每 200ms 重渲染一次。 */
private const val CONSOLE_FLUSH_MS = 200L

/** 断线重连退避起点（毫秒）。 */
private const val CONSOLE_BACKOFF_INIT = 3_000L

/** 断线重连退避上限（毫秒）。 */
private const val CONSOLE_BACKOFF_MAX = 30_000L
