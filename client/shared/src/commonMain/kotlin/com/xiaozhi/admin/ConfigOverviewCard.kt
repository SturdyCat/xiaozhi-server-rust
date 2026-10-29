package com.xiaozhi.admin

import androidx.compose.runtime.Composable
import com.gearui.foundation.layout.Spacing
import com.gearui.foundation.primitives.Text
import com.gearui.theme.Theme
import com.tencent.kuikly.compose.foundation.layout.Arrangement
import com.tencent.kuikly.compose.foundation.layout.Column
import com.tencent.kuikly.compose.foundation.layout.Row
import com.tencent.kuikly.compose.foundation.layout.fillMaxWidth
import com.tencent.kuikly.compose.foundation.layout.padding
import com.tencent.kuikly.compose.ui.Alignment
import com.tencent.kuikly.compose.ui.Modifier
import com.xiaozhi.admin.kit.dividerH
import com.xiaozhi.admin.kit.pluginStatusBadge

/**
 * 「能力总览」section（`overview`）—— Compose 版，一次给两张卡：
 *
 * 1. [CapabilityOverviewCard]：`GET /api/plugins` 的**配置级**投影（用户选择 `enabled` +
 *    实际阶段 `phase` 分开显示）；
 * 2. [EngineHealthCard]（在 `EngineHealthCard.kt`）：`GET /api/health/engines` 的
 *    **运行期**合并结论（含本地探测），可点击跳到对应配置项修复。
 *
 * 两张卡都保留旧实现的三条呈现规则（`docs/plugin-architecture-unification.md` §5.3）：
 *
 * - **只在偏离时打标**：`active` 行不渲染徽标（一屏绿点等于没有信息）；
 * - **降级/异常必须显示原因**：`last_error` 行内展开（旧实现刻意超过 DSH 的"只有阶段没有原因"）；
 * - **首次加载失败 → 就地错误 + 「刷新」**，不弹 toast（toast 只用于用户动作的结果）。
 *
 * ⚠️ 为什么两张卡只留**一个** section 入口：`ConfigSectionHost` 每个 id 只调一个函数，
 * 页面顺序（总览在上、健康在下）属于本 section 的内部结构，不该泄漏到分发器。
 */
@Composable
fun OverviewSection(form: ConfigFormState, meta: PluginMetaState, health: EngineHealthState) {
    CapabilityOverviewCard(meta)
    EngineHealthCard(health) { section -> form.openConfigSection(section) }
}

/** 能力总览卡：`GET /api/plugins` 的可读投影。 */
@Composable
internal fun CapabilityOverviewCard(meta: PluginMetaState) {
    val pager = LocalAdminPagerOrThrow()
    LabeledCard(title = "能力总览") {
        SectionSummaryRow(
            summary = when {
                meta.loadError.isNotEmpty() -> "无法读取能力清单"
                meta.summary.isEmpty() -> if (meta.loading) "加载中…" else "—"
                else -> meta.summary
            },
            actionText = "刷新",
            busy = meta.loading,
            busyText = "刷新中…",
            onAction = { meta.refresh(pager) },
        )
        // 首次加载失败：就地错误 + 可行动指引（全部能力都查不到时最可能是没连上服务端）
        if (meta.loadError.isNotEmpty()) {
            WarnLine(meta.loadError + "（确认服务端地址可达后点「刷新」）", error = true)
        }
        // 加载成功但注册表为空：这不是"全部正常"，是服务端没给数据
        if (meta.loaded && meta.requiredRows.isEmpty() && meta.optionalRows.isEmpty()) {
            HintLine("服务端未返回任何能力（注册表为空？请查看服务端日志）。")
        }
        CapabilityGroup(title = "必需能力（缺失即启动失败）", rows = meta.requiredRows)
        CapabilityGroup(title = "可选能力（失败只降级）", rows = meta.optionalRows)
    }
}

/** 一个能力分组：标题 + 行列表（空分组整体不渲染，避免出现"只有标题"的空壳）。 */
@Composable
private fun CapabilityGroup(title: String, rows: List<CapabilityRow>) {
    if (rows.isEmpty()) return
    Text(
        text = title,
        modifier = Modifier.fillMaxWidth().padding(top = Spacing.md, bottom = Spacing.xs),
        style = Theme.typography.caption,
        color = Theme.colors.mutedForeground,
    )
    // Compose 循环：rows 是 SnapshotStateList，整表重建即重组，不需要桥接层
    for (row in rows) CapabilityRowView(row)
}

/**
 * 一行：能力名 + 状态徽标（仅偏离时）/ 实现名 + 生效语义 / 原因（降级、异常时）。
 *
 * 徽标走 [pluginStatusBadge]（唯一映射出处：`enabled`+`phase` 四态），本文件不另写 `when`。
 */
@Composable
private fun CapabilityRowView(row: CapabilityRow) {
    Column(modifier = Modifier.fillMaxWidth().padding(top = Spacing.sm)) {
        Row(
            modifier = Modifier.fillMaxWidth(),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.SpaceBetween,
        ) {
            Text(
                text = row.display,
                modifier = Modifier.weight(1f),
                style = Theme.typography.titleSmall,
                color = Theme.colors.foreground,
            )
            pluginStatusBadge(phase = row.phase, enabled = row.enabled)
        }
        Text(
            text = when {
                row.implDisplay.isEmpty() -> row.hotHint
                row.hotHint.isEmpty() -> row.implDisplay
                else -> "${row.implDisplay} · ${row.hotHint}"
            },
            modifier = Modifier.fillMaxWidth().padding(top = Spacing.xs),
            style = Theme.typography.caption,
            color = Theme.colors.mutedForeground,
        )
        // 降级/异常必须显示原因：用户看到「异常」却查不到原因，最容易招致"这功能没用"
        if (row.reason.isNotEmpty()) {
            WarnLine(row.reason, error = row.phase == "failed")
        }
        dividerH()
    }
}
