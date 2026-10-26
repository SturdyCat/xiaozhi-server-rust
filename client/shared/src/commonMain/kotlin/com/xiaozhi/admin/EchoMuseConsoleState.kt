package com.xiaozhi.admin

import com.xiaozhi.admin.ui.*

import com.tencent.kuikly.core.nvi.serialization.json.JSONObject
import com.tencent.kuikly.core.pager.Pager
import com.tencent.kuikly.core.reactive.handler.observable
import com.tencent.kuikly.core.timer.clearTimeout
import com.tencent.kuikly.core.timer.setTimeout

/**
 * 六种连接态（设计 §6.4 表）。**取值即文案键**：`connecting`/`ready`/`offline`/`busy`/
 * `closed`/`unsupported`，渲染层只比这些常量，不写裸字符串。
 */
const val EM_CONSOLE_CONNECTING = "connecting"
const val EM_CONSOLE_READY = "ready"
const val EM_CONSOLE_OFFLINE = "offline"
const val EM_CONSOLE_BUSY = "busy"
const val EM_CONSOLE_CLOSED = "closed"
const val EM_CONSOLE_UNSUPPORTED = "unsupported"

/** `ready.mode` 一期唯一值（§8.1：留着字段而非布尔，为将来 pty 模式前向兼容）。 */
const val EM_CONSOLE_MODE_PIPE = "pipe"

/** 单行输入上限（字节，§8.2 `LINE_MAX`）：超了**不发**，只在输入框下提示。 */
const val EM_CONSOLE_LINE_MAX = 4096

/** 输出合并刷新间隔（毫秒，§8.2 `FLUSH_MS`）。 */
const val EM_CONSOLE_FLUSH_MS = 200

/** ↑↓ 历史条数上限（§8.2 `HISTORY_MAX`，仅本会话内存）。 */
const val EM_CONSOLE_HISTORY_MAX = 100

/** 应用层心跳间隔（毫秒）：与 [AdminRealtime] 的 25 s 同值（浏览器 WS 无协议层 ping）。 */
const val EM_CONSOLE_PING_MS = 25_000

/**
 * 设备控制台（emOS 受限 shell 的行模式终端）的**状态机**：WS 通道、六种连接态、
 * 输入/中断/清屏/复制、↑↓ 历史、200 ms 合并刷新、手工重连。
 *
 * 与 [ConsoleState]（管理台全局日志流 `/api/logs`）是**两条不同的 WS**，帧协议不同
 * （那边是 `backlog`/`log` 单行文本，这边是 §5.2 的结构化帧），故不复用；但**文本管线
 * 复用同一份**（[EchoMuseConsoleText]，剥离/覆盖/上限全在纯函数里）。
 *
 * 为什么**不进** [AdminRealtime] 的共享连接组：控制台是单设备单会话的独占租约
 * （服务端 `pending()` 一把槽，见设计 §5.1/§5.5），共享连接会把"另一个标签页已经占住"
 * 变成静默复用；而且控制台**只手工重连**（自动退避重连 = 悄悄开一条新的 root shell，
 * 服务端还会把它记成一次 `superseded`）。
 *
 * `PLATFORM_WS_SUPPORTED=false`（Android 宿主未接 WS）时直接进 [EM_CONSOLE_UNSUPPORTED]，
 * 不建连、不重试——与 [AdminRealtime]/[ConsoleState] 同一退化口径。
 *
 * ⚠️ 只在 Kuikly 线程使用：WS 回调由 `PlatformWsSocket` 的实现投递到主线程
 * （见 `AdminRealtime.kt` 的契约注释），定时器走 `ctx.setTimeout`，全部同线程。
 */
class EchoMuseConsoleState(private val ctx: Pager) {

    /** 六种连接态之一（[EM_CONSOLE_CONNECTING] 起）。渲染见 §6.4 表。 */
    var phase by ctx.observable(EM_CONSOLE_CONNECTING)

    /** 已合并刷新的可见输出（200 ms 一次，见 [EM_CONSOLE_FLUSH_MS]）。 */
    var display by ctx.observable("")

    /** 常驻提示条：`ready.notice`（行模式说明）、截断提示、封顶提示。 */
    var notice by ctx.observable("")

    /** 一次性操作提示：输入过长 / 限流 / 服务端 `error` 文案（下一次操作成功后清空）。 */
    var hint by ctx.observable("")

    /** 服务端 `ready.insecure`：设备未设 token（同网段可直连）→ 琥珀警告条。 */
    var insecure by ctx.observable(false)

