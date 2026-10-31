package com.xiaozhi.admin

import kotlin.test.*

class EchoMuseConsoleKeysTest {
    @Test fun rawEditingAndControls() {
        assertEquals("\r", EchoMuseConsoleKeys.encode("Enter"))
        assertEquals("\u007f", EchoMuseConsoleKeys.encode("Backspace"))
        assertEquals("\t", EchoMuseConsoleKeys.encode("Tab"))
        assertEquals("\u001b", EchoMuseConsoleKeys.encode("Escape"))
        assertEquals("\u0003", EchoMuseConsoleKeys.encode("c", ctrl = true))
        assertEquals("\u0004", EchoMuseConsoleKeys.encode("d", ctrl = true))
        assertEquals("\u0000", EchoMuseConsoleKeys.encode(" ", ctrl = true))
        assertEquals("\u001bb", EchoMuseConsoleKeys.encode("b", alt = true))
    }
    @Test fun cursorFunctionKeysAndModifiers() {
        assertEquals("\u001b[A", EchoMuseConsoleKeys.encode("ArrowUp"))
        assertEquals("\u001bOA", EchoMuseConsoleKeys.encode("ArrowUp", applicationCursor = true))
        assertEquals("\u001b[1;5D", EchoMuseConsoleKeys.encode("ArrowLeft", ctrl = true))
        assertEquals("\u001b[Z", EchoMuseConsoleKeys.encode("Tab", shift = true))
        assertEquals("\u001b[3~", EchoMuseConsoleKeys.encode("Delete"))
        assertEquals("\u001bOP", EchoMuseConsoleKeys.encode("F1"))
        assertEquals("\u001b[15~", EchoMuseConsoleKeys.encode("F5"))
        assertEquals("\u001b[24~", EchoMuseConsoleKeys.encode("F12"))
    }
    @Test fun systemShortcutsAndImeAreNotSentAsSyntheticKeys() {
        assertNull(EchoMuseConsoleKeys.encode("v", meta = true))
        assertNull(EchoMuseConsoleKeys.encode("c", ctrl = true, shift = true))
        assertNull(EchoMuseConsoleKeys.encode("Dead"))
        assertNull(EchoMuseConsoleKeys.encode("a"))
    }
    @Test fun boundedUnicodeChunksReassembleExactly() {
        val text = "中文😀\u001b[A".repeat(1300)
        val chunks = EchoMuseConsoleKeys.chunks(text, 2048)
        assertTrue(chunks.all { it.encodeToByteArray().size <= 2048 })
        assertTrue(chunks.all { !it.first().isLowSurrogate() && !it.last().isHighSurrogate() })
        assertEquals(text, chunks.joinToString(""))
        assertEquals(emptyList(), EchoMuseConsoleKeys.chunks(""))
    }
    @Test fun terminalQueryResponsesAreBoundedAndReset() {
        val screen = EchoMuseTerminalScreen(80, 24)
        screen.write("abc\u001b[6n\u001b[c\u001b[18t")
        assertEquals(listOf("\u001b[1;4R", "\u001b[?1;2c", "\u001b[8;24;80t"), screen.takeResponses())
        assertTrue(screen.takeResponses().isEmpty())
        screen.write("\u001b[6n".repeat(100))
        assertEquals(32, screen.takeResponses().size)
        screen.write("\u001b[6n"); screen.clear()
        assertTrue(screen.takeResponses().isEmpty())
    }
}
