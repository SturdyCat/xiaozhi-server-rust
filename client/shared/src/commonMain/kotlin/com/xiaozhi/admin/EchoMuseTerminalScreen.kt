package com.xiaozhi.admin

/** Pure VT screen model. No IO, timers or Kuikly dependency. Dimensions are bounded to 400 x 200.
 * Colors: -1 default, 0..255 indexed palette, 0x01000000 | RGB for true color.
 * Render true color as `value or 0xff000000.toInt()`; the marker keeps white distinct from -1.
 * This is a cell terminal, not an ANSI-stripped append-only transcript. Resize clips/pads, not reflow.
 * snapshot() contains visible rows; scrollbackSnapshot() contains history only, oldest first.
 */
class EchoMuseTerminalScreen(cols: Int = 80, rows: Int = 24) {
    var cols: Int = cols.coerceIn(1, 400); private set
    var rows: Int = rows.coerceIn(1, 200); private set
    var cursorRow: Int = 0; private set
    var cursorCol: Int = 0; private set
    var cursorVisible: Boolean = true; private set
    var applicationCursorKeys: Boolean = false; private set
    var applicationKeypad: Boolean = false; internal set
    var bracketedPaste: Boolean = false; private set
    var alternateScreen: Boolean = false; private set
    val scrollbackLineCount: Int get() = history.size
    val scrollbackCharCount: Int get() = historyChars
    private data class Cell(var text: String = " ", var style: TerminalStyle = TerminalStyle(), var width: Int = 1)
    private data class Saved(val row: Int, val col: Int, val style: TerminalStyle, val wrap: Boolean)
    private data class Main(val lines: MutableList<MutableList<Cell>>, val saved: Saved, val top: Int, val bottom: Int)
    private var style = TerminalStyle()
    private var lines = MutableList(this.rows) { blankLine() }
    private var history = ArrayDeque<TerminalLine>()
    private var historyChars = 0
    private var top = 0
    private var bottom = this.rows - 1
    private var wrap = false
    private var autoWrap = true
    private var origin = false
    private var insert = false
    private var saved = Saved(0, 0, style, false)
    private var main: Main? = null
    private val parser = EchoMuseTerminalAnsi(this)

    private val responses = mutableListOf<String>()
    fun takeResponses(): List<String> = responses.toList().also { responses.clear() }
    fun write(text: String) = parser.write(text)
    fun snapshot(): List<TerminalLine> = lines.map { render(it) }
    fun scrollbackSnapshot(): List<TerminalLine> = history.toList()

    /** Reset terminal modes, both screen buffers, parser and history. */
    fun clear() {
        parser.reset(); responses.clear(); style = TerminalStyle(); lines = MutableList(rows) { blankLine() }
        history.clear(); historyChars = 0; cursorRow = 0; cursorCol = 0; top = 0; bottom = rows - 1
        wrap = false; autoWrap = true; origin = false; insert = false; cursorVisible = true
        applicationCursorKeys = false; applicationKeypad = false; bracketedPaste = false
        alternateScreen = false; main = null; saved = Saved(0, 0, style, false)
    }

    fun resize(cols: Int, rows: Int) {
        val newCols = cols.coerceIn(1, 400); val newRows = rows.coerceIn(1, 200)
        fun fit(buffer: MutableList<MutableList<Cell>>) {
            buffer.forEach { line ->
                while (line.size > newCols) line.removeAt(line.lastIndex)
                while (line.size < newCols) line.add(Cell())
                repair(line)
            }
            while (buffer.size > newRows) buffer.removeAt(buffer.lastIndex)
            while (buffer.size < newRows) buffer.add(MutableList(newCols) { Cell() })
        }
        fit(lines); main?.let { fit(it.lines) }
        this.cols = newCols; this.rows = newRows
        cursorRow = cursorRow.coerceIn(0, newRows - 1); cursorCol = cursorCol.coerceIn(0, newCols - 1)
        top = 0; bottom = newRows - 1; wrap = false
        trimHistory()
    }

