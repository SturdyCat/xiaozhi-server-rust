package com.xiaozhi.admin

import androidx.compose.runtime.Composable
import com.gearui.components.code.Code
import com.gearui.foundation.primitives.ScrollView
import com.gearui.foundation.primitives.Text
import com.gearui.foundation.scroll.rememberScrollState
import com.gearui.theme.Theme
import com.tencent.kuikly.compose.foundation.layout.Column
import com.tencent.kuikly.compose.foundation.layout.Row
import com.tencent.kuikly.compose.foundation.layout.fillMaxWidth
import com.tencent.kuikly.compose.foundation.layout.height
import com.tencent.kuikly.compose.foundation.layout.padding
import com.tencent.kuikly.compose.ui.Alignment
import com.tencent.kuikly.compose.ui.Modifier
import com.tencent.kuikly.compose.ui.unit.Dp
import com.tencent.kuikly.compose.ui.unit.dp
import com.xiaozhi.admin.kit.LocalAdminSpacing

/**
 * EchoMuse 六 tab 共用的**最小呈现原语**（Compose 版）。
 *
 * ## 为什么单独成文件
 *
 * 旧 DSL 里这些是 `ui/Rows.kt` 的 `infoRow` / `detailLine` / `groupLabel` / `outputBlock` +
 * `ui/Widgets.kt` 的 `emStatTile`。迁移后 `ui/` 要在 T12 整体删除，而这四类东西在
 * 状态/活动/配置/更新/日志/BLE 六个 tab 里被用了 80 多次——各自复制一份必然漂移
 * （典型：一处 `—` 是 em dash、另一处是空串，看起来像"还没渲染"）。
 *
 * 所以这里只放**与业务无关的排版与文案原语**，不放任何 EchoMuse 数据读取：
 * 数值取不到就是调用方传 [EM_DASH]，本文件不认识 `EchoMuseState`。
 *
 * ## 颜色纪律
 *
 * 全部取 `Theme.colors` 的语义档（`foreground` / `mutedForeground` / `warning` /
 * `destructive`），不出现任何色值字面量，也不出现旧 `AdminColors.*` 符号。
 */

/**
 * 缺测占位：全页统一（`—`，不是空串）。
 *
 * 空串在表格里看起来像"这一格还没渲染完"，而 `—` 明确是"没有这个值"。
 * 这条口径来自设备遥测的实际情况：`stats` 是白名单标量，设备不报就永远没有，
 * 页面不能拿 0 或空白糊过去（PRD「诚实展示」）。
 */
internal const val EM_DASH = "—"

/** 数值 → 文本；`null` / 非有限 = [EM_DASH]（缺测绝不与 0 混）。 */
internal fun emNum(value: Double?, decimals: Int = 0, unit: String = ""): String {
    if (value == null || !value.isFinite()) return EM_DASH
    val text = when (decimals) {
        0 -> kotlin.math.round(value).toLong().toString()
        1 -> (kotlin.math.round(value * 10.0) / 10.0).toString()
        else -> (kotlin.math.round(value * 100.0) / 100.0).toString()
    }
    return text + unit
}

/** 信号强度（dBm）。 */
internal fun emRssi(value: Double?): String = emNum(value, unit = " dBm")

/** WiFi 频率（MHz；设备上报的就是 MHz，不换算）。 */
internal fun emMhz(value: Double?): String = emNum(value, unit = " MHz")

/** 链路速率（Mbps）。 */
internal fun emMbps(value: Double?): String = emNum(value, unit = " Mbps")

/** 温度（℃）。 */
internal fun emCelsius(value: Double?): String = emNum(value, unit = " ℃")

/** MB → GB 文本（一位小数）；`null` = [EM_DASH]。 */
internal fun emGb(mb: Double?): String = if (mb == null) EM_DASH else emNum(mb / 1024.0, decimals = 1)

/** 占比（part/total → 0～1）；任一缺失或 `total <= 0` = `null`（不画半截进度条）。 */
internal fun emRatio(part: Double?, total: Double?): Double? {
    if (part == null || total == null || total <= 0.0) return null
    return (part / total).coerceIn(0.0, 1.0)
}

