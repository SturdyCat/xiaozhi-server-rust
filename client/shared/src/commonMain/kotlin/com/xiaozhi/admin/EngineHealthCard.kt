package com.xiaozhi.admin

import androidx.compose.runtime.Composable
import com.gearui.foundation.layout.Spacing
import com.gearui.foundation.primitives.Text
import com.gearui.theme.Theme
import com.tencent.kuikly.compose.foundation.clickable
import com.tencent.kuikly.compose.foundation.layout.Arrangement
import com.tencent.kuikly.compose.foundation.layout.Column
import com.tencent.kuikly.compose.foundation.layout.Row
import com.tencent.kuikly.compose.foundation.layout.fillMaxWidth
import com.tencent.kuikly.compose.foundation.layout.padding
import com.tencent.kuikly.compose.ui.Alignment
import com.tencent.kuikly.compose.ui.Modifier
import com.tencent.kuikly.compose.ui.unit.dp
import com.xiaozhi.admin.contracts.IconToken
import com.xiaozhi.admin.icons.GearIcon
import com.xiaozhi.admin.kit.dividerH
import com.xiaozhi.admin.kit.statusBadge

/**
 * 引擎健康自检的两个渲染件（P1-6）—— Compose 版。
 *
 * - [HealthBanner]：**就绪度横幅**，只渲染偏离行（`degraded` / `failed`），可点击跳配置 section。
 *   正常时**一行都不渲染**（延续全站"只在偏离时打标"铁律：一屏绿点等于没有信息）。
 * - [EngineHealthCard]：**能力健康卡**（「总览」section、能力总览卡之下）。11 行 × phase 一览：
 *   偏离行打徽标 + 原因展开 + 可点击跳 section；正常行弱化一行带过；行尾显示当前实现 id。
 *
 * ## 四态颜色（K5）的唯一出处
 *
 * 徽标不自己配色，一律走 `kit/GearBadges.statusBadge`：
 *
 * | 业务态 | `state` | 颜色 |
 * | --- | --- | --- |
 * | 异常 `failed` | `"error"` | `colors.destructive`（红） |
 * | 降级 `degraded` | `"busy"` | `colors.warning`（黄） |
 * | 正常 `active` | `"connected"` | `colors.success`（绿） |
 * | 未知/已关闭 | `"idle"` | `colors.mutedForeground`（灰） |
 *
 * **行内仍遵守「只在偏离时打标」**：正常行不渲染绿胶囊（否则 11 行全绿等于没有信息），
 * 正常态用中性文字「正常」一行带过 —— 这是旧实现刻意定的规则，不是这次简化。
 *
 * ## 与 `/api/plugins` 的分工
 *
 * `Phase` 由服务端两处同源聚合（`PluginHost::capability_summary`），客户端**不自算任何状态**；
 * 本文件的 `probeState` / `probeDetail` 只是服务端给的探测原文投影。
 */
@Composable
fun EngineHealthCard(health: EngineHealthState, onJumpSection: (String) -> Unit) {
    val pager = LocalAdminPagerOrThrow()
    LabeledCard(title = "能力健康") {
        SectionSummaryRow(
            summary = when {
                health.loadError.isNotEmpty() -> "无法读取健康自检"
                health.summary.isEmpty() -> if (health.loading) "检测中…" else "—"
                else -> health.summary
            },
            actionText = "重新检测",
            busy = health.loading,
            busyText = "检测中…",
            onAction = { health.refresh(pager) },
        )
        // 首次加载失败：就地错误 + 可行动指引（不弹 toast —— toast 只用于用户动作的结果）
        if (health.loadError.isNotEmpty()) {
            WarnLine(health.loadError + "（确认服务端地址可达后点「重新检测」）", error = true)
        }
        if (health.loaded && health.rows.isEmpty()) {
            HintLine("服务端未返回任何能力（注册表为空？请查看服务端日志）。")
        }
        for (row in health.rows) HealthRowView(row, onJumpSection)
        HintLine("健康 = 配置校验（plan）与本地探测（模型文件 / 记忆目录可写）的合并结论；与「能力总览」同源。")
    }
}

/**
 * 就绪度横幅：仅当存在偏离（degraded/failed）行时渲染；[onJumpSection] 收到的是配置 section id。
 *
 * 加载失败**不渲染横幅本体**（健康卡内有就地错误 + 重新检测），横幅位置不留给空白。
 */
