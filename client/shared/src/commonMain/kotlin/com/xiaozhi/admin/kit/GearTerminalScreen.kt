package com.xiaozhi.admin.kit

import androidx.compose.runtime.Composable
import com.tencent.kuikly.compose.foundation.background
import com.tencent.kuikly.compose.foundation.layout.*
import com.tencent.kuikly.compose.foundation.text.BasicText
import com.tencent.kuikly.compose.ui.Modifier
import com.tencent.kuikly.compose.ui.graphics.Color
import com.tencent.kuikly.compose.ui.text.*
import com.tencent.kuikly.compose.ui.text.font.FontFamily
import com.tencent.kuikly.compose.ui.text.font.FontStyle
import com.tencent.kuikly.compose.ui.text.font.FontWeight
import com.tencent.kuikly.compose.ui.text.style.TextDecoration
import com.tencent.kuikly.compose.ui.unit.dp
import com.tencent.kuikly.compose.ui.unit.sp
import com.xiaozhi.admin.TerminalLine

/** VT protocol colors, not a second application theme. Kept beside the shared terminal palette. */
private val ANSI_BASE = intArrayOf(0x000000, 0xcd0000, 0x00cd00, 0xcdcd00, 0x0000ee, 0xcd00cd, 0x00cdcd, 0xe5e5e5,
    0x7f7f7f, 0xff0000, 0x00ff00, 0xffff00, 0x5c5cff, 0xff00ff, 0x00ffff, 0xffffff)
private fun terminalColor(value: Int, fallback: Color): Color {
    if (value == -1) return fallback
    val rgb = when {
        value >= 0x01000000 -> value and 0xffffff
        value in 0..15 -> ANSI_BASE[value]
        value in 16..231 -> {
            val n = value - 16
            fun channel(x: Int) = if (x == 0) 0 else 55 + 40 * x
            (channel(n / 36) shl 16) or (channel(n / 6 % 6) shl 8) or channel(n % 6)
        }
        value in 232..255 -> { val n = 8 + (value - 232) * 10; (n shl 16) or (n shl 8) or n }
        else -> return fallback
    }
    return Color(0xff000000L or rgb.toLong())
}

/** Fixed-size cell row; styled native text supports ANSI colors instead of stripping escape data. */
@Composable
internal fun GearTerminalScreenLine(line: TerminalLine, cursorCol: Int? = null) {
    val annotated = buildAnnotatedString {
        append(line.text)
        for (span in line.spans) {
            val s = span.style
            val fg = terminalColor(s.foreground, GearTerminalPalette.fg)
            val bg = terminalColor(s.background, GearTerminalPalette.bg)
            addStyle(SpanStyle(color = if (s.inverse) bg else fg, background = if (s.inverse) fg else bg,
                fontWeight = if (s.bold) FontWeight.Bold else FontWeight.Normal,
                fontStyle = if (s.italic) FontStyle.Italic else FontStyle.Normal,
                textDecoration = if (s.underline) TextDecoration.Underline else TextDecoration.None), span.start, span.end)
        }
    }
    Box(modifier = Modifier.fillMaxWidth().height(16.dp)) {
        BasicText(text = annotated, softWrap = false, maxLines = 1,
            style = TextStyle(color = GearTerminalPalette.fg, fontSize = 12.sp, lineHeight = 16.sp,
                fontFamily = FontFamily.Monospace), modifier = Modifier.fillMaxWidth())
        if (cursorCol != null) Box(modifier = Modifier.offset(x = (cursorCol * 7.23f).dp)
            .width(7.23.dp).height(16.dp).background(GearTerminalPalette.accent.copy(alpha = 0.45f)))
    }
}