/** 从快照取一层数值（`obj[key]`）；非数字 = `null`。 */
internal fun emNumber(obj: com.tencent.kuikly.core.nvi.serialization.json.JSONObject?, key: String): Double? =
    (obj?.opt(key) as? Number)?.toDouble()

/** 分组小标题（原 DSL `groupLabel`）：上留白 + 小标题，行间不画线。 */
@Composable
internal fun EmGroupLabel(text: String, topGap: Dp = LocalAdminSpacing.current.lg) {
    Text(
        text = text,
        modifier = Modifier.fillMaxWidth().padding(top = topGap, bottom = LocalAdminSpacing.current.xs),
        style = Theme.typography.titleSmall,
        color = Theme.colors.foreground,
    )
}

/** 一句解释（原 DSL `detailLine` / `textNote`）：弱化小字，空串不占位。 */
@Composable
internal fun EmNote(text: String, modifier: Modifier = Modifier) {
    if (text.isEmpty()) return
    Text(
        text = text,
        modifier = modifier.fillMaxWidth().padding(top = 2.dp),
        style = Theme.typography.caption,
        color = Theme.colors.mutedForeground,
    )
}

/** 需要人注意的一句（原 DSL `textNote(tone = "warn" / "error")`）。 */
@Composable
internal fun EmWarn(text: String, error: Boolean = false, modifier: Modifier = Modifier) {
    if (text.isEmpty()) return
    Text(
        text = text,
        modifier = modifier.fillMaxWidth().padding(top = LocalAdminSpacing.current.xs),
        style = Theme.typography.caption,
        color = if (error) Theme.colors.destructive else Theme.colors.warning,
    )
}

/**
 * 一行「名称 │ 值」（原 DSL `infoRow`）。
 *
 * 值**不换行截断**：设备号、内核版本、链路三态都是要逐字核对的运维信息，截断等于隐藏证据。
 * 左列按 40% 固定，右列吃掉剩余宽度并允许折行（窄窗口下仍可读）。
 */
@Composable
internal fun EmInfoRow(label: String, value: String, topGap: Dp = 0.dp) {
    Row(
        modifier = Modifier.fillMaxWidth().padding(top = topGap, bottom = 2.dp),
        verticalAlignment = Alignment.Top,
    ) {
        Text(
            text = label,
            modifier = Modifier.weight(0.42f),
            style = Theme.typography.bodySmall,
            color = Theme.colors.mutedForeground,
        )
        Text(
            text = value,
            modifier = Modifier.weight(0.58f),
            style = Theme.typography.bodySmall,
            color = Theme.colors.foreground,
        )
    }
}

/**
 * 指标块（原 DSL `emStatTile`）：小标题 + 大号读数 + 一句副标。
 *
 * [hint] 必填：数字脱离口径就是误读来源（"成功率 100%" 与 "没测过" 长得一样）。
 */
@Composable
internal fun EmStatTile(
    label: String,
    value: String,
    hint: String,
    modifier: Modifier = Modifier,
) {
    Column(modifier = modifier.padding(top = LocalAdminSpacing.current.sm)) {
        Text(
            text = label,
            style = Theme.typography.caption,
            color = Theme.colors.mutedForeground,
        )
        Text(
            text = value,
            modifier = Modifier.padding(top = 2.dp),
            style = Theme.typography.titleMedium,
            color = Theme.colors.foreground,
        )
        EmNote(hint)
    }
}

/**
 * 只读输出块（原 DSL `outputBlock`）：定高 + 可滚动 + 等宽。
 *
 * 用于系统信息、日志与 GATT 原始响应——这些是**设备原文**，不是终端会话，
 * 所以不进 `GearTerminal`（K2 只覆盖控制台 tab 与日志流页），用 GearUI `Code` 的等宽档。
 * 定高是刻意的：几百行原文铺开会把上面的按钮挤到屏外。
 */
@Composable
internal fun EmOutputBlock(text: String, height: Dp, modifier: Modifier = Modifier) {
    ScrollView(
        modifier = modifier.fillMaxWidth().height(height),
        state = rememberScrollState(),
    ) {
        Code(text = text)
    }
}
