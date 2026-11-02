package com.xiaozhi.admin

import com.tencent.kuikly.core.coroutines.Job
import com.tencent.kuikly.core.coroutines.delay
import com.tencent.kuikly.core.coroutines.launch
import com.tencent.kuikly.core.nvi.serialization.json.JSONObject
import com.tencent.kuikly.core.pager.Pager
import com.tencent.kuikly.core.timer.clearTimeout
import com.tencent.kuikly.core.timer.setTimeout
import com.xiaozhi.admin.state.stateVar

const val EM_CONSOLE_CONNECTING = "connecting"
const val EM_CONSOLE_READY = "ready"
const val EM_CONSOLE_OFFLINE = "offline"
const val EM_CONSOLE_BUSY = "busy"
const val EM_CONSOLE_CLOSED = "closed"
const val EM_CONSOLE_UNSUPPORTED = "unsupported"
const val EM_CONSOLE_MODE_PIPE = "pipe"
const val EM_CONSOLE_MODE_PTY = "pty"
const val EM_CONSOLE_LINE_MAX = 4096
const val EM_CONSOLE_FLUSH_MS = 50
const val EM_CONSOLE_HISTORY_MAX = 100
const val EM_CONSOLE_PING_MS = 25_000

internal data class EchoMuseConsoleFrame(
    val lines: List<TerminalLine>, val history: List<TerminalLine>,
    val cursorRow: Int, val cursorCol: Int, val cursorVisible: Boolean,
    val cols: Int, val rows: Int,
)

/** Single exclusive device shell lease. All methods/callbacks run on the Kuikly context thread.
 * The JSON transport remains compatible with line-only clients; raw_input is explicitly negotiated.
 * A real VT screen replaces the former ANSI-stripped append-only log. No iframe, binary WS or
 * second configuration/request chain is needed. Never automatically reconnect a privileged shell.
 */
class EchoMuseConsoleState(private val ctx: Pager) {
    internal val terminal = EchoMuseTerminalScreen()
    internal var frame by stateVar(snapshot())
        private set
    var phase by stateVar(EM_CONSOLE_CONNECTING)
        private set
    var notice by stateVar("")
        private set
    var hint by stateVar("")
        private set
    var insecure by stateVar(false)
        private set
    var mode by stateVar(EM_CONSOLE_MODE_PIPE)
        private set
    var input by stateVar("")
        private set
    var autoScroll by stateVar(true)
    var closing by stateVar("")
        private set
    var closingStats by stateVar("")
        private set

    private var socket: PlatformWsSocket? = null
    private var flushRef: String? = null
    private var heartbeat: Job? = null
    private var generation = 0
    private var deviceId = ""
    private var base = ""
    private var token = ""
    private var lastOnline = false
    private var session = ""
    private var opened = false
    private var rawInput = false
    private var resizeSupported = false
    private val history = mutableListOf<String>()
    private var historyCursor = -1
    val canInput: Boolean get() = phase == EM_CONSOLE_READY
    val interactive: Boolean get() = canInput && mode == EM_CONSOLE_MODE_PTY && rawInput
    val canReconnect: Boolean get() = phase in setOf(EM_CONSOLE_OFFLINE, EM_CONSOLE_BUSY, EM_CONSOLE_CLOSED)
    val display: String get() = (frame.history + frame.lines).joinToString("\n") { it.text.trimEnd() }.trimEnd()
    val phaseText: String get() = when (phase) {
        EM_CONSOLE_CONNECTING -> ConsoleCopy.CONNECTING
        EM_CONSOLE_READY -> if (interactive) "已连接 · PTY 终端" else "已连接 · 兼容行模式"
        EM_CONSOLE_OFFLINE -> ConsoleCopy.OFFLINE
        EM_CONSOLE_BUSY -> hint.ifEmpty { ConsoleCopy.BUSY }
        EM_CONSOLE_UNSUPPORTED -> ConsoleCopy.UNSUPPORTED
        else -> hint.ifEmpty { ConsoleCopy.RECONNECT_LOST }
    }