    /** `ready.mode`（一期恒为 `pipe`；留着字段不做布尔，见 §8.1）。 */
    var mode by ctx.observable(EM_CONSOLE_MODE_PIPE)

    /** 输入框草稿（↑↓ 历史回填也写这里）。 */
    var input by ctx.observable("")

    /** 自动滚底开关（输出区内容高度变化时是否滚到底）。 */
    var autoScroll by ctx.observable(true)

    /**
     * 首次进入本设备控制台的危险说明是否已确认（PRD §5.6：同一会话只出一次、非模态）。
     *
     * 放状态层而不是视图局部变量：Kuikly 的视图函数在每次重渲染时重新执行，局部标志会被
     * 重置成"每次都弹"；换设备时 [resetForDevice] 再置回 false（新设备的说明该重看一遍）。
     */
    var introSeen by ctx.observable(false)

    /** 「关于行模式」折叠说明是否展开（PRD §7.1 要求把能力差异给用户看，但默认不占屏）。 */
    var aboutOpen by ctx.observable(false)

    /** 最近一次 `closed.reason` 原文（排查用；对外文案见 [phaseText]）。 */
    var closing by ctx.observable("")

    /** `closed` 帧的统计行（`inputs`/`bytes_out`/`secs`），只在排查时看。 */
    var closingStats by ctx.observable("")

    /** 清屏时一并复位的历史游标（-1 = 未在浏览历史）。 */
    private var historyCursor = -1

    /** 本会话输入历史（最多 [EM_CONSOLE_HISTORY_MAX] 条，仅内存）。 */
    private val history = mutableListOf<String>()

    private val buffer = ConsoleBuffer()
    private val carry = AnsiCarry()
    private var socket: PlatformWsSocket? = null
    private var flushRef: String? = null
    private var heartbeatRef: String? = null

    /** 代际计数器：断连/重开后残留的定时器与回调按代际作废（同 [AdminRealtime] 手法）。 */
    private var generation = 0
    private var deviceId = ""
    private var baseUrl = ""
    private var token = ""
    private var lastOnline = false

    /** 是否已由某个 tab 打开过（区分"从没进过"与"进过又离开"）。 */
    private var opened = false

    /** 登录态：只有 `ready` 才允许输入（其余态输入框置灰，见 §6.4 表）。 */
    val canInput: Boolean get() = phase == EM_CONSOLE_READY

    /**
     * 输入是否已超过 `LINE_MAX`（**按 UTF-8 字节**算，中文 3 字节/字）。
     *
     * 前置拦截的意义：服务端也会拒（`error{input-too-long}`），但那时用户已经按了回车，
     * 得到的反馈是"发出去被拒";这里在输入框下方就先提示并置灰「执行」，不浪费一次往返。
     */
    val inputTooLong: Boolean get() = input.encodeToByteArray().size > EM_CONSOLE_LINE_MAX

    /** 「执行」可用性：已连接 + 非空行 + 未超长（空行服务端会当一条空命令，直接不发）。 */
    val canSubmit: Boolean get() = canInput && input.isNotBlank() && !inputTooLong

    /** 输出区为空时的占位文案（按 phase 说清"为什么还没东西"）。 */
    val emptyText: String
        get() = when (phase) {
            EM_CONSOLE_UNSUPPORTED -> ConsoleCopy.UNSUPPORTED
            EM_CONSOLE_OFFLINE -> "（没有历史输出：设备离线时不会建连）"
            EM_CONSOLE_BUSY -> "（没有历史输出：设备被占用，会话未建立）"
            EM_CONSOLE_CONNECTING -> "（正在与设备建立会话…）"
            else -> "（等待设备输出：行模式不回显、无提示符，命令跑起来这里才有字）"
        }

    /** 确认首次进入的危险说明（同一设备同一会话内不再出现）。 */
    fun dismissIntro() {
        introSeen = true
    }

    /** 输出区是否处于"历史输出"态（`offline`/`busy`/`closed` 保留上次缓冲）。 */
    val keepsOutput: Boolean get() = phase != EM_CONSOLE_UNSUPPORTED

    /** 状态行文案（§8.3 唯一出处，渲染层直接贴，不许各处各写）。 */
    val phaseText: String
        get() = when (phase) {
            EM_CONSOLE_CONNECTING -> ConsoleCopy.CONNECTING
            EM_CONSOLE_READY -> ConsoleCopy.CONNECTED
            EM_CONSOLE_OFFLINE -> ConsoleCopy.OFFLINE
            EM_CONSOLE_BUSY -> ConsoleCopy.BUSY
            EM_CONSOLE_UNSUPPORTED -> ConsoleCopy.UNSUPPORTED
            else -> hint.ifEmpty { ConsoleCopy.RECONNECT_LOST }
        }

