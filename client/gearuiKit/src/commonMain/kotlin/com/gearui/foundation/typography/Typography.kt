package com.gearui.foundation.typography

import com.tencent.kuikly.compose.ui.text.font.FontWeight
import com.gearui.unit.TextUnit
import com.gearui.theme.Typographies
import com.tencent.kuikly.compose.ui.unit.sp

/** Root font size: 16sp == 1rem. Final rendered size = 16.sp * rem * fontScale. */
const val ROOT_FONT_SIZE_SP = 16f

/**
 * TextStyle - text style token
 *
 * Contains:
 * - fontSize
 * - lineHeight
 * - fontWeight
 */
data class TextStyle(
    val fontSize: TextUnit,
    val lineHeight: TextUnit,
    val fontWeight: FontWeight,
    val fontFamily: List<String> = listOf("system-ui"),
    val letterSpacing: TextUnit = 0.sp,
) {
    companion object {
        /**
         * Build a [TextStyle] from a rem value.
         *
         * Rendered size = [ROOT_FONT_SIZE_SP].sp * rem * fontScale. `sp` absorbs system
         * scaling, `rem` fixes the proportion, and `fontScale` (applied later via [scale])
         * does platform / viewport magnification — the three are orthogonal. Both fontSize
         * and lineHeight are stored in sp (line-height in sp, not dp) so Android measures the
         * line box correctly.
         */
        fun fromRem(
            rem: Double,
            lineHeightRatio: Double = 1.5,
            weight: FontWeight = FontWeight(400),
        ): TextStyle = TextStyle(
            fontSize = (ROOT_FONT_SIZE_SP * rem).sp,
            lineHeight = (ROOT_FONT_SIZE_SP * rem * lineHeightRatio).sp,
            fontWeight = weight,
        )
    }

    /** Multiply only fontSize and lineHeight by [factor] (fontScale). Other fields unchanged. */
    fun scale(factor: Float): TextStyle = copy(
        fontSize = fontSize * factor,
        lineHeight = lineHeight * factor,
    )
}

/** Static defaults use the generated platform profile. Components read Theme.typography. */
object Typography {
    val DisplayLarge: TextStyle = Typographies.Default.displayLarge
    val DisplayMedium: TextStyle = Typographies.Default.displayMedium
    val HeadlineLarge: TextStyle = Typographies.Default.headlineLarge
    val HeadlineMedium: TextStyle = Typographies.Default.headlineMedium
    val HeadlineSmall: TextStyle = Typographies.Default.headlineSmall
    val TitleExtraLarge: TextStyle = Typographies.Default.titleExtraLarge
    val TitleLarge: TextStyle = Typographies.Default.titleLarge
    val TitleMedium: TextStyle = Typographies.Default.titleMedium
    val TitleSmall: TextStyle = Typographies.Default.titleSmall
    val BodyExtraLarge: TextStyle = Typographies.Default.bodyExtraLarge
    val BodyLarge: TextStyle = Typographies.Default.bodyLarge
    val BodyMedium: TextStyle = Typographies.Default.bodyMedium
    val BodySmall: TextStyle = Typographies.Default.bodySmall
    val BodyExtraSmall: TextStyle = Typographies.Default.bodyExtraSmall
    val MarkLarge: TextStyle = Typographies.Default.markLarge
    val MarkMedium: TextStyle = Typographies.Default.markMedium
    val MarkSmall: TextStyle = Typographies.Default.markSmall
    val MarkExtraSmall: TextStyle = Typographies.Default.markExtraSmall
    val LinkLarge: TextStyle = Typographies.Default.linkLarge
    val LinkMedium: TextStyle = Typographies.Default.linkMedium
    val LinkSmall: TextStyle = Typographies.Default.linkSmall
    val Caption: TextStyle = Typographies.Default.caption
    val Label: TextStyle = Typographies.Default.label
}
