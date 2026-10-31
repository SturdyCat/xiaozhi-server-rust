package com.xiaozhi.admin

/** Immutable render data. Span offsets are UTF-16 offsets into TerminalLine.text, not cell columns.
 * Colors: -1 default, 0..255 palette, 0x01000000 | RGB true color (opaque ARGB = color or -0x1000000).
 */
data class TerminalStyle(
    val foreground: Int = -1,
    val background: Int = -1,
    val bold: Boolean = false,
    val italic: Boolean = false,
    val underline: Boolean = false,
    val inverse: Boolean = false,
)
data class TerminalSpan(val start: Int, val end: Int, val style: TerminalStyle)
data class TerminalLine(val text: String, val spans: List<TerminalSpan>)

/** Bounded streaming decoder; OSC/DCS/APC/PM strings are consumed, never displayed as commands. */
internal class EchoMuseTerminalAnsi(private val screen: EchoMuseTerminalScreen) {
    private var state = 0 // text, ESC, CSI, string, string-ESC, discard-CSI, charset
    private val sequence = StringBuilder()
    private var stringLength = 0
    private var high: Char? = null

    fun reset() { state = 0; sequence.clear(); high = null; stringLength = 0 }

    fun write(text: String) {
        for (c in text) {
            if (state == 3 || state == 4) {
                if (c == '\u0007' || c == '\u009c' || (state == 4 && c == '\\')) {
                    state = 0; stringLength = 0
                } else {
                    // Remain in discard mode after overflow until the terminator. No retained payload.
                    stringLength = (stringLength + 1).coerceAtMost(4096)
                    state = if (c == '\u001b') 4 else 3
                }
                continue
            }
            if (c == '\u001b') { high = null; state = 1; sequence.clear(); continue }
            if (c == '\u0018' || c == '\u001a') { state = 0; sequence.clear(); continue }
            if (state == 6) { state = 0; continue }
            if (state == 1) {
                state = 0
                when (c) {
                    '[' -> { state = 2; sequence.clear() }
                    ']', 'P', '^', '_' -> { state = 3; stringLength = 0 }
                    '(', ')', '*', '+' -> state = 6
                    '7' -> screen.saveCursor()
                    '8' -> screen.restoreCursor()
                    'D' -> screen.index()
                    'E' -> { screen.carriageReturn(); screen.index() }
                    'M' -> screen.reverseIndex()
                    'c' -> screen.clear()
                    '=' -> screen.applicationKeypad = true
                    '>' -> screen.applicationKeypad = false
                }
                continue
            }
            if (state == 2 || state == 5) {
                if (c in '@'..'~') {
                    if (state == 2) screen.csi(sequence.toString(), c)
                    sequence.clear(); state = 0
                } else if (c.code < 32) screen.control(c)
                else if (state == 2) {
                    if (sequence.length < 256) sequence.append(c) else { sequence.clear(); state = 5 }
                }
                continue
            }
            when {
                c == '\u009b' -> { state = 2; sequence.clear() }
                c == '\u009d' || c == '\u0090' -> { state = 3; stringLength = 0 }
                c.code < 32 || c == '\u007f' -> { high = null; screen.control(c) }
                c.isHighSurrogate() -> { if (high != null) screen.printCodePoint(0xfffd); high = c }
                c.isLowSurrogate() -> {
                    val h = high; high = null
                    screen.printCodePoint(if (h == null) 0xfffd else 0x10000 + ((h.code - 0xd800) shl 10) + c.code - 0xdc00)
                }
                else -> { if (high != null) { high = null; screen.printCodePoint(0xfffd) }; screen.printCodePoint(c.code) }
            }
        }
    }
}

internal fun terminalCodePointText(cp: Int): String = if (cp <= 0xffff) cp.toChar().toString() else {
    val n = cp - 0x10000
    "${(0xd800 + (n shr 10)).toChar()}${(0xdc00 + (n and 1023)).toChar()}"
}

/** Practical wcwidth subset: combining marks zero-width, CJK/fullwidth and emoji two cells. */
internal fun terminalCellWidth(cp: Int): Int = when {
    cp in 0x0300..0x036f || cp in 0x1ab0..0x1aff || cp in 0x1dc0..0x1dff ||
        cp in 0x20d0..0x20ff || cp in 0xfe00..0xfe0f || cp in 0xfe20..0xfe2f ||
        cp == 0x200d || cp in 0xe0100..0xe01ef -> 0
    cp in 0x1100..0x115f || cp in 0x2329..0x232a || cp in 0x2e80..0xa4cf ||
        cp in 0xac00..0xd7a3 || cp in 0xf900..0xfaff || cp in 0xfe10..0xfe19 ||
        cp in 0xfe30..0xfe6f || cp in 0xff01..0xff60 || cp in 0xffe0..0xffe6 ||
        cp in 0x1f300..0x1faff || cp in 0x20000..0x3fffd -> 2
    else -> 1
}
