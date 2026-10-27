package com.xiaozhi.admin

import kotlinx.datetime.Clock
import kotlinx.datetime.Instant
import kotlinx.datetime.TimeZone
import kotlinx.datetime.offsetAt
import kotlinx.datetime.toLocalDateTime

/**
 * 时间戳展示口径（commonMain 唯一实现）。
 *
 * 引入 kotlinx-datetime 之前这里是 4 份 expect/actual：js 手拼 `Date` getter、Android
 * `SimpleDateFormat`、iOS/macOS `NSDateFormatter`。四份各写一遍既容易漂移，也踩过两个隐坑：
 *
 * - **日历**：Android 原实现用 `Locale.getDefault()`，泰语/昭和历区域会输出佛历/和历年份
 *   （如 2569 年），同一台服务器的时间在三种客户端上显示不同年份；
 * - **数字变体**：靠 locale 的 `en_US_POSIX` 兜住，漏一处就出现本地数字字形。
 *
 * 现在统一由 kotlinx-datetime 换算：固定 ISO 公历 + 手写补零，与时区无关的部分
 * （格式、日历、数字）在所有平台字节级一致；时区仍取系统本地时区（含 DST）。
 *
 * 保留的行为契约：`secs <= 0` 返回空串——服务端用 -1/0 表示"未给"，调用方据此显示
 * 占位符（如「接入时间未知」/ [EM_DASH]），与 [FilesModel.mtimeText] 同锚点。
 */
fun formatEpochSeconds(secs: Long): String =
    formatEpochSeconds(secs, TimeZone.currentSystemDefault())

/**
 * 同上，但时区由调用方给出。
 *
 * 时区抽成参数只为一个目的：让「格式契约」可被单测钉住（`adminCoreTests` 的
 * `TimeFormatTest` 用固定时区断言年月日时分秒与补零），与 [FilesModel.mtimeText]
 * 显式传 `zoneOffsetSeconds` 的思路一致。生产代码一律走无时区参数的重载。
 */
internal fun formatEpochSeconds(secs: Long, zone: TimeZone): String {
    if (secs <= 0) return ""
    val dt = Instant.fromEpochSeconds(secs).toLocalDateTime(zone)
    return buildString {
        append(dt.year.toString().padStart(4, '0'))
        append('-').append(dt.monthNumber.twoDigits())
        append('-').append(dt.dayOfMonth.twoDigits())
        append(' ').append(dt.hour.twoDigits())
        append(':').append(dt.minute.twoDigits())
        append(':').append(dt.second.twoDigits())
    }
}

/**
 * 本地时区**当前时刻**的 UTC 偏移（秒，东八区 = +28800）。
 *
 * 「文件」页 mtime 展示用：纯函数 [FilesModel.mtimeText] 不读系统时钟，页面把本值显式
 * 传入即可得到本地时间（列表行只按"当前偏移"折算，不追踪条目时刻的历史 DST——展示口径
 * 如此即可）。取「当前时刻」的偏移而非固定值，才能在 DST 切换后立刻反映新偏移。
 */
fun localZoneOffsetSeconds(): Int =
    TimeZone.currentSystemDefault().offsetAt(Clock.System.now()).totalSeconds

/** 两位补零（月份/日/时/分/秒共用）。 */
private fun Int.twoDigits(): String = if (this < 10) "0$this" else toString()