    /** 是否显示"重新连接/重试"按钮（`connecting`/`ready`/`unsupported` 都不显示）。 */
    val canReconnect: Boolean
        get() = phase == EM_CONSOLE_OFFLINE || phase == EM_CONSOLE_BUSY || phase == EM_CONSOLE_CLOSED

    /**
     * 进入控制台 tab：按设备与连接参数开一次会话。
     *
     * - 同一设备重复调用（重渲染、切回同一 tab）**幂等**：已在建连/已连接就直接返回，
     *   不会因为一次重渲染多开一条 root shell；
     * - `online == false` → [EM_CONSOLE_OFFLINE]，**不发 WS**（§6.4 表，官方 2509 同义）；
     * - 换设备才清空输出/历史；同设备的"重新连接"**保留**上次缓冲（`输出已保留`）。
     */
    fun open(deviceId: String, online: Boolean, baseUrl: String, token: String) {
        val sameDevice = opened && this.deviceId == deviceId &&
            this.baseUrl == baseUrl && this.token == token
        if (sameDevice && (phase == EM_CONSOLE_CONNECTING || phase == EM_CONSOLE_READY)) return

        this.deviceId = deviceId
        this.baseUrl = baseUrl
        this.token = token
        this.lastOnline = online
        this.opened = true
        if (!sameDevice) resetForDevice()

        // 换设备/重连都要断旧连：旧 socket 的残留回调靠代际作废
        stopTimers()
        generation++
        socket?.close()
        socket = null
        closing = ""
        closingStats = ""
        hint = ""

        if (!online) {
            phase = EM_CONSOLE_OFFLINE
            return
        }
        if (!PLATFORM_WS_SUPPORTED) {
            phase = EM_CONSOLE_UNSUPPORTED
            return
        }
        phase = EM_CONSOLE_CONNECTING
        connect()
    }

    /** 离开控制台 tab（页面切走/销毁）：断连并停一切定时器，输出留在内存里给下次看。 */
    fun close() {
        stopTimers()
        generation++
        socket?.close()
        socket = null
        if (phase == EM_CONSOLE_READY || phase == EM_CONSOLE_CONNECTING) phase = EM_CONSOLE_CLOSED
    }

    /** 手工重连（`重新连接` / `重试` 按钮）：不自动退避，只按上次参数重开。 */
    fun reconnect() {
        if (deviceId.isEmpty()) return
        open(deviceId, lastOnline, baseUrl, token)
    }

    // ============================================================
    // 输入
    // ============================================================

    /**
     * 提交输入框内容。
     *
     * **超长不发**（§8.2 `LINE_MAX`：4096 字节，按 UTF-8 字节算——中文 3 字节/字，
     * 用字符数判会放过 3 倍的超长命令）。空行不发（服务端会把它当一条空命令）。
     */
    fun submit() {
        if (phase != EM_CONSOLE_READY) return
        val line = input.trimEnd()
        if (line.isEmpty()) return
        if (line.encodeToByteArray().size > EM_CONSOLE_LINE_MAX) {
            hint = ConsoleCopy.INPUT_TOO_LONG
            return
        }
        val frame = JSONObject().apply {
            put("type", ConsoleWire.INPUT)
            put("data", line)
        }
        if (!send(frame.toString())) return
        remember(line)
        input = ""
        hint = ""
    }

    /** 发 `ctrl-c`（中断设备端当前前台命令；行模式无 tty，信号只对这一条命令有意义）。 */
    fun interrupt() {
        if (phase != EM_CONSOLE_READY) return
        send(ConsoleWire.CTRL_C_FRAME)
    }

    /** 清屏：只清本地缓冲与提示，**不断会话**（设备端的 `sh` 与 working dir 保持）。 */
    fun clearBuffer() {
        cancel(flushRef)
        flushRef = null
        buffer.clear()
        display = ""
        notice = ""
        hint = ""
        historyCursor = -1
    }

    /**
     * 全部可见输出（`复制` 按钮取它交给调用方写剪贴板，本类不碰平台 API）。
     *
     * 复制前先把待合并的缓冲并进去：否则最近 200 ms 的输出会"屏幕上看得见、复制下来却没有"。
     */
    fun copyAll(): String {
        flushNow()
        return display
    }

    /** ↑ 上一条历史；到顶停住（不再回绕，避免误删重发）。 */
    fun historyPrev() {
        if (history.isEmpty()) return
        historyCursor = if (historyCursor < 0) history.size - 1 else (historyCursor - 1).coerceAtLeast(0)
        input = history[historyCursor]
    }