    internal fun control(c: Char) {
        when (c) {
            '\r' -> carriageReturn()
            '\n', '\u000b', '\u000c' -> index()
            '\b' -> { cursorCol = (cursorCol - 1).coerceAtLeast(0); wrap = false }
            '\t' -> { cursorCol = ((cursorCol / 8 + 1) * 8).coerceAtMost(cols - 1); wrap = false }
        }
    }
    internal fun carriageReturn() { cursorCol = 0; wrap = false }
    internal fun index() {
        wrap = false
        if (cursorRow == bottom) scrollUp(1) else cursorRow = (cursorRow + 1).coerceAtMost(rows - 1)
    }
    internal fun reverseIndex() {
        wrap = false
        if (cursorRow == top) scrollDown(1) else cursorRow = (cursorRow - 1).coerceAtLeast(0)
    }
    internal fun saveCursor() { saved = Saved(cursorRow, cursorCol, style, wrap) }
    internal fun restoreCursor() {
        cursorRow = saved.row.coerceIn(0, rows - 1); cursorCol = saved.col.coerceIn(0, cols - 1)
        style = saved.style; wrap = saved.wrap
    }

    internal fun printCodePoint(cp: Int) {
        val width = terminalCellWidth(cp).coerceAtMost(cols)
        if (width == 0) {
            val col = if (wrap) cursorCol else cursorCol - 1
            if (col >= 0) {
                val line = lines[cursorRow]
                val lead = if (line[col].width == 0) col - 1 else col
                if (lead >= 0) {
                    val mark = terminalCodePointText(cp)
                    if (line[lead].text.length + mark.length <= 2) line[lead].text += mark
                }
            }
            return
        }
        if (wrap || (width == 2 && cursorCol == cols - 1)) {
            if (autoWrap) { carriageReturn(); index() }
            wrap = false
        }
        val line = lines[cursorRow]
        if (insert) insertChars(width)
        clearCell(line, cursorCol)
        if (width == 2 && cursorCol + 1 < cols) clearCell(line, cursorCol + 1)
        line[cursorCol] = Cell(terminalCodePointText(cp), style, width)
        if (width == 2 && cursorCol + 1 < cols) line[cursorCol + 1] = Cell("", style, 0)
        val next = cursorCol + width
        if (next >= cols) { cursorCol = cols - 1; wrap = autoWrap }
        else cursorCol = next
    }

