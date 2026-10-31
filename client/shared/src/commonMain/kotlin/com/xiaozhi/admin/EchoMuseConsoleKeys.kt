package com.xiaozhi.admin

/** VT input encoding shared by browser and Catalyst hosts. PTY echo belongs to the device. */
internal object EchoMuseConsoleKeys {
    fun encode(key: String, ctrl: Boolean = false, alt: Boolean = false, shift: Boolean = false,
               meta: Boolean = false, applicationCursor: Boolean = false): String? {
        if (meta || (ctrl && shift && key.lowercase() in setOf("c", "v"))) return null
        val modifier = 1 + (if (shift) 1 else 0) + (if (alt) 2 else 0) + (if (ctrl) 4 else 0)
        val cursor = when (key) {
            "ArrowUp" -> "A"; "ArrowDown" -> "B"; "ArrowRight" -> "C"; "ArrowLeft" -> "D"
            "Home" -> "H"; "End" -> "F"; else -> null
        }
        if (cursor != null) return if (modifier > 1) "\u001b[1;${modifier}$cursor"
            else (if (applicationCursor) "\u001bO" else "\u001b[") + cursor
        val tilde = when (key) {
            "Insert" -> 2; "Delete" -> 3; "PageUp" -> 5; "PageDown" -> 6
            "F5" -> 15; "F6" -> 17; "F7" -> 18; "F8" -> 19; "F9" -> 20
            "F10" -> 21; "F11" -> 23; "F12" -> 24; else -> null
        }
        if (tilde != null) return "\u001b[$tilde" + (if (modifier > 1) ";$modifier" else "") + "~"
        val fn = when (key) { "F1" -> "P"; "F2" -> "Q"; "F3" -> "R"; "F4" -> "S"; else -> null }
        if (fn != null) return if (modifier > 1) "\u001b[1;${modifier}$fn" else "\u001bO$fn"
        val value = when (key) {
            "Enter" -> "\r"; "Backspace" -> "\u007f"; "Escape" -> "\u001b"
            "Tab" -> if (shift) "\u001b[Z" else "\t"
            else -> if (key.length == 1 && ctrl) {
                val ch = key.uppercase()[0]
                when { ch in '@'..'_' -> (ch.code - 64).toChar().toString()
                    key == " " || key == "2" -> "\u0000"; key == "?" || key == "8" -> "\u007f"; else -> null }
            } else if (key.length == 1 && alt) key else null
        } ?: return null
        return if (alt) "\u001b$value" else value
    }

    /** Whole UTF-8 code points, never split surrogate pairs or exceed the input frame limit. */
    fun chunks(text: String, maxBytes: Int = 4096): List<String> {
        require(maxBytes >= 4)
        val result = mutableListOf<String>()
        var start = 0; var index = 0; var bytes = 0
        while (index < text.length) {
            val count = if (text[index].isHighSurrogate() && index + 1 < text.length && text[index + 1].isLowSurrogate()) 2 else 1
            val size = text.substring(index, index + count).encodeToByteArray().size
            if (bytes + size > maxBytes) { result.add(text.substring(start, index)); start = index; bytes = 0 }
            index += count; bytes += size
        }
        if (start < text.length) result.add(text.substring(start))
        return result
    }
}