    fun open(deviceId: String, online: Boolean, baseUrl: String, token: String) {
        val same = opened && this.deviceId == deviceId && base == baseUrl && this.token == token
        lastOnline = online
        if (same && (phase == EM_CONSOLE_CONNECTING || phase == EM_CONSOLE_READY)) return
        close()
        this.deviceId = deviceId; base = baseUrl; this.token = token; opened = true
        if (!same) {
            terminal.clear(); history.clear(); historyCursor = -1; input = ""; session = ""
            insecure = false; autoScroll = true; publish()
        }
        rawInput = false; resizeSupported = false; mode = EM_CONSOLE_MODE_PIPE; closing = ""; closingStats = ""; hint = ""; notice = ""
        if (!online) { phase = EM_CONSOLE_OFFLINE; return }
        if (!PLATFORM_WS_SUPPORTED) { phase = EM_CONSOLE_UNSUPPORTED; return }
        phase = EM_CONSOLE_CONNECTING
        val gen = generation
        val path = "/api/echomuse/devices/${echoMuseUrlComponent(deviceId)}/console"
        socket = platformWsSocket(adminRealtimeUrl(base, token, path),
            onOpen = {},
            onText = { if (gen == generation) onFrame(it) },
            onClosed = {
                if (gen == generation) {
                    socket = null; stopTimers(); publish()
                    if (phase == EM_CONSOLE_READY || phase == EM_CONSOLE_CONNECTING) {
                        phase = EM_CONSOLE_CLOSED; hint = ConsoleCopy.RECONNECT_LOST
                    }
                }
            })
    }

    /** Live list changes invalidate a shell immediately, but never silently acquire a new one. */
    fun updateDevice(online: Boolean, sessionId: String) {
        lastOnline = online
        if (!online && (phase == EM_CONSOLE_READY || phase == EM_CONSOLE_CONNECTING)) {
            close(); phase = EM_CONSOLE_OFFLINE; hint = ConsoleCopy.DEVICE_LOST
        } else if (session.isNotEmpty() && sessionId != session &&
            (phase == EM_CONSOLE_READY || phase == EM_CONSOLE_CONNECTING)) {
            close(); hint = ConsoleCopy.SUPERSEDED
        }
        session = sessionId
    }

    fun close() {
        generation++; stopTimers(); publish()
        socket?.close(); socket = null
        if (phase == EM_CONSOLE_READY || phase == EM_CONSOLE_CONNECTING) phase = EM_CONSOLE_CLOSED
    }
    fun reconnect() { if (deviceId.isNotEmpty()) open(deviceId, lastOnline, base, token) }

    fun key(key: String, ctrl: Boolean, alt: Boolean, shift: Boolean, meta: Boolean) {
        val text = EchoMuseConsoleKeys.encode(key, ctrl, alt, shift, meta, terminal.applicationCursorKeys) ?: return
        if (!interactive && key == "ArrowUp") { historyPrev(); return }
        if (!interactive && key == "ArrowDown") { historyNext(); return }
        type(text)
    }

    fun type(text: String, paste: Boolean = false) {
        if (!canInput || text.isEmpty()) return
        // Bounded paste, never truncate a command silently or execute half of an oversized paste.
        if (text.encodeToByteArray().size > 32_768) { hint = "粘贴过长（上限 32 KiB），未发送"; return }
        autoScroll = true
        if (interactive) {
            val data = if (paste && terminal.bracketedPaste) "\u001b[200~${text.replace("\u001b", "")}\u001b[201~" else text
            for (chunk in EchoMuseConsoleKeys.chunks(data, maxBytes = 2048)) sendInput(chunk, raw = true)
        } else {
            // Legacy devices/servers: explicit local line editing, as in upstream's pipe branch.
            val normalized = text.replace("\r\n", "\r").replace('\n', '\r')
            if (paste && normalized.contains('\r')) { hint = "行模式不自动执行多行粘贴，请逐行输入"; return }
            for (ch in normalized) when {
                ch == '\r' -> submit()
                ch == '\u0003' -> { interrupt(); input = ""; terminal.write("^C\r\n") }
                ch == '\u007f' || ch == '\b' -> if (input.isNotEmpty()) {
                    val n = if (input.last().isLowSurrogate() && input.length > 1) 2 else 1
                    val last = input.takeLast(n); input = input.dropLast(n)
                    terminal.write(if (last[0].code in 0x2e80..0xffff) "\b\b  \b\b" else "\b \b")
                }
                ch >= ' ' -> {
                    if ((input + ch).encodeToByteArray().size > EM_CONSOLE_LINE_MAX) hint = ConsoleCopy.INPUT_TOO_LONG
                    else { input += ch; terminal.write(ch.toString()) }
                }
            }
            scheduleFlush()
        }
    }

    fun submit() {
        if (!canInput || input.encodeToByteArray().size > EM_CONSOLE_LINE_MAX) return
        if (sendInput(input, raw = false)) {
            if (input.isNotEmpty() && history.lastOrNull() != input) history.add(input)
            while (history.size > EM_CONSOLE_HISTORY_MAX) history.removeAt(0)
            historyCursor = -1; input = ""; hint = ""; terminal.write("\r\n"); scheduleFlush()
        }
    }
    fun interrupt() { if (canInput) send(ConsoleWire.CTRL_C_FRAME) }
    fun clearBuffer() { terminal.clear(); publish(); hint = "" }
    fun copyAll(): String { publish(); return display }
    private fun replaceInput(value: String) {
        repeat(input.length) { terminal.write("\b \b") }; input = value; terminal.write(value); scheduleFlush()
    }
    fun historyPrev() {
        if (!canInput || history.isEmpty()) return
        historyCursor = if (historyCursor < 0) history.lastIndex else (historyCursor - 1).coerceAtLeast(0)
        replaceInput(history[historyCursor])
    }
    fun historyNext() {
        if (!canInput || historyCursor < 0) return
        historyCursor++
        if (historyCursor >= history.size) { historyCursor = -1; replaceInput("") }
        else replaceInput(history[historyCursor])
    }

