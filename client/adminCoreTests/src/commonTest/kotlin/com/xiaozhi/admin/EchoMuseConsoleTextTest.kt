package com.xiaozhi.admin

import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertTrue

/**
 * `EchoMuseConsoleText` 纯文本管线的单测（设计 §7-T03 验收清单）。
 *
 * 覆盖：跨块 ESC 的 ANSI 剥离、OSC 剥离、`\r` 覆盖、`\t` 展开、2000 行裁剪丢最旧、
 * 200k 字符裁剪、单行 4000 截断、非法字节不崩。
 *
 * 这些边界在真机上要么复现不了（需要设备在特定位置切帧），要么一复现就是"屏幕上一坨
 * `[1;32m`"这种没法写断言的现象——所以全部落成纯函数输入输出。
 */
class EchoMuseConsoleTextTest {

    // ============================================================
    // ANSI 剥离
    // ============================================================

    @Test
    fun ansi_csi_is_stripped_across_chunk_boundary() {
        val carry = AnsiCarry()
        // 前半截：ESC [ 3 —— 序列被切在帧边界，此时不能吐出任何可见字符
        assertEquals("", EchoMuseConsoleText.sanitize("\u001b[3", carry))
        // 后半截：1m 结束这条 CSI，后面才是正文
        assertEquals("red", EchoMuseConsoleText.sanitize("1mred\u001b[0m", carry))
        // 状态机回到文本态，后续帧正常透出
        assertEquals("plain", EchoMuseConsoleText.sanitize("plain", carry))
    }

    @Test
    fun ansi_csi_with_multiple_params_is_stripped() {
        val carry = AnsiCarry()
        val raw = "\u001b[1;32mOK\u001b[0m \u001b[2Kdone"
        assertEquals("OK done", EchoMuseConsoleText.sanitize(raw, carry))
    }

    @Test
    fun ansi_osc_is_stripped_with_bel_and_st_terminators() {
        val carry = AnsiCarry()
        // OSC 以 BEL 结束（标题设置，shell 启动脚本里常见）
        assertEquals("hi", EchoMuseConsoleText.sanitize("\u001b]0;标题\u0007hi", carry))
        // OSC 8 超链接以 ESC \ 结束，剥完只剩可见文本
        assertEquals(
            "link",
            EchoMuseConsoleText.sanitize("\u001b]8;;http://x\u001b\\link\u001b]8;;\u001b\\", carry),
        )
    }

    @Test
    fun dangling_escape_at_chunk_end_does_not_crash() {
        val carry = AnsiCarry()
        assertEquals("", EchoMuseConsoleText.sanitize("\u001b", carry))
        // 下一帧来了才判定它其实是 CSI
        assertEquals("", EchoMuseConsoleText.sanitize("[0m", carry))
        assertEquals("tail", EchoMuseConsoleText.sanitize("tail", carry))
    }

    // ============================================================
    // \r 覆盖 与 \t 展开
    // ============================================================

    @Test
    fun carriage_return_overwrites_current_line() {
        val buffer = ConsoleBuffer()
        val carry = AnsiCarry()
        buffer.appendChunk("progress 0%\rprogress 100%", carry)
        assertEquals("progress 100%", buffer.text)
        assertEquals(1, buffer.lineCount)
    }

    @Test
    fun carriage_return_keeps_untouched_tail() {
        // 短串覆盖长串时，后半截残留是终端的真实语义（不能用新串直接换掉旧行）
        val buffer = ConsoleBuffer()
        buffer.appendChunk("abcdef\rxy", carry = AnsiCarry())
        assertEquals("xycdef", buffer.text)
    }

    @Test
    fun tab_expands_to_four_spaces() {
        val carry = AnsiCarry()
        assertEquals("a    b", EchoMuseConsoleText.sanitize("a\tb", carry))
        // 连续制表符各展开 4 格，不做制表位对齐
        assertEquals("        x", EchoMuseConsoleText.sanitize("\t\tx", carry))
    }

    /**
     * `\r\n` 混排：`\r` 只归零光标、`\n` 才提交整行，所以 CRLF 必须留下**完整**的那一行
     * （进程日志里 CRLF 很常见；把 `\r\n` 当"空行"会让整段日志变成一片空白）。
     */
    @Test
    fun crlf_commits_the_whole_line_instead_of_blank() {
        val buffer = ConsoleBuffer()
        buffer.appendChunk("boot ok\r\nready\r\n", carry = AnsiCarry())
        assertEquals("boot ok\nready\n", buffer.text)
        assertEquals(2, buffer.lineCount)
    }

    /** 行尾悬挂的 `\r` 只把光标拉回行首，不换行、不丢内容。 */
    @Test
    fun trailing_carriage_return_moves_cursor_without_dropping_content() {
        val buffer = ConsoleBuffer()
        buffer.appendChunk("abc\r", carry = AnsiCarry())
        assertEquals("abc", buffer.text)
        assertEquals(1, buffer.lineCount)
        // 再 `\r` + 覆盖：从行首改两个字符，尾部残留是终端真实语义
        buffer.appendChunk("\rde", carry = AnsiCarry())
        assertEquals("dec", buffer.text)
    }

    // ============================================================
    // 双上限与单行截断
    // ============================================================

