package com.xiaozhi.admin

/**
 * 控制台输出的**纯文本管线**（设计 §6.4「缓冲与刷新」表、§8.2 常量）。
 *
 * 为什么单独成文件且**不 import 任何 Kuikly 类型**：这一层全部是字符串变换，
 * 放进 `adminCoreTests` 的 `kotlin.include` 就能用普通 JVM 单测直驱（跨块 ESC、
 * `\r` 覆盖、字符上限这些边界靠手点是测不出来的）。渲染只读 [ConsoleBuffer.text]。
 *
 * 分工：
 * - 服务端只做 **UTF-8 边界对齐**（`console_frames.rs::Utf8Carry`），不剥离转义序列；
 * - 客户端在这里**一处**做净化，避免"服务端剥了一半、前端又剥一遍"的规则漂移。
 *
 * 净化的四项（PRD §5.4）：
 * 1. ANSI CSI / OSC 序列整体丢弃（含 chunk 边界被切开的情形，见 [AnsiCarry]）；
 * 2. `\t` → 4 个空格（不做制表位对齐：等宽字体未必命中，见 §6.4 字体表）；
 * 3. `\r` → 光标回行首，后续字符**就地覆盖**（进度条语义，不是换行）；
 * 4. 非法字节 → `�`：未配对的代理项、C0 控制字符、DEL。
 */
object EchoMuseConsoleText {
    /** 环形缓冲行数上限（复用 `ConsoleState.kt` 的 `CONSOLE_MAX_LINES` 口径）。 */
    const val MAX_LINES = 2000

    /** 环形缓冲字符上限：行多但短（`ps`）与行少但长（`dmesg`）都要有界。 */
    const val MAX_CHARS = 200_000

    /** 单行字符上限：超出即截断并追加 [TRUNCATED_SUFFIX]。 */
    const val LINE_CHAR_MAX = 4000

    /** 单行截断后缀（PRD §5.4 定稿文案，逐字）。 */
    const val TRUNCATED_SUFFIX = "…（行过长已截断）"

    /** 缓冲开始丢弃最旧内容时的一次性提示（§8.3 `TRUNCATED`）。 */
    const val TRUNCATED_NOTICE = "输出已截断（保留最近 2000 行）"

    /** 非法字节的替换字符。与 Rust `String::from_utf8_lossy` 同形，读起来不刺眼。 */
    const val REPLACEMENT = "\uFFFD"

    /** 制表符展开宽度（PRD §5.4）。 */
    private const val TAB_SPACES = 4

    /**
     * 净化一段（已解码的）文本帧正文：剥转义序列、展开 `\t`、非法字符换 `�`。
     *
     * `\n` / `\r` 原样保留（换行与覆盖由 [ConsoleBuffer] 解释）。
     * [carry] 必须**跨 chunk 复用**：ESC 序列被切成两帧时，前半截状态留在这里，
     * 否则后半截会被当成可见文本打出来（"`[1;32m` 突然出现在屏幕上"就是这么来的）。
     */
    fun sanitize(chunk: String, carry: AnsiCarry): String {
        if (chunk.isEmpty()) return ""
        val out = StringBuilder(chunk.length)
        var i = 0
        while (i < chunk.length) {
            val c = chunk[i]
            when (carry.state) {
                AnsiCarry.TEXT -> when (c) {
                    ESC_CHAR -> carry.state = AnsiCarry.ESC
                    '\t' -> repeat(TAB_SPACES) { out.append(' ') }
                    '\n', '\r' -> out.append(c)
                    else -> i = appendVisible(chunk, i, out)
                }
                AnsiCarry.ESC -> carry.state = when (c) {
                    '[' -> AnsiCarry.CSI
                    ']' -> AnsiCarry.OSC
                    ESC_CHAR -> AnsiCarry.ESC
                    // 两字节转义（`ESC ( B`、`ESC =`、`ESC M`…）：吞掉引导字符后回文本态。
                    // 注意 `ESC (` 后面还有一个字符，一起吞掉更接近真实宽度语义。
                    '(', ')', '*', '+', '#', '%' -> AnsiCarry.ESC_CHARSET
                    else -> AnsiCarry.TEXT
                }
                AnsiCarry.ESC_CHARSET -> carry.state = AnsiCarry.TEXT
                AnsiCarry.CSI -> {
                    // CSI 以 0x40..0x7E 的**终结字节**结束（`m`/`K`/`H`/`J`…）；中间参数不缓存。
                    if (c.code in 0x40..0x7e) carry.state = AnsiCarry.TEXT
                }
                AnsiCarry.OSC -> when (c) {
                    BEL_CHAR -> carry.state = AnsiCarry.TEXT
                    ESC_CHAR -> carry.state = AnsiCarry.OSC_ESC
                }
                AnsiCarry.OSC_ESC -> carry.state = if (c == '\\') AnsiCarry.TEXT else AnsiCarry.OSC
                else -> carry.state = AnsiCarry.TEXT
            }
            i++
        }
        return out.toString()
    }