    fun resize(cols: Int, rows: Int) {
        val c = cols.coerceIn(2, 500); val r = rows.coerceIn(2, 200)
        if (terminal.cols == c && terminal.rows == r) return
        terminal.resize(c, r); publish(); sendResize()
    }
    private fun sendResize() {
        if (interactive && resizeSupported) send(JSONObject().apply {
            put("type", "resize"); put("cols", terminal.cols); put("rows", terminal.rows)
        }.toString())
    }
    private fun sendInput(data: String, raw: Boolean): Boolean = send(JSONObject().apply {
        put("type", ConsoleWire.INPUT); put("data", data); if (raw) put("raw", true)
    }.toString())

    private fun onFrame(raw: String) {
        val data = try { JSONObject(raw) } catch (_: Throwable) { return }
        when (data.optString("type", "")) {
            ConsoleWire.READY -> {
                phase = EM_CONSOLE_READY; insecure = data.optBoolean("insecure", false)
                rawInput = data.optBoolean("raw_input", false); resizeSupported = data.optBoolean("resize", false)
                mode = data.optString("mode", EM_CONSOLE_MODE_PIPE)
                notice = data.text("notice"); hint = ""; closing = ""; closingStats = ""
                if (mode == EM_CONSOLE_MODE_PTY && !rawInput) notice = "服务端为兼容行协议；更新服务端后可用完整 PTY 按键和窗口尺寸同步。"
                sendResize(); armHeartbeat()
            }
            ConsoleWire.SHELL_META -> {
                mode = if (data.optBoolean("pty", false)) EM_CONSOLE_MODE_PTY else EM_CONSOLE_MODE_PIPE
                sendResize()
            }
            ConsoleWire.OUTPUT -> {
                terminal.write(data.text("text"))
                for (response in terminal.takeResponses()) if (interactive) sendInput(response, raw = true)
                scheduleFlush()
            }
            ConsoleWire.NOTICE -> notice = data.text("text").ifEmpty { ConsoleCopy.CAPPED }
            ConsoleWire.ERROR -> {
                val code = data.text("code"); val message = data.text("error")
                when (code) {
                    ConsoleWire.CODE_INPUT_TOO_LONG, ConsoleWire.CODE_RATE_LIMITED,
                    ConsoleWire.CODE_UNSUPPORTED_MODE, ConsoleWire.CODE_INVALID_RESIZE -> hint = message
                    else -> {
                        phase = if (code == ConsoleWire.CODE_BUSY) EM_CONSOLE_BUSY else EM_CONSOLE_CLOSED
                        hint = message.ifEmpty { if (code == ConsoleWire.CODE_BUSY) ConsoleCopy.BUSY else ConsoleCopy.RECONNECT_LOST }
                        stopTimers(); publish(); generation++; socket?.close(); socket = null
                    }
                }
            }
            ConsoleWire.CLOSED -> {
                closing = data.text("reason"); hint = ConsoleHint.closed(closing, data.text("detail"))
                closingStats = "输入 ${data.optInt("inputs", 0)} 次 · 输出 ${data.optLong("bytes_out", 0)} 字节"
                phase = EM_CONSOLE_CLOSED; stopTimers(); publish(); generation++; socket?.close(); socket = null
            }
        }
    }

    private fun scheduleFlush() {
        if (flushRef != null) return
        flushRef = ctx.setTimeout(EM_CONSOLE_FLUSH_MS) { flushRef = null; publish() }
    }
    private fun snapshot() = EchoMuseConsoleFrame(terminal.snapshot(), terminal.scrollbackSnapshot(),
        terminal.cursorRow, terminal.cursorCol, terminal.cursorVisible, terminal.cols, terminal.rows)
    private fun publish() { frame = snapshot() }
    private fun armHeartbeat() {
        heartbeat?.cancel()
        heartbeat = ctx.lifecycleScope.launch {
            while (true) { delay(EM_CONSOLE_PING_MS); send(ConsoleWire.PING_FRAME) }
        }
    }
    private fun stopTimers() {
        flushRef?.let { ctx.clearTimeout(it) }; flushRef = null; heartbeat?.cancel(); heartbeat = null
    }
    private fun send(text: String): Boolean {
        val active = socket ?: return false
        active.send(text); return true
    }
}