@Composable
fun HealthBanner(health: EngineHealthState, onJumpSection: (String) -> Unit) {
    if (!health.loaded || health.deviations.isEmpty()) return
    LabeledCard(title = "引擎健康（${health.deviations.size} 项偏离）") {
        for (row in health.deviations) BannerRow(row, onJumpSection)
        HintLine("点击条目可跳到对应配置项修复；探测为本地文件系统检查，服务端每次请求实时执行。")
    }
}

/** 横幅里的一行：能力名（偏离色）+ 徽标 + 原因摘要；有对应 section 时整行可点。 */
@Composable
private fun BannerRow(row: HealthRow, onJump: (String) -> Unit) {
    val jumpable = row.jumpable
    val clickModifier = if (jumpable) Modifier.clickable { onJump(row.configSection) } else Modifier
    Column(modifier = Modifier.fillMaxWidth().then(clickModifier).padding(vertical = Spacing.xs)) {
        Row(
            modifier = Modifier.fillMaxWidth(),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(Spacing.sm),
        ) {
            Text(
                text = row.display,
                style = Theme.typography.titleSmall,
                color = if (row.phase == "failed") Theme.colors.destructive else Theme.colors.warning,
            )
            statusBadge(
                state = if (row.phase == "failed") "error" else "busy",
                text = if (row.phase == "failed") "异常" else "降级",
            )
            if (jumpable) {
                // 行尾指示箭头走内嵌矢量图标（沿用旧实现的 ChevronRight，不用「›」字符当图标）
                GearIcon(
                    token = IconToken.ChevronRight,
                    size = 14.dp,
                    tint = Theme.colors.primary,
                )
            }
        }
        // 原因摘要（含「怎么修」）：横幅里也直接可见，不用点进健康卡
        if (row.reason.isNotEmpty()) HintLine(row.reason)
    }
}

/**
 * 健康卡里的一行。
 *
 * 偏离行：能力名 + 实现 + 徽标 + 原因 + 可跳转提示（跳转入口是行标题，整块可点）；
 * 正常/关闭行：名称 + 实现 + 状态词，全部弱化为次要文字色，不打徽标不展开。
 */
@Composable
private fun HealthRowView(row: HealthRow, onJump: (String) -> Unit) {
    val jumpable = row.jumpable
    val clickModifier = if (jumpable) Modifier.clickable { onJump(row.configSection) } else Modifier
    Column(modifier = Modifier.fillMaxWidth().then(clickModifier).padding(top = Spacing.sm)) {
        Row(
            modifier = Modifier.fillMaxWidth(),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(Spacing.sm),
        ) {
            Text(
                text = row.display,
                modifier = Modifier.weight(1f),
                style = if (row.isDeviation) Theme.typography.titleSmall else Theme.typography.bodyMedium,
                color = if (row.isDeviation) Theme.colors.foreground else Theme.colors.mutedForeground,
            )
            if (row.activeImpl.isNotEmpty()) {
                Text(
                    text = row.activeImpl,
                    style = Theme.typography.caption,
                    color = Theme.colors.mutedForeground,
                )
            }
            if (row.isDeviation) {
                statusBadge(
                    state = if (row.phase == "failed") "error" else "busy",
                    text = if (row.phase == "failed") "异常" else "降级",
                )
            } else {
                Text(
                    text = when (row.phase) {
                        "active" -> "正常"
                        "disabled" -> "已关闭"
                        else -> row.phase
                    },
                    style = Theme.typography.caption,
                    color = Theme.colors.mutedForeground,
                )
            }
        }
        if (row.isDeviation && row.reason.isNotEmpty()) {
            WarnLine(row.reason, error = row.phase == "failed")
        }
        if (jumpable) {
            Row(
                modifier = Modifier.fillMaxWidth().padding(top = Spacing.xs),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(Spacing.xs),
            ) {
                Text(
                    text = "点击跳到「${sectionLabelOf(row.configSection)}」配置项",
                    style = Theme.typography.caption,
                    color = Theme.colors.primary,
                )
                GearIcon(token = IconToken.ChevronRight, size = 12.dp, tint = Theme.colors.primary)
            }
        }
        dividerH()
    }
}

/** section id → 管理台导航标签（跳转提示文案用；`ConfigSections` 是唯一权威）。 */
internal fun sectionLabelOf(sectionId: String): String =
    ConfigSections.all.firstOrNull { it.id == sectionId }?.label ?: sectionId
