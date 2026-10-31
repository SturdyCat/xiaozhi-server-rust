package com.xiaozhi.admin

import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertTrue

class EchoMuseTerminalScreenTest {
    private fun EchoMuseTerminalScreen.text() = snapshot().map { it.text.trimEnd() }

    @Test fun controlsAndDeferredWrap() {
        val s = EchoMuseTerminalScreen(5, 3)
        s.write("abcde")
        assertEquals(0, s.cursorRow)
        s.write("F\rZ\bY\nQ")
        assertEquals(listOf("abcde", "Y", " Q"), s.text())
        s.clear(); s.write("a\tb")
        assertEquals("a   b", s.snapshot()[0].text)
    }

    @Test fun ansiCanSplitAtEveryBoundary() {
        val s = EchoMuseTerminalScreen(8, 2)
        val data = "abcd\u001b[2D\u001b[31;1mXY\u001b[0m!"
        data.forEach { s.write(it.toString()) }
        assertEquals("abXY!", s.text()[0])
        val red = s.snapshot()[0].spans.first { it.style.foreground == 1 }
        assertEquals(2, red.start); assertEquals(4, red.end); assertTrue(red.style.bold)
        assertEquals(-1, s.snapshot()[0].spans.last().style.foreground)
    }

    @Test fun cursorPositionSaveRestoreAndEdits() {
        val s = EchoMuseTerminalScreen(8, 3)
        s.write("abcdef\u001b[1;3H\u001b[s\u001b[2@XY\u001b[u\u001b[P")
        assertEquals("abYcdef", s.text()[0])
        s.write("\u001b[2;4Hz\u001b7\u001b[H!\u001b8Q")
        assertEquals("   zQ", s.text()[1])
        s.write("\u001b[4hA\u001b[4l")
        assertEquals("   zQA", s.text()[1])
    }

    @Test fun eraseModesPreserveCursorAndBackground() {
        val s = EchoMuseTerminalScreen(6, 3)
        s.write("abcdef\r\n123456\u001b[1;3H\u001b[44m\u001b[K")
        assertEquals("ab", s.text()[0]); assertEquals(2, s.cursorCol)
        assertEquals(4, s.snapshot()[0].spans.last().style.background)
        s.write("\u001b[2J")
        assertEquals(listOf("", "", ""), s.text())
        assertEquals(0, s.cursorRow); assertEquals(2, s.cursorCol)
        s.write("\u001b[Habcdef\u001b[1;3H\u001b[1K")
        assertEquals("   def", s.text()[0])
    }

    @Test fun regionScrollAndLineInsertionDeletion() {
        val s = EchoMuseTerminalScreen(4, 4)
        s.write("1111\r\n2222\r\n3333\r\n4444")
        s.write("\u001b[2;3r\u001b[3;1H\n")
        assertEquals(listOf("1111", "3333", "", "4444"), s.text())
        assertEquals(0, s.scrollbackLineCount)
        s.write("\u001b[2;1H\u001b[LX")
        assertEquals(listOf("1111", "X", "3333", "4444"), s.text())
        s.write("\u001b[2;1H\u001b[M")
        assertEquals(listOf("1111", "3333", "", "4444"), s.text())
        s.write("\u001b[2;1H\u001bM")
        assertEquals(listOf("1111", "", "3333", "4444"), s.text())
    }

    @Test fun alternateScreenRestoresMainAndModes() {
        val s = EchoMuseTerminalScreen(8, 2)
        s.write("main\u001b[?1h\u001b[?2004h\u001b[?25l\u001b[?1049hALT\r\nx\r\ny")
        assertTrue(s.applicationCursorKeys); assertTrue(s.bracketedPaste); assertFalse(s.cursorVisible)
        assertTrue(s.alternateScreen); assertEquals(0, s.scrollbackLineCount)
        s.write("\u001b[?1049l!")
        assertFalse(s.alternateScreen); assertEquals("main!", s.text()[0]); assertEquals(5, s.cursorCol)
        s.clear()
        assertFalse(s.applicationCursorKeys); assertFalse(s.bracketedPaste); assertTrue(s.cursorVisible)
    }

