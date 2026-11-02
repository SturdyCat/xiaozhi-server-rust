package com.xiaozhi.admin.kit
import com.xiaozhi.admin.kit.LocalAdminSpacing

import androidx.compose.runtime.Composable
import com.gearui.components.popover.Popover
import com.gearui.components.popover.rememberPopoverState
import com.gearui.components.popup.Popup
import com.gearui.foundation.primitives.ScrollView
import com.gearui.foundation.primitives.Text
import com.gearui.theme.Theme
import com.tencent.kuikly.compose.foundation.background
import com.tencent.kuikly.compose.foundation.clickable
import com.tencent.kuikly.compose.foundation.layout.Arrangement
import com.tencent.kuikly.compose.foundation.layout.Column
import com.tencent.kuikly.compose.foundation.layout.Row
import com.tencent.kuikly.compose.foundation.layout.fillMaxWidth
import com.tencent.kuikly.compose.foundation.layout.heightIn
import com.tencent.kuikly.compose.foundation.layout.padding
import com.tencent.kuikly.compose.foundation.layout.widthIn
import com.tencent.kuikly.compose.foundation.shape.RoundedCornerShape
import com.tencent.kuikly.compose.ui.Alignment
import com.tencent.kuikly.compose.ui.Modifier
import com.tencent.kuikly.compose.ui.draw.clip
import com.tencent.kuikly.compose.ui.unit.dp
import com.xiaozhi.admin.contracts.AdminFieldHelpState
import com.xiaozhi.admin.contracts.IconToken
import com.xiaozhi.admin.icons.GearIcon

/**
 * 字段帮助（ⓘ tooltip）—— Compose 版。
 *
 * ## 交互口径（产品决策 2026-10：key 不上界面，中文名 + 按需解释）
 *
 * - 界面只出**中文见名知意**的字段名（CPU / AI 这类通用术语保留英文），配置 key 不上界面；
 * - 有复杂明细的字段：名称旁挂 ⓘ（[GearInfoBadge]），点开弹层看完整说明；
 * - 见名知意的字段（端口 / 语速 / 声道数…）：**不挂** ⓘ（由调用方传 `help = ""` 控制）；
 * - 弹层关闭方式：**点外部关闭**、点「知道了」关闭；同一时刻只有一个弹层。
 *
 * ## 页面级单例 vs 逐字段锚定（两条路径，语义不同，都保留）
 *
 * 1. **[GearFieldHelpDialog]**（页面级单例）：状态由页面持有（`LocalFieldHelp`），
 *    整页只渲染一个弹层节点 —— 一张卡几十个字段若各挂一个弹层，节点数会翻倍。
 *    它**没有锚点**（页面级弹层不属于某个字段），因此用 GearUI 的 `Popup.Host`
 *    （非模态、点外部关闭、不阻塞滚动），而不是需要 `trigger` 锚定的 `Popover`。
 * 2. **[GearInfoPopover]**（逐字段锚定）：调用方需要"气泡贴着徽标弹出"时用，
 *    走 GearUI `Popover`（自带定位/翻转/点外部关闭）。
 *
 * 方案 §4.1-14 要求「窄屏回落 Popup」—— 第 1 条正是那条 Popup 路径，**宽窄屏通用**；
 * 第 2 条提供 Popover 能力给需要锚定的场景。两种路径都由 `LocalFieldHelp` 或各自 state 驱动，
 * 不会出现"点了 A 字段的 ⓘ、B 字段的弹层亮"。
 */

/** 帮助弹层正文区的最大高度（px 换算成 dp 前的上限，超长时内部滚动）。 */
private val HelpBodyMaxHeight = 300.dp

/** 帮助弹层卡片的最大宽度（小屏按窗口自适应）。 */
private val HelpCardMaxWidth = 360.dp

/**
 * 帮助弹层的外观壳：与卡片同一套观感（`Theme.colors.popover` 底 + 圆角 + 内距）。
 *
 * 正文限高 `300.dp` 并可滚动（用 GearUI `ScrollView`；Kuikly Compose 2.30.0 的
 * `foundation` 包没有 `Modifier.verticalScroll`，见 `kit/GearLayout.kt` 的同类说明）。
 */