    internal fun csi(raw: String, final: Char) {
        val private = raw.startsWith('?')
        val body = raw.trimStart('?', '>', '!')
        val p = body.split(';').map { it.toIntOrNull()?.coerceIn(0, 1_000_000) ?: 0 }
        fun n(i: Int = 0, default: Int = 1) = p.getOrNull(i)?.takeIf { it != 0 } ?: default
        val count = n().coerceAtMost(maxOf(cols, rows))
        val minRow = if (origin) top else 0; val maxRow = if (origin) bottom else rows - 1
        fun move(r: Int = cursorRow, c: Int = cursorCol) {
            cursorRow = r.coerceIn(minRow, maxRow); cursorCol = c.coerceIn(0, cols - 1); wrap = false
        }
        when (final) {
            'n' -> if (responses.size < 32) when (p[0]) {
                5 -> responses.add("\u001b[0n")
                6 -> responses.add("\u001b[${if (private) "?" else ""}${cursorRow + 1};${cursorCol + 1}R")
            }
            'c' -> if (responses.size < 32) responses.add(if (raw.startsWith('>')) "\u001b[>0;1;0c" else "\u001b[?1;2c")
            't' -> if (p[0] == 18 && responses.size < 32) responses.add("\u001b[8;${rows};${cols}t")
            'A' -> move(r = cursorRow - count)
            'B', 'e' -> move(r = cursorRow + count)
            'C', 'a' -> move(c = cursorCol + count)
            'D' -> move(c = cursorCol - count)
            'E' -> move(r = cursorRow + count, c = 0)
            'F' -> move(r = cursorRow - count, c = 0)
            'G', '`' -> move(c = n() - 1)
            'd' -> move(r = n() - 1 + minRow)
            'H', 'f' -> move(r = n() - 1 + minRow, c = n(1) - 1)
            'J' -> eraseDisplay(p[0])
            'K' -> eraseLine(p[0])
            'X' -> eraseCells(cursorRow, cursorCol, (cursorCol + count).coerceAtMost(cols))
            '@' -> insertChars(count)
            'P' -> deleteChars(count)
            'L' -> if (cursorRow in top..bottom) repeat(count.coerceAtMost(bottom - cursorRow + 1)) {
                lines.removeAt(bottom); lines.add(cursorRow, blankLine())
            }
            'M' -> if (cursorRow in top..bottom) repeat(count.coerceAtMost(bottom - cursorRow + 1)) {
                lines.removeAt(cursorRow); lines.add(bottom, blankLine())
            }
            'S' -> scrollUp(count)
            'T' -> scrollDown(count)
            'r' -> if (!private) {
                val t = n() - 1; val b = n(1, rows) - 1
                if (t in 0 until rows && b in 0 until rows && t < b) { top = t; bottom = b; move(r = if (origin) top else 0, c = 0) }
            }
            's' -> saveCursor()
            'u' -> restoreCursor()
            'm' -> sgr(p)
            'h', 'l' -> {
                val enabled = final == 'h'
                p.forEach { mode ->
                    if (private) when (mode) {
                        1 -> applicationCursorKeys = enabled
                        6 -> { origin = enabled; move(r = if (enabled) top else 0, c = 0) }
                        7 -> { autoWrap = enabled; wrap = false }
                        25 -> cursorVisible = enabled
                        47, 1047, 1049 -> alternate(enabled)
                        1048 -> if (enabled) saveCursor() else restoreCursor()
                        2004 -> bracketedPaste = enabled
                    } else if (mode == 4) insert = enabled
                }
            }
        }
        if (final in "JKX@PLMST") wrap = false
    }

    private fun sgr(p: List<Int>) {
        var i = 0
        while (i < p.size) {
            val v = p[i++]
            style = when (v) {
                0 -> TerminalStyle()
                1 -> style.copy(bold = true)
                3 -> style.copy(italic = true)
                4 -> style.copy(underline = true)
                7 -> style.copy(inverse = true)
                22 -> style.copy(bold = false)
                23 -> style.copy(italic = false)
                24 -> style.copy(underline = false)
                27 -> style.copy(inverse = false)
                39 -> style.copy(foreground = -1)
                49 -> style.copy(background = -1)
                in 30..37 -> style.copy(foreground = v - 30)
                in 40..47 -> style.copy(background = v - 40)
                in 90..97 -> style.copy(foreground = v - 90 + 8)
                in 100..107 -> style.copy(background = v - 100 + 8)
                38, 48 -> {
                    var color: Int? = null
                    if (p.getOrNull(i) == 5 && p.getOrNull(i + 1) != null) {
                        color = p[i + 1].coerceIn(0, 255); i += 2
                    } else if (p.getOrNull(i) == 2 && p.getOrNull(i + 3) != null) {
                        color = 0x01000000 or (p[i + 1].coerceIn(0, 255) shl 16) or
                            (p[i + 2].coerceIn(0, 255) shl 8) or p[i + 3].coerceIn(0, 255); i += 4
                    }
                    if (color == null) style else if (v == 38) style.copy(foreground = color) else style.copy(background = color)
                }
                else -> style
            }
        }
    }

    private fun alternate(enabled: Boolean) {
        if (enabled == alternateScreen) return
        if (enabled) {
            main = Main(lines, Saved(cursorRow, cursorCol, style, wrap), top, bottom)
            lines = MutableList(rows) { blankLine() }; cursorRow = 0; cursorCol = 0
            top = 0; bottom = rows - 1; wrap = false
        } else {
            main?.let {
                lines = it.lines; saved = it.saved; restoreCursor()
                top = it.top.coerceIn(0, rows - 1); bottom = it.bottom.coerceIn(top, rows - 1)
            }
            main = null
        }
        alternateScreen = enabled
    }