    @Test fun resizeClipsPadsAndKeepsMainBufferUsable() {
        val s = EchoMuseTerminalScreen(8, 3)
        s.write("abcdef\u001b[3;8Hz")
        s.resize(4, 2)
        assertEquals(listOf("abcd", ""), s.text()); assertEquals(1, s.cursorRow); assertEquals(3, s.cursorCol)
        s.write("\u001b[?1049hALT"); s.resize(10, 4); s.write("\u001b[?1049l")
        assertEquals(4, s.snapshot().size); assertEquals(10, s.snapshot()[0].text.length)
        assertEquals("abcd", s.text()[0])
    }

    @Test fun cjkSurrogatesCombiningAndWideCellErasure() {
        val s = EchoMuseTerminalScreen(8, 2)
        s.write("中a\u0301"); assertEquals(3, s.cursorCol)
        s.write("\ud83d"); s.write("\ude00")
        assertEquals(5, s.cursorCol); assertTrue(s.text()[0].startsWith("中a\u0301😀"))
        s.write("\u001b[1;2HX")
        assertEquals(" Xa\u0301😀", s.text()[0])
        s.clear(); s.write("123456中Z")
        assertEquals("123456中", s.text()[0]); assertEquals("Z", s.text()[1])
    }

    @Test fun indexedAndTrueColorSgr() {
        val s = EchoMuseTerminalScreen(10, 2)
        s.write("\u001b[38;5;200;48;2;1;2;3mX\u001b[94;103;3;4;7mY")
        val spans = s.snapshot()[0].spans
        assertEquals(200, spans[0].style.foreground)
        assertEquals(0x01010203, spans[0].style.background)
        assertEquals(12, spans[1].style.foreground); assertEquals(11, spans[1].style.background)
        assertTrue(spans[1].style.italic); assertTrue(spans[1].style.underline); assertTrue(spans[1].style.inverse)
        s.write("\u001b[38;2;255;255;255mZ")
        assertEquals(0x01ffffff, s.snapshot()[0].spans[2].style.foreground)
    }

    @Test fun stringControlsAndOverlongCsiRemainBoundedAcrossChunks() {
        val s = EchoMuseTerminalScreen(20, 2)
        s.write("A\u001b]0;" + "x".repeat(20000)); s.write("\u001b"); s.write("\\B")
        s.write("\u001b[" + "1;".repeat(5000)); s.write("mC")
        s.write("\u001bPignored\u001b\\D")
        assertEquals("ABCD", s.text()[0])
        s.write("\u001b[?7l\u001b[1;20HXY")
        assertEquals(0, s.cursorRow); assertTrue(s.text()[0].endsWith("Y"))
    }

    @Test fun chunkingDoesNotChangeScreenOrModes() {
        val data = "first\rsecond\n中😀\u001b[2;3H\u001b[32mX\u001b[K\u001b[?1049hALT\u001b[?1049l\u001b[?2004h\u001b]title\u0007!"
        val whole = EchoMuseTerminalScreen(12, 4).apply { write(data) }
        for (cut in 0..data.length) {
            val split = EchoMuseTerminalScreen(12, 4)
            split.write(data.take(cut)); split.write(data.drop(cut))
            assertEquals(whole.snapshot(), split.snapshot(), "split at $cut")
            assertEquals(whole.cursorCol, split.cursorCol)
            assertEquals(whole.cursorRow, split.cursorRow)
            assertEquals(whole.bracketedPaste, split.bracketedPaste)
        }
    }

    @Test fun outputAndHistoryBoundsAreEnforced() {
        val s = EchoMuseTerminalScreen(400, 2)
        repeat(7000) { s.write("x".repeat(400) + "\r\n") }
        assertTrue(s.scrollbackLineCount <= 5000)
        assertTrue(s.scrollbackCharCount + s.snapshot().sumOf { it.text.length } <= 512 * 1024)
        s.resize(400, 200)
        assertTrue(s.scrollbackCharCount <= 512 * 1024 - 400 * 200 * 4)
        s.write("\u001b[3J")
        assertEquals(0, s.scrollbackLineCount)
    }
}