    /** ↓ 下一条历史；越过最新一条则回到空输入。 */
    fun historyNext() {
        if (historyCursor < 0) return
        historyCursor++
        if (historyCursor >= history.size) {
            historyCursor = -1
            input = ""
        } else {
            input = history[historyCursor]
        }
    }

    // ============================================================
    // 连接与帧处理
    // ============================================================

    private fun connect() {
        val gen = generation
        val path = "/api/echomuse/devices/${echoMuseUrlComponent(deviceId)}/console"
        socket = PlatformWsSocket(
            adminRealtimeUrl(baseUrl, token, path),
            onOpen = {
                // WS 建连成功 ≠ 会话可用：服务端要先回拨设备 shell、占住槽，才发 `ready`。
                // 所以这里不改 phase，只等 `ready`（`busy`/`error` 也可能先到）。
                if (gen == generation && phase != EM_CONSOLE_READY) phase = EM_CONSOLE_CONNECTING
            },
            onText = { raw -> if (gen == generation) onFrame(raw) },
            onClosed = {
                if (gen == generation) {
                    socket = null
                    stopTimers()
                    // 没收到 `closed` 帧就断了 = 服务端退出/网络中断；已收 `closed` 的
                    // 情形 phase 已是 CLOSED，不覆盖服务端给的 reason 文案。
                    if (phase == EM_CONSOLE_READY || phase == EM_CONSOLE_CONNECTING) {
                        phase = EM_CONSOLE_CLOSED
                        closing = ""
                        hint = ConsoleCopy.RECONNECT_LOST
                    }
                }
            },
        )
    }

    /** 严格按 §8.1 的 `type` 分发；未知类型静默忽略（服务端保留前向兼容的余地）。 */
    private fun onFrame(raw: String) {
        val data = try {
            JSONObject(raw)
        } catch (_: Throwable) {
            null // 非法帧：连接是长驻的，不能让一帧坏数据把会话打掉
        } ?: return
        when (data.optString("type", "")) {
            ConsoleWire.READY -> onReady(data)
            ConsoleWire.OUTPUT -> {
                val trimmed = buffer.appendChunk(data.text("text"), carry)
                if (trimmed) notice = EchoMuseConsoleText.TRUNCATED_NOTICE
                scheduleFlush()
            }
            ConsoleWire.NOTICE -> {
                // 服务端发的是定值文案（`output-capped`）；会话继续，输入仍可用。
                notice = data.text("text").ifEmpty { ConsoleCopy.CAPPED }
                scheduleFlush()
            }
            ConsoleWire.ERROR -> onError(data.text("code"), data.text("error"))
            ConsoleWire.CLOSED -> onClosedFrame(data)
        }
    }

    private fun onReady(data: JSONObject) {
        phase = EM_CONSOLE_READY
        insecure = data.optBoolean("insecure", false)
        mode = data.optString("mode", EM_CONSOLE_MODE_PIPE)
        notice = data.text("notice")
        hint = ""
        closing = ""
        closingStats = ""
        armHeartbeat()
    }

    /**
     * `error` 帧：`busy` 是**准入失败**（槽被别的操作占着，会话没建立），要转 `busy` 态；
     * `input-too-long`/`rate-limited` 是**会话内可恢复**（只影响这一条，连接还在）；
     * 其余是**收尾**（设备离线 / 服务端内部错误）：会话已不可用，转 `closed`。
     */
    private fun onError(code: String, message: String) {
        when (code) {
            ConsoleWire.CODE_BUSY -> {
                phase = EM_CONSOLE_BUSY
                hint = message.ifEmpty { ConsoleCopy.BUSY }
            }
            ConsoleWire.CODE_INPUT_TOO_LONG -> hint = message.ifEmpty { ConsoleCopy.INPUT_TOO_LONG }
            ConsoleWire.CODE_RATE_LIMITED -> hint = message.ifEmpty { ConsoleCopy.RATE_LIMITED }
            // 设备掉线：正常路径先收 `closed{device-offline}`，这支是竞态下的兜底
            ConsoleWire.CODE_DEVICE_OFFLINE -> {
                phase = EM_CONSOLE_CLOSED
                closing = code
                hint = message.ifEmpty { ConsoleCopy.DEVICE_LOST }
            }
            // `internal`（回拨超时/占槽撤销/帧编码失败）与将来新增的收尾码：`message` 是
            // 服务端 `ErrorCode::message` 拼好的 `COPY_INTERNAL + detail`，**已带前缀**，
            // ⛔ 这里不许再拼 ConsoleCopy.INTERNAL_PREFIX（否则双前缀）；只有 `message`
            // 为空时才退成通用文案。
            else -> {
                phase = EM_CONSOLE_CLOSED
                closing = code
                hint = message.ifEmpty { ConsoleCopy.RECONNECT_LOST }
            }
        }
    }