@Composable
private fun GearHelpCard(title: String, text: String, onDismiss: () -> Unit) {
    Column(
        modifier = Modifier
            .widthIn(max = HelpCardMaxWidth)
            .clip(RoundedCornerShape(12.dp))
            .background(Theme.colors.popover)
            .padding(LocalAdminSpacing.current.cardPadding),
    ) {
        Text(
            text = title,
            modifier = Modifier.padding(bottom = LocalAdminSpacing.current.sm),
            style = Theme.typography.titleSmall,
            color = Theme.colors.popoverForeground,
        )
        // 正文可能很长（整段字段说明收进弹层）：限高滚动，避免小屏溢出。
        // 用 GearUI 的 ScrollView —— Kuikly Compose 2.30.0 没有 `Modifier.verticalScroll`。
        ScrollView(
            modifier = Modifier
                .fillMaxWidth()
                .heightIn(max = HelpBodyMaxHeight),
        ) {
            Text(
                text = text,
                style = Theme.typography.bodyMedium,
                color = Theme.colors.mutedForeground,
            )
        }
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .padding(top = LocalAdminSpacing.current.sm),
            horizontalArrangement = Arrangement.End,
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Text(
                text = "知道了",
                modifier = Modifier
                    .clip(RoundedCornerShape(8.dp))
                    .clickable { onDismiss() }
                    .padding(horizontal = LocalAdminSpacing.current.sm, vertical = LocalAdminSpacing.current.xs),
                style = Theme.typography.label,
                color = Theme.colors.primary,
            )
        }
    }
}

/**
 * ⓘ 信息徽标：可点，点击后由页面级 [AdminFieldHelpState] 打开帮助弹层。
 *
 * `LocalFieldHelp` 为 null（页面未提供宿主）时**不渲染** —— 与旧 `fieldHelpState()` 返回 null
 * 的语义一致（该页不渲染 ⓘ）。
 */
@Composable
fun GearInfoBadge(title: String, help: String) {
    val helpState = LocalFieldHelp.current ?: return
    if (help.isEmpty()) return
    GearIcon(
        token = IconToken.Info,
        size = 16.dp,
        tint = Theme.colors.mutedForeground,
        onClick = { helpState.show(title, help) },
        modifier = Modifier.padding(start = 6.dp),
    )
}

/**
 * 页面级帮助弹层：**每页根部渲染一次**，显隐/内容全部由 [LocalFieldHelp] 驱动。
 *
 * 未实现宿主或当前不可见时什么都不渲染（零节点）。
 */
@Composable
fun GearFieldHelpDialog() {
    val helpState = LocalFieldHelp.current ?: return
    Popup.Host(
        visible = helpState.visible,
        anchorBounds = null,
        dismissOnOutside = true,
        onDismiss = { helpState.dismiss() },
    ) {
        GearHelpCard(
            title = helpState.title,
            text = helpState.text,
            onDismiss = { helpState.dismiss() },
        )
    }
}

/**
 * 逐字段锚定的帮助气泡（GearUI `Popover`）：气泡贴着 ⓘ 弹出，点外部关闭。
 *
 * 与 [GearInfoBadge] 的区别：这里**不依赖**页面级状态，适合"一次性说明"（如测试台的参数释义），
 * 代价是每个字段一个弹层节点。
 */
@Composable
fun GearInfoPopover(title: String, help: String) {
    if (help.isEmpty()) return
    val state = rememberPopoverState()
    Popover(
        state = state,
        closeOnClickOutside = true,
        content = {
            GearHelpCard(title = title, text = help, onDismiss = { state.hide() })
        },
        trigger = { onClick ->
            GearIcon(
                token = IconToken.Info,
                size = 16.dp,
                tint = Theme.colors.mutedForeground,
                modifier = Modifier.padding(start = 6.dp),
                onClick = onClick,
            )
        },
    )
}