    /**
     * 追加一个可见字符（含代理对合并与非法字符替换），返回**下一个未消费的下标**。
     *
     * 代理对必须整对输出：把它拆成两个 `�` 会让 emoji/生僻字变成两个方块，
     * 而中文语音设备的日志里这类字符并不少见（用户自定义唤醒词、曲目名）。
     */
    private fun appendVisible(chunk: String, index: Int, out: StringBuilder): Int {
        val c = chunk[index]
        val code = c.code
        return when {
            c.isHighSurrogate() && index + 1 < chunk.length && chunk[index + 1].isLowSurrogate() -> {
                out.append(c).append(chunk[index + 1])
                index + 1
            }
            // 单个代理项（高低都算）无法构成字符：换 `�` 而不是崩在 `append`
            c.isSurrogate() -> {
                out.append(REPLACEMENT)
                index
            }
            code < 0x20 || code == 0x7f -> {
                out.append(REPLACEMENT)
                index
            }
            else -> {
                out.append(c)
                index
            }
        }
    }

    private const val ESC_CHAR = '\u001b'
    private const val BEL_CHAR = '\u0007'
}

/**
 * [EchoMuseConsoleText.sanitize] 的**跨块残留**：ESC 序列被切在两帧中间时的状态。
 *
 * 一次控制台会话持有一个实例，逐帧传进去；`clearBuffer()` 不重置它（清屏不该影响
 * 转义序列的解析状态），但会话重连时重置（新连接的协议流从干净状态开始）。
 */
class AnsiCarry {
    internal var state: Int = TEXT

    /** 丢弃进行中的转义序列，回到"文本"状态（重连/换会话时用）。 */
    fun reset() {
        state = TEXT
    }

    internal companion object {
        const val TEXT = 0
        const val ESC = 1
        const val ESC_CHARSET = 2
        const val CSI = 3
        const val OSC = 4
        const val OSC_ESC = 5
    }
}

/**
 * 终端行缓冲：行内 `\r` 就地覆盖、双上限（[EchoMuseConsoleText.MAX_LINES] /
 * [EchoMuseConsoleText.MAX_CHARS]）丢最旧、单行超长截断。
 *
 * 为什么不用 `List<String>` 逐行 `add`：`\r` 覆盖要求"改当前行"，`List` 得先
 * `remove(last)` 再 `add`，每次刷新都动列表末尾，且行/字符两个上限要各自扫一遍。
 * 这里"已完成行列表 + 当前行 `StringBuilder`"的形态，两个上限都只需从头部丢。
 *
 * 所有方法都在 Kuikly 线程（主线程）调用：`appendChunk` 由 WS 文本回调驱动，
 * `text` 由 200 ms 合并刷新的 `setTimeout` 驱动，二者同线程，无需加锁。
 */
class ConsoleBuffer {
    /** 已完成的行（含换行符之前的内容，不含 `\n`）。 */
    private val done = mutableListOf<String>()

