package com.xiaozhi.admin

import kotlinx.datetime.TimeZone
import kotlin.test.Test
import kotlin.test.assertEquals

/**
 * 时间展示口径回归：`formatEpochSeconds` 的格式契约。
 *
 * 这组断言的存在意义是**钉住四端收敛后的输出**——此前 js / Android / iOS / macOS 各有一份
 * 实现（手拼 Date getter / SimpleDateFormat / NSDateFormatter），同一时间戳在四种客户端上
 * 可能因为日历或 locale 差异显示不同结果（泰语佛历、日语和历是最典型的坑）。现在统一由
 * kotlinx-datetime 换算，用固定时区断言即可复现，不依赖运行机器的系统时区。
 */
class TimeFormatTest {

    private val utc = TimeZone.UTC
    private val shanghai = TimeZone.of("Asia/Shanghai")
    private val newYork = TimeZone.of("America/New_York")

    // ============================================================
    // 格式契约
    // ============================================================

    @Test fun formatsAsFixedWidthLocalDateTime() {
        // 2023-11-14T22:13:20Z：四段补零都要命中（月/日/时/分/秒）
        assertEquals("2023-11-14 22:13:20", formatEpochSeconds(1_700_000_000L, utc))
    }

    @Test fun padsSingleDigitComponents() {
        // 2000-01-01T00:00:00Z：月/日/时/分/秒全部是 0 或 1 → 补零成固定列宽
        assertEquals("2000-01-01 00:00:00", formatEpochSeconds(946_684_800L, utc))
        assertEquals("1970-01-01 00:00:01", formatEpochSeconds(1L, utc))
    }

    @Test fun appliesGivenZone() {
        // 同一时间戳在三个时区：跨日、跨月边界都要按 zone 折算
        assertEquals("2023-11-15 06:13:20", formatEpochSeconds(1_700_000_000L, shanghai))
        assertEquals("2023-11-14 17:13:20", formatEpochSeconds(1_700_000_000L, newYork))
    }

    @Test fun followsDaylightSavingOfZone() {
        // 纽约 DST：2023-03-12 当地 02:00 EST 直接跳到 03:00 EDT。
        // 1678602000 = 2023-03-12T06:20:00Z → 切换前 01:20（EST，UTC−5）
        // 1678618200 = 2023-03-12T10:50:00Z → 切换后 06:50（EDT，UTC−4）
        assertEquals("2023-03-12 01:20:00", formatEpochSeconds(1_678_602_000L, newYork))
        assertEquals("2023-03-12 06:50:00", formatEpochSeconds(1_678_618_200L, newYork))
    }

    // ============================================================
    // 未给时间的锚点（与 FilesModel.mtimeText 同口径）
    // ============================================================

    @Test fun treatsNonPositiveAsMissing() {
        assertEquals("", formatEpochSeconds(0L, utc))
        assertEquals("", formatEpochSeconds(-1L, utc))
        assertEquals("", formatEpochSeconds(-100L, shanghai))
    }
}