    /**
     * `closed{reason, detail}`：按 reason 表给文案，输出**保留**（用户还得看断开前的输出）。
     *
     * 文案分派在 [ConsoleHint.closed]（纯函数，`adminCoreTests` 直接断言）；这里只负责
     * 取 `reason`/`detail` 两个字段、写状态、按统计字段收尾。
     */
    private fun onClosedFrame(data: JSONObject) {
        val reason = data.text("reason")
        // `detail` 是**裸原因文本**（无前缀），`internal` 的唯一信息就在里面
        val detail = data.text("detail")
        closing = reason
        phase = EM_CONSOLE_CLOSED
        hint = ConsoleHint.closed(reason, detail)
        val inputs = (data.opt("inputs") as? Number)?.toInt() ?: 0
        val bytes = (data.opt("bytes_out") as? Number)?.toLong() ?: 0L
        val secs = (data.opt("secs") as? Number)?.toLong() ?: 0L
        closingStats = "输入 $inputs 行 · 输出 $bytes 字节 · 时长 ${humanDuration(secs * 1000L)}"
        stopTimers()
        // 服务端随后会关连接；这里主动关一次，避免个别平台把半开连接留到页面销毁
        socket?.close()
        socket = null
        scheduleFlush()
    }

    // ============================================================
    // 定时器：合并刷新与心跳
    // ============================================================

    /** 200 ms 合并刷新：高频输出下每帧重排一次文本布局太贵（§6.4 表）。 */
    private fun scheduleFlush() {
        if (flushRef != null) return
        flushRef = ctx.setTimeout(EM_CONSOLE_FLUSH_MS) {
            flushRef = null
            display = buffer.text
        }
    }

    /** 立刻把缓冲并进 [display]（复制前调用，保证"看到的就是复制到的"）。 */
    private fun flushNow() {
        cancel(flushRef)
        flushRef = null
        display = buffer.text
    }

    /** 应用层心跳：浏览器 WS 无协议层 ping，服务端每 25 s 收不到东西也不会主动断，只用于探活。 */
    private fun armHeartbeat() {
        cancel(heartbeatRef)
        val gen = generation
        heartbeatRef = ctx.setTimeout(EM_CONSOLE_PING_MS) {
            heartbeatRef = null
            if (gen != generation) return@setTimeout
            send(ConsoleWire.PING_FRAME)
            armHeartbeat()
        }
    }

    private fun stopTimers() {
        cancel(flushRef)
        flushRef = null
        cancel(heartbeatRef)
        heartbeatRef = null
    }

    private fun cancel(ref: String?) {
        if (ref != null) ctx.clearTimeout(ref)
    }

    // ============================================================
    // 内部工具
    // ============================================================

    /** 发一帧文本；没连接时返回 false（调用方据此不写历史、不清草稿）。 */
    private fun send(text: String): Boolean {
        val active = socket ?: return false
        active.send(text)
        return true
    }

    /** 记一条历史（连续重复命令只留一条，最多 [EM_CONSOLE_HISTORY_MAX] 条）。 */
    private fun remember(line: String) {
        if (history.lastOrNull() != line) history.add(line)
        while (history.size > EM_CONSOLE_HISTORY_MAX) history.removeAt(0)
        historyCursor = -1
    }

    /** 换设备：输出/历史/状态全清（旧设备的输出绝不能出现在新设备的控制台里）。 */
    private fun resetForDevice() {
        buffer.clear()
        carry.reset()
        history.clear()
        historyCursor = -1
        display = ""
        notice = ""
        hint = ""
        insecure = false
        mode = EM_CONSOLE_MODE_PIPE
        autoScroll = true
        input = ""
        // 换设备 = 换一台 root 文件系统：危险说明重新出一遍
        introSeen = false
        aboutOpen = false
    }
}

// ============================================================
// 协议字面量与用户文案
// ============================================================

// `ConsoleWire`（§8.1 帧字面量）与 `ConsoleCopy`（§8.3 用户文案）已移到
// `EchoMuseConsoleProtocol.kt`：它们是**跨端契约**，与状态机无关，且视图层也要用
// （`insecure` 警告、审计提示、危险横幅）。留在本文件会把状态机顶过 600 行上限。
