package com.xiaozhi.admin

import androidx.compose.runtime.Composable
import com.gearui.components.alert.Alert
import com.gearui.components.alert.AlertStatus
import com.gearui.components.button.Button
import com.gearui.components.button.ButtonTheme
import com.gearui.components.button.ButtonType
import com.gearui.components.empty.EmptyState
import com.gearui.components.skeleton.SkeletonText
import com.gearui.foundation.primitives.Text
import com.gearui.primitives.composite.Card
import com.gearui.theme.Theme
import com.tencent.kuikly.compose.foundation.layout.Column
import com.tencent.kuikly.compose.foundation.layout.ColumnScope
import com.tencent.kuikly.compose.foundation.layout.PaddingValues
import com.tencent.kuikly.compose.foundation.layout.fillMaxWidth
import com.tencent.kuikly.compose.foundation.layout.padding
import com.tencent.kuikly.compose.ui.Modifier
import com.tencent.kuikly.compose.ui.text.style.TextAlign
import com.tencent.kuikly.compose.ui.unit.dp
import com.xiaozhi.admin.contracts.IconToken
import com.xiaozhi.admin.icons.GearIcon
import com.xiaozhi.admin.kit.LocalAdminSpacing
import com.xiaozhi.admin.kit.pageScroller

/**
 * 运维面 / 存储面页面共用的**版式零件**（Compose）—— 旧 `OpsPages.kt` 顶部那三个
 * DSL 扩展（`opsHeader` / `opsError` / `opsSkeleton`）的等价物。
 *
 * ## 为什么集中在一处
 *
 * 「摘要 + 刷新」「失败就地报错 + 怎么修」「首次加载出骨架」这三件事在六个运维页 + 文件页
 * 里逐页重复。旧实现把它们写成 `ViewContainer` 扩展，谁都要先套 `pageScroller {}` 再逐项调；
 * Compose 版把它们收进一个 [OpsPageFrame]，页面只写「正文」一件事，就不会出现
 * 「A 页有刷新按钮、B 页忘了给」这种漂移。
 *
 * ## 三条贯穿全站的呈现规则（与迁移前逐字一致）
 *
 * 1. **只在偏离时打标**：正常/在线不占视觉，只有断开、异常、降级、被截断才打徽标；
 * 2. **失败必须给出原因 + 怎么修**：加载失败走 [OpsLoadError]（`Alert(DANGER)` + 「刷新」），
 *    不弹 toast —— toast 只用于"用户动作的结果"（上传、删除、注入）；
 * 3. **空列表给可行动的解释**：用 [OpsEmptyState]，说明"怎么样才会有数据"，而不是留白。
 *
 * ## 数字一律右对齐（P1-2）
 *
 * 磁贴的数值走 [OpsTile] 的 `textAlign = End`；表格的数值列给
 * `TableColumn(align = TableAlign.RIGHT)`。这条规则是运维页可读性的核心 ——
 * 一列左对齐的 ms/次数/令牌数没法纵向比较。
 *
 * 颜色只取 GearUI 语义色（`foreground` / `mutedForeground` / `destructive`），**不引入自研色值**。
 */

/**
 * 一页运维页的外壳：页边距 + 摘要/刷新头行 + 就地错误 + 首次加载骨架 + 正文。
 *
 * @param summary    头部摘要（服务端 `summary` 原文；空串时按加载态显示「加载中…」/「—」）
 * @param loading    当前是否在请求中（首行按钮显示「刷新中…」）
 * @param loaded     是否已成功加载过一次（决定是否出骨架：已有数据的刷新**不**回到骨架，
 *                   避免整页闪烁，刷新反馈由头行按钮承担）
 * @param loadError  首次加载失败原因（空串 = 无错误）
 * @param onRefresh  头行「刷新」与错误块「刷新」共用的回调
 * @param header     头行**之上**的可选插槽（概览页的就绪度横幅挂这里，仍在滚动区内）
 * @param content    正文
 */
@Composable
internal fun OpsPageFrame(
    summary: String,
    loading: Boolean,
    loaded: Boolean,
    loadError: String,
    onRefresh: () -> Unit,
    header: (@Composable ColumnScope.() -> Unit)? = null,
    content: @Composable ColumnScope.() -> Unit,
) {
    pageScroller {
        if (header != null) header()
        SectionSummaryRow(
            summary = summary.ifEmpty { if (loading) "加载中…" else "—" },
            actionText = "刷新",
            busy = loading,
            busyText = "刷新中…",
            onAction = onRefresh,
        )
        if (loadError.isNotEmpty()) {
            OpsLoadError(message = loadError, onRefresh = onRefresh)
        }
        if (loading && !loaded) {
            OpsSkeleton()
        } else {
            content()
        }
    }
}

