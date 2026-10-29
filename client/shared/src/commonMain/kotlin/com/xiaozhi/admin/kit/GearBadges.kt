package com.xiaozhi.admin.kit

import androidx.compose.runtime.Composable
import com.gearui.components.tag.Tag
import com.gearui.components.tag.TagSize
import com.gearui.components.tag.TagTheme
import com.gearui.components.tag.TagVariant
import com.tencent.kuikly.compose.foundation.background
import com.tencent.kuikly.compose.foundation.layout.Box
import com.tencent.kuikly.compose.foundation.layout.size
import com.tencent.kuikly.compose.foundation.shape.CircleShape
import com.tencent.kuikly.compose.ui.Modifier
import com.tencent.kuikly.compose.ui.draw.clip
import com.tencent.kuikly.compose.ui.graphics.Color
import com.tencent.kuikly.compose.ui.unit.Dp
import com.tencent.kuikly.compose.ui.unit.dp
import com.gearui.theme.Theme

/**
 * 状态徽标族（Compose 版）—— 旧 `ui/Widgets.kt` 的 `dot` / `statusBadge` / `pluginStatusBadge`。
 *
 * 承载换成 GearUI `Tag`，但**四态语义与「正常不打标」规则一字不改**（K3）：
 *
 * | 业务态 | TagTheme | 圆点色 |
 * |---|---|---|
 * | `connected` | SUCCESS | `colors.success` |
 * | `recording` / `error` | DANGER | `colors.destructive` |
 * | `busy` | WARNING | `colors.warning` |
 * | 其他（`idle` / `active`） | DEFAULT | `colors.mutedForeground` |
 *
 * 旧实现是「自绘胶囊 + 圆点 + Text」，新版是「`Tag(variant = LIGHT)` + `icon` 槽放圆点」——
 * 胶囊高度、内距、字号交给 GearUI 主题（方案 §4.1-11 已登记为允许的视觉差异），
 * **但要保留圆点**：圆点是「状态」的一等信息，纯文字标签会丢掉它。
 */

/**
 * 业务态 → GearUI `TagTheme` 的唯一映射出处。
 * 集中在这里，避免各页面各写一份 `when` 后语义漂移。
 */
internal fun statusTagTheme(state: String): TagTheme = when (state) {
    "connected" -> TagTheme.SUCCESS
    "recording", "error" -> TagTheme.DANGER
    "busy" -> TagTheme.WARNING
    else -> TagTheme.DEFAULT
}

/** 业务态 → 圆点色。与 [statusTagTheme] 同源，保证胶囊底色与圆点色始终属同一档语义。 */
@Composable
internal fun statusDotColor(state: String): Color = when (state) {
    "connected" -> Theme.colors.success
    "recording", "error" -> Theme.colors.destructive
    "busy" -> Theme.colors.warning
    else -> Theme.colors.mutedForeground
}

/**
 * 状态圆点（在线/离线、菜单告警、徽标前置点）：纯色小圆。
 *
 * 与相邻元素的间距由调用方给（不内置 margin，避免和两侧留白重复计算）。
 */
@Composable
fun dot(color: Color, size: Dp = 8.dp) {
    Box(
        Modifier
            .size(size)
            .clip(CircleShape)
            .background(color),
    )
}

/**
 * 状态徽标：胶囊 + 圆点 + 文案，颜色按 [state] 分四档。
 *
 * 无障碍：`Tag` 的文本本身就是可读内容，圆点是纯装饰（不参与语义）。
 */
@Composable
fun statusBadge(state: String, text: String) {
    Tag(
        text = text,
        theme = statusTagTheme(state),
        size = TagSize.MEDIUM,
        variant = TagVariant.LIGHT,
        icon = { dot(color = statusDotColor(state), size = 8.dp) },
    )
}

/**
 * 能力状态徽标（`GET /api/plugins` 的 `phase` + `enabled`）。
 *
 * **只在偏离时打标**（照 DSH 的规则）：`enabled && phase == active` 是"正常"，
 * **不渲染任何东西** —— 否则一屏绿点等于没有信息。
 *
 * `enabled`（用户保存的选择）与 `phase`（实际状态）是两种信息，必须都看：
 * 「已启用但加载失败」显示「异常」，「用户关掉了」显示「已关闭」。
 *
 * 与 [statusBadge] 一样接收**快照值**：调用方（总览页）每次刷新整表重建，行本身是不可变快照。
 */
@Composable
fun pluginStatusBadge(phase: String, enabled: Boolean) {
    when {
        !enabled || phase == "disabled" -> statusBadge(state = "idle", text = "已关闭")
        phase == "degraded" -> statusBadge(state = "busy", text = "降级")
        phase == "failed" -> statusBadge(state = "error", text = "异常")
        phase == "active" -> Unit // 正常不打标
        else -> statusBadge(state = "idle", text = phase)
    }
}