    /** 当前行（正在被追加/覆盖）。 */
    private val current = StringBuilder()

    /** 当前行的光标列：`\r` 归零，覆盖写不增长长度。 */
    private var cursor = 0

    /** 当前行已达上限、后续字符丢弃直到下一个 `\n`。 */
    private var lineTruncated = false

    /** 可见字符总数（含截断后缀），用于字符上限判定。 */
    private var total = 0

    /** 是否已因行/字符上限丢过内容（[EchoMuseConsoleText.TRUNCATED_NOTICE] 只提示一次）。 */
    var trimmed: Boolean = false
        private set

    /** 可见行数（含未换行的当前行）。 */
    val lineCount: Int
        get() = done.size + if (current.isNotEmpty()) 1 else 0

    /** 缓冲是否为空（空态文案用）。 */
    val isEmpty: Boolean
        get() = done.isEmpty() && current.isEmpty()

    /** 清屏：内容与光标归零，[trimmed] 一并复位（用户主动清的不算"被截断"）。 */
    fun clear() {
        done.clear()
        current.setLength(0)
        cursor = 0
        lineTruncated = false
        total = 0
        trimmed = false
    }

    /**
     * 追加一帧原始文本（先净化再入缓冲）。返回本帧是否**首次**触发上限提示——
     * 调用方据此把 [EchoMuseConsoleText.TRUNCATED_NOTICE] 贴到常驻文案上。
     */
    fun appendChunk(raw: String, carry: AnsiCarry): Boolean {
        val before = trimmed
        append(EchoMuseConsoleText.sanitize(raw, carry))
        return trimmed && !before
    }

    /** 追加一段**已净化**文本（`\n` 换行、`\r` 覆盖、其余照写）。 */
    fun append(text: String) {
        for (ch in text) {
            when (ch) {
                '\n' -> commitLine()
                '\r' -> cursor = 0
                else -> put(ch)
            }
        }
        trimToCaps()
    }

    /**
     * 全部可见内容（行间 `\n` 连接）。
     *
     * 结尾不带多余空行：最后一行为空时不再追加分隔符，否则 `Text` 底部会多出一行
     * 空白、`Scroller` 的自动滚底也会差一行。
     */
    val text: String
        get() {
            if (done.isEmpty() && current.isEmpty()) return ""
            val sb = StringBuilder(total.coerceAtLeast(done.size + current.length))
            for (line in done) sb.append(line).append('\n')
            sb.append(visibleCurrent())
            return sb.toString()
        }

    /** 当前行（已截断时带后缀）。 */
    private fun visibleCurrent(): String =
        if (lineTruncated) current.toString() + EchoMuseConsoleText.TRUNCATED_SUFFIX else current.toString()

    /** 写一个字符：光标在行尾则追加，否则就地覆盖。 */
    private fun put(ch: Char) {
        if (lineTruncated) return
        if (current.length >= EchoMuseConsoleText.LINE_CHAR_MAX) {
            // 只在"还要再写一个字符"时判定截断：恰好 4000 字的完整行不该带后缀。
            lineTruncated = true
            return
        }
        if (cursor < current.length) {
            current[cursor] = ch
        } else {
            current.append(ch)
            total++
        }
        cursor++
    }

    /** 提交当前行（`\n`），行数超上限就从头部丢。 */
    private fun commitLine() {
        done.add(visibleCurrent())
        if (lineTruncated) total += EchoMuseConsoleText.TRUNCATED_SUFFIX.length
        current.setLength(0)
        cursor = 0
        lineTruncated = false
        while (done.size > EchoMuseConsoleText.MAX_LINES) {
            total -= done.removeAt(0).length
            trimmed = true
        }
    }

    /** 字符上限：按整行丢最旧的（不切半行——半行日志比少一行更难读）。 */
    private fun trimToCaps() {
        while (total > EchoMuseConsoleText.MAX_CHARS && done.size > 1) {
            total -= done.removeAt(0).length
            trimmed = true
        }
    }
}