/**
 * 加载失败的就地错误：[Alert]（DANGER）+ 「刷新」按钮。
 *
 * 文案里带上"确认服务端可达后点刷新"——只说"加载失败"等于让人自己猜是网络、鉴权还是服务端挂了。
 */
@Composable
internal fun OpsLoadError(message: String, onRefresh: () -> Unit) {
    Alert(
        title = "加载失败",
        description = "$message（确认服务端可达后点「刷新」）",
        status = AlertStatus.DANGER,
        modifier = Modifier.padding(top = LocalAdminSpacing.current.sm, bottom = LocalAdminSpacing.current.sm),
        action = {
            Button(
                text = "刷新",
                onClick = onRefresh,
                theme = ButtonTheme.DEFAULT,
                type = ButtonType.OUTLINE,
            )
        },
    )
}

/** 首次加载骨架：两段骨架行模拟"摘要 + 若干指标行"的节奏。 */
@Composable
internal fun OpsSkeleton() {
    Column(modifier = Modifier.fillMaxWidth().padding(top = LocalAdminSpacing.current.md)) {
        SkeletonText(lines = 3)
        SkeletonText(
            lines = 4,
            modifier = Modifier.fillMaxWidth().padding(top = LocalAdminSpacing.current.lg),
        )
    }
}

/**
 * 空态：**可行动的解释**，不是一句"暂无数据"。
 *
 * [description] 必须回答"怎么样才会有数据"，例如"设备只在唤醒时才建立连接"。
 */
@Composable
internal fun OpsEmptyState(message: String, description: String) {
    Card(modifier = Modifier.fillMaxWidth().padding(bottom = LocalAdminSpacing.current.lg)) {
        EmptyState(
            message = message,
            description = description.ifEmpty { null },
            icon = {
                GearIcon(
                    token = IconToken.Info,
                    size = 24.dp,
                    tint = Theme.colors.mutedForeground,
                )
            },
        )
    }
}

/**
 * 指标磁贴：标签（caption）+ 数值（titleMedium，**右对齐**）+ 可选口径说明（caption）。
 *
 * 数值右对齐是 P1-2 的要求：一列数字左对齐时位数不同就对不成列，无法一眼比大小。
 */
@Composable
internal fun OpsTile(label: String, value: String, note: String = "") {
    Card(
        modifier = Modifier.fillMaxWidth(),
        padding = PaddingValues(LocalAdminSpacing.current.sm),
    ) {
        Text(
            text = label,
            style = Theme.typography.caption,
            color = Theme.colors.mutedForeground,
        )
        Text(
            text = value,
            modifier = Modifier.fillMaxWidth().padding(top = LocalAdminSpacing.current.xs),
            style = Theme.typography.titleMedium,
            color = Theme.colors.foreground,
            textAlign = TextAlign.End,
            maxLines = 1,
        )
        if (note.isNotEmpty()) {
            Text(
                text = note,
                modifier = Modifier.fillMaxWidth().padding(top = LocalAdminSpacing.current.xs),
                style = Theme.typography.caption,
                color = Theme.colors.mutedForeground,
            )
        }
    }
}

/** 表格里的数值单元格（对齐由 `TableColumn(align = RIGHT)` 负责，这里只管字号/颜色）。 */
@Composable
internal fun OpsNumberCell(text: String) {
    Text(
        text = text,
        style = Theme.typography.bodyMedium,
        color = Theme.colors.foreground,
        maxLines = 1,
    )
}

/** 次要动作按钮（旧 `secondaryButton`）：`ButtonTheme.DEFAULT` + `OUTLINE`。 */
@Composable
internal fun OpsOutlineButton(
    text: String,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
    loading: Boolean = false,
) {
    Button(
        text = text,
        onClick = onClick,
        modifier = modifier,
        theme = ButtonTheme.DEFAULT,
        type = ButtonType.OUTLINE,
        disabled = !enabled,
        loading = loading,
    )
}

/** 主动作按钮（旧 `primaryButton`）：品牌色 + 实心。 */
@Composable
internal fun OpsPrimaryButton(
    text: String,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
    loading: Boolean = false,
) {
    Button(
        text = text,
        onClick = onClick,
        modifier = modifier,
        theme = ButtonTheme.PRIMARY,
        type = ButtonType.FILL,
        disabled = !enabled,
        loading = loading,
    )
}
