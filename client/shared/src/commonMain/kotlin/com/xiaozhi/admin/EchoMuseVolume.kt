package com.xiaozhi.admin

import kotlin.math.roundToInt

/** Device mixer units, not a persisted setting or a controller-side volume estimate. */
internal object EchoMuseVolume {
    const val MAX = 127

    /** Missing, fractional and out-of-range reports must not turn into a believable zero. */
    fun reported(value: Any?): Int? {
        val number = (value as? Number)?.toDouble() ?: return null
        return number.takeIf { it.isFinite() && it in 0.0..MAX.toDouble() && it == it.toInt().toDouble() }?.toInt()
    }

    fun fromSlider(value: Float): Int? =
        value.takeIf { it.isFinite() }?.roundToInt()?.coerceIn(0, MAX)

    fun percent(level: Int): String = "${(level.coerceIn(0, MAX) * 100.0 / MAX).roundToInt()}%"
}