    private fun scrollUp(n: Int) {
        repeat(n.coerceAtMost(bottom - top + 1)) {
            val removed = lines.removeAt(top)
            if (top == 0 && bottom == rows - 1 && !alternateScreen) rememberLine(render(removed))
            lines.add(bottom, blankLine())
        }
    }
    private fun scrollDown(n: Int) {
        repeat(n.coerceAtMost(bottom - top + 1)) { lines.removeAt(bottom); lines.add(top, blankLine()) }
    }
    private fun rememberLine(line: TerminalLine) {
        history.addLast(line); historyChars += line.text.length
        trimHistory()
    }
    private fun trimHistory() {
        // Reserve BOTH buffers (main plus alternate); each cell retains at most two UTF-16 units.
        val allowance = (512 * 1024 - cols * rows * 4).coerceAtLeast(0)
        while (history.size > 5000 || historyChars > allowance) historyChars -= history.removeFirst().text.length
    }
    private fun blankLine() = MutableList(cols) { Cell(style = style) }
    private fun clearCell(line: MutableList<Cell>, col: Int) {
        if (col !in line.indices) return
        if (line[col].width == 0 && col > 0) line[col - 1] = Cell(style = style)
        if (line[col].width == 2 && col + 1 < line.size) line[col + 1] = Cell(style = style)
        line[col] = Cell(style = style)
    }
    private fun repair(line: MutableList<Cell>) {
        for (i in line.indices) {
            if (line[i].width == 2 && (i == line.lastIndex || line[i + 1].width != 0)) line[i] = Cell()
            if (line[i].width == 0 && (i == 0 || line[i - 1].width != 2)) line[i] = Cell()
        }
    }
    private fun eraseCells(row: Int, start: Int, end: Int) {
        for (c in start.coerceAtLeast(0) until end.coerceAtMost(cols)) clearCell(lines[row], c)
    }
    private fun eraseLine(mode: Int) {
        when (mode) {
            0 -> eraseCells(cursorRow, cursorCol, cols)
            1 -> eraseCells(cursorRow, 0, cursorCol + 1)
            2 -> eraseCells(cursorRow, 0, cols)
        }
    }
    private fun eraseDisplay(mode: Int) {
        when (mode) {
            0 -> { eraseLine(0); for (r in cursorRow + 1 until rows) eraseCells(r, 0, cols) }
            1 -> { for (r in 0 until cursorRow) eraseCells(r, 0, cols); eraseLine(1) }
            2 -> for (r in 0 until rows) eraseCells(r, 0, cols)
            3 -> { history.clear(); historyChars = 0 }
        }
    }
    private fun insertChars(n: Int) {
        val line = lines[cursorRow]
        if (line[cursorCol].width == 0) clearCell(line, cursorCol)
        repeat(n.coerceAtMost(cols - cursorCol)) { line.add(cursorCol, Cell(style = style)); line.removeAt(cols) }
        repair(line)
    }
    private fun deleteChars(n: Int) {
        val line = lines[cursorRow]
        repeat(n.coerceAtMost(cols - cursorCol)) { line.removeAt(cursorCol); line.add(Cell(style = style)) }
        repair(line)
    }
    private fun render(line: List<Cell>): TerminalLine {
        val text = StringBuilder(); val spans = mutableListOf<TerminalSpan>()
        var start = 0; var last = line.firstOrNull()?.style ?: TerminalStyle()
        for (cell in line) {
            if (cell.width == 0) continue
            if (cell.style != last) {
                if (text.length > start) spans.add(TerminalSpan(start, text.length, last))
                start = text.length; last = cell.style
            }
            text.append(cell.text)
        }
        if (text.length > start) spans.add(TerminalSpan(start, text.length, last))
        return TerminalLine(text.toString(), spans)
    }
}
