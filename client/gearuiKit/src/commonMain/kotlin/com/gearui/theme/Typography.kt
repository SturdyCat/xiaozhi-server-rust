package com.gearui.theme

import androidx.compose.runtime.Immutable
import com.gearui.foundation.typography.TextStyle

/**
 * GearUI semantic type scale — the themeable typography axis.
 *
 * This is the type [Theme.typography] resolves to, and the type a brand
 * replaces when it wants different type without forking components.
 *
 * It carries [com.gearui.foundation.typography.TextStyle], the same token type
 * the `Text` primitive accepts. Previously it carried Kuikly's `TextStyle`
 * instead, which meant the themed scale and the static scale were structurally
 * incompatible — nothing in the library could read a field off it, and every
 * component reached past the theme to the static
 * [com.gearui.foundation.typography.Typography] object. Unifying the two types
 * is what makes the axis actually replaceable.
 *
 * Usage:
 * ```kotlin
 * Text(text, style = Theme.typography.bodyMedium)
 * ```
 *
 * ⚠️ Component code uses ONLY these semantic styles.
 * No `fontSize = xx.sp` and no hardcoded values.
 */
@Immutable
data class Typography(

    /* ---------- Display (oversized headings) ---------- */

    /** Display Large - marketing heading */
    val displayLarge: TextStyle,

    /** Display Medium */
    val displayMedium: TextStyle,

    /* ---------- Headline (large headings) ---------- */

    /** Headline Large */
    val headlineLarge: TextStyle,

    /** Headline Medium */
    val headlineMedium: TextStyle,

    /** Headline Small */
    val headlineSmall: TextStyle,

    /* ---------- Title (headings) ---------- */

    /** Title Extra Large */
    val titleExtraLarge: TextStyle,

    /** Title Large */
    val titleLarge: TextStyle,

    /** Title Medium */
    val titleMedium: TextStyle,

    /** Title Small */
    val titleSmall: TextStyle,

    /* ---------- Body (body copy) ---------- */

    /** Body Extra Large */
    val bodyExtraLarge: TextStyle,

    /** Body Large */
    val bodyLarge: TextStyle,

    /** Body Medium (most used) */
    val bodyMedium: TextStyle,

    /** Body Small */
    val bodySmall: TextStyle,

    /** Body Extra Small */
    val bodyExtraSmall: TextStyle,

    /* ---------- Mark (emphasis) ---------- */

    /** Mark Large - bold */
    val markLarge: TextStyle,

    /** Mark Medium - bold */
    val markMedium: TextStyle,

    /** Mark Small - bold */
    val markSmall: TextStyle,

    /** Mark Extra Small - bold */
    val markExtraSmall: TextStyle,

    /* ---------- Link ---------- */

    /** Link Large */
    val linkLarge: TextStyle,

    /** Link Medium */
    val linkMedium: TextStyle,

    /** Link Small */
    val linkSmall: TextStyle,

    /* ---------- Caption / Label ---------- */

    /** Caption - supporting note */
    val caption: TextStyle,

    /** Label - label / badge */
    val label: TextStyle,
) {
    /** Return a copy with every style scaled by [factor] (fontScale). Spacing tokens untouched. */
    fun scale(factor: Float): Typography = Typography(
        displayLarge = displayLarge.scale(factor),
        displayMedium = displayMedium.scale(factor),
        headlineLarge = headlineLarge.scale(factor),
        headlineMedium = headlineMedium.scale(factor),
        headlineSmall = headlineSmall.scale(factor),
        titleExtraLarge = titleExtraLarge.scale(factor),
        titleLarge = titleLarge.scale(factor),
        titleMedium = titleMedium.scale(factor),
        titleSmall = titleSmall.scale(factor),
        bodyExtraLarge = bodyExtraLarge.scale(factor),
        bodyLarge = bodyLarge.scale(factor),
        bodyMedium = bodyMedium.scale(factor),
        bodySmall = bodySmall.scale(factor),
        bodyExtraSmall = bodyExtraSmall.scale(factor),
        markLarge = markLarge.scale(factor),
        markMedium = markMedium.scale(factor),
        markSmall = markSmall.scale(factor),
        markExtraSmall = markExtraSmall.scale(factor),
        linkLarge = linkLarge.scale(factor),
        linkMedium = linkMedium.scale(factor),
        linkSmall = linkSmall.scale(factor),
        caption = caption.scale(factor),
        label = label.scale(factor),
    )
}

/* --------------------------------------------------------- */
/* --------------------------------------------------------- */

object Typographies {
    /**
     * The type scale GearUI uses unless a host picks another one: the reference scale
     * (HeroUI Native), 12/16, 14/20, 16/24, 18/28, with emphasis at 500.
     *
     * It is one scale on every platform on purpose. A per-platform scale made the same
     * screen a different size on iOS than on the web, so a layout checked on one could
     * not be trusted on the other, and the reference's own metrics could never be met.
     */
    val Default: Typography = TypographyProfiles.Reference

    /**
     * The reference scale, named for callers that want to be explicit. Same object as
     * [Default].
     */
    val Reference: Typography = TypographyProfiles.Reference

    /**
     * 历史平台档位（iOS 17/20 量度，emphasis 600）。自 rem 排版体系落地后，平台差异
     * 统一由 `fontScale` 体现，单套 canonical ramp（[Reference]）即覆盖全端。保留别名以兼容
     * 旧调用点，新代码请用 [Reference] / [Default]。
     */
    @Deprecated("平台差异现由 fontScale 体现；统一使用 Reference")
    val Platform: Typography = TypographyProfiles.Reference

    /** 历史 web 档位（15 body）。同上，已并入 [Reference]，由 fontScale 体现密度差异。 */
    @Deprecated("平台差异现由 fontScale 体现；统一使用 Reference")
    val Web: Typography = TypographyProfiles.Reference
}
