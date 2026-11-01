package com.xiaozhi.admin

import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNull

class EchoMuseVolumeTest {
    @Test fun reportsAreStrictMixerUnits() {
        for (value in listOf(null, "60", true, -1, 128, 0.5, Double.NaN, Double.POSITIVE_INFINITY)) {
            assertNull(EchoMuseVolume.reported(value), "$value must not become a valid volume")
        }
        for (level in 0..127) assertEquals(level, EchoMuseVolume.reported(level.toDouble()))
    }
    @Test fun sliderCommitRoundsAndClamps() {
        assertEquals(0, EchoMuseVolume.fromSlider(-10f))
        assertEquals(127, EchoMuseVolume.fromSlider(200f))
        assertEquals(61, EchoMuseVolume.fromSlider(60.8f))
        assertNull(EchoMuseVolume.fromSlider(Float.NaN))
        assertNull(EchoMuseVolume.fromSlider(Float.POSITIVE_INFINITY))
    }
    @Test fun displayedPercentComesFromReportedUnits() {
        assertEquals("0%", EchoMuseVolume.percent(0))
        assertEquals("47%", EchoMuseVolume.percent(60))
        assertEquals("100%", EchoMuseVolume.percent(127))
    }
}