    @Test
    fun line_cap_drops_oldest() {
        val buffer = ConsoleBuffer()
        val carry = AnsiCarry()
        val total = EchoMuseConsoleText.MAX_LINES + 500
        for (i in 0 until total) buffer.appendChunk("L$i\n", carry)

        assertEquals(EchoMuseConsoleText.MAX_LINES, buffer.lineCount)
        val text = buffer.text
        // 最旧的 500 行被丢掉：首行是 L500，末行是 L2499
        assertTrue(text.startsWith("L500\n"), "首行应为 L500，实际：${text.take(20)}")
        assertTrue(text.contains("L${total - 1}\n"), "末行应保留")
        assertFalse(text.contains("L499\n"), "被丢弃的行不应残留")
        assertTrue(buffer.trimmed, "丢过行必须置 trimmed，供一次性提示")
    }

    @Test
    fun char_cap_drops_oldest_lines() {
        val buffer = ConsoleBuffer()
        val carry = AnsiCarry()
        // 每行刚好 4000 字（不触发单行截断），60 行 = 240 000 字 > 200 000
        val line = "x".repeat(EchoMuseConsoleText.LINE_CHAR_MAX)
        repeat(60) { buffer.appendChunk("$line\n", carry) }

        assertEquals(50, buffer.lineCount, "24 万字应被裁到 20 万字（整行丢）")
        assertTrue(buffer.trimmed)
        assertFalse(buffer.text.contains(EchoMuseConsoleText.TRUNCATED_SUFFIX), "整行上限不该给行加后缀")
    }

    @Test
    fun single_long_line_is_truncated_with_suffix() {
        val buffer = ConsoleBuffer()
        val raw = "y".repeat(EchoMuseConsoleText.LINE_CHAR_MAX + 500)
        buffer.appendChunk(raw, carry = AnsiCarry())

        val text = buffer.text
        assertEquals(
            "y".repeat(EchoMuseConsoleText.LINE_CHAR_MAX) + EchoMuseConsoleText.TRUNCATED_SUFFIX,
            text,
        )
        assertEquals(1, buffer.lineCount)
    }

    @Test
    fun append_chunk_reports_first_trim_only() {
        val buffer = ConsoleBuffer()
        val carry = AnsiCarry()
        for (i in 0 until EchoMuseConsoleText.MAX_LINES) buffer.appendChunk("L$i\n", carry)
        assertFalse(buffer.trimmed, "刚好到上限时还没有丢内容")
        // 第 2001 行才真正丢一行 → 首次返回 true
        assertTrue(buffer.appendChunk("overflow\n", carry))
        // 之后不再重复提示
        assertFalse(buffer.appendChunk("more\n", carry))
    }

    // ============================================================
    // 非法字节
    // ============================================================

    @Test
    fun invalid_bytes_are_replaced_and_do_not_crash() {
        val carry = AnsiCarry()
        // 未配对的高代理项
        assertEquals(EchoMuseConsoleText.REPLACEMENT, EchoMuseConsoleText.sanitize("\uD800", carry))
        // C0 控制字符与 DEL 都不是可见字符
        assertEquals(
            "${EchoMuseConsoleText.REPLACEMENT}${EchoMuseConsoleText.REPLACEMENT}",
            EchoMuseConsoleText.sanitize("\u0000\u007f", carry),
        )
        // 成对代理项必须整对保留（emoji / 生僻字不该变成两个方块）
        assertEquals("\uD83D\uDE00", EchoMuseConsoleText.sanitize("\uD83D\uDE00", carry))
        // NUL 混在正文里只影响自己那一格
        assertEquals(
            "a${EchoMuseConsoleText.REPLACEMENT}b",
            EchoMuseConsoleText.sanitize("a\u0000b", carry),
        )
    }

    @Test
    fun invalid_bytes_survive_the_buffer() {
        val buffer = ConsoleBuffer()
        buffer.appendChunk("ok\u0000\uD800\nnext\n", carry = AnsiCarry())
        assertEquals("ok${EchoMuseConsoleText.REPLACEMENT}${EchoMuseConsoleText.REPLACEMENT}\nnext\n", buffer.text)
        assertEquals(2, buffer.lineCount)
    }

    // ============================================================
    // 清屏与重置
    // ============================================================

    @Test
    fun clear_resets_content_and_trim_flag() {
        val buffer = ConsoleBuffer()
        val carry = AnsiCarry()
        for (i in 0 until EchoMuseConsoleText.MAX_LINES + 1) buffer.appendChunk("L$i\n", carry)
        assertTrue(buffer.trimmed)

        buffer.clear()
        assertTrue(buffer.isEmpty)
        assertEquals("", buffer.text)
        assertEquals(0, buffer.lineCount)
        assertFalse(buffer.trimmed, "用户主动清屏不该继续提示“已截断”")
    }

    @Test
    fun carry_reset_abandons_half_parsed_sequence() {
        val carry = AnsiCarry()
        EchoMuseConsoleText.sanitize("\u001b]0;未结束的标题", carry)
        // 会话重连：半截 OSC 不能带到新连接里，否则下一段正文会被一起吞掉
        carry.reset()
        assertEquals("fresh", EchoMuseConsoleText.sanitize("fresh", carry))
    }
}
