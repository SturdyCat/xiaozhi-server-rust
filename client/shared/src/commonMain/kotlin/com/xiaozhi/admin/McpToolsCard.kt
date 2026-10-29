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
import com.tencent.kuikly.compose.ui.unit.dp
import com.xiaozhi.admin.contracts.IconToken
import com.xiaozhi.admin.icons.GearIcon
import com.xiaozhi.admin.kit.dividerH
import com.xiaozhi.admin.kit.statusBadge

/**
 * 「MCP 工具」section（`mcp`）—— Compose 版：`GET /api/mcp/tools` 的可读投影。
 *
 * 为什么要有这一页：MCP 工具清单**只存在于固件里**（`main/boards/<板>/<板>.cc` 的
 * `InitializeTools()` 调 `AddTool`），服务器不问就不知道。此前想知道"这台设备能被 AI 指挥
 * 做什么"只能翻设备串口日志或读固件源码；现在会话开始时服务器会通过 MCP
 * `initialize` + `tools/list` 拉一次，这里直接显示。
 *
 * 呈现规则（与「总览」一致，**只在偏离时打标**）：
 *
 * - 在线不打标（只有"已断开"才显示），正常工具不打标；
 * - 失败必须显示原因（`error` 行内展开），并说清"下面这些是上次成功的结果"；
 * - 空列表给可行动的解释（用服务端的 `note`：设备空闲时不建连，所以刚部署时是空的）；
 * - 每个工具显示 **name / 参数摘要 / description 原文**——description 就是喂给大模型的提示词，
 *   显示原文才能解释"为什么 AI 会这么用这个工具"。
 *
 * ⚠️ 只读投影：不参与 `POST /api/config`，本段没有保存语义。
 */
@Composable
fun McpSection(mcp: McpToolsState) {
    val pager = LocalAdminPagerOrThrow()
    LabeledCard(title = "MCP 工具（ESP 设备上报）") {
        SectionSummaryRow(
            summary = when {
                mcp.loadError.isNotEmpty() -> "无法读取设备工具清单"
                mcp.summary.isEmpty() -> if (mcp.loading) "加载中…" else "—"
                else -> mcp.summary
            },
            actionText = "刷新",
            busy = mcp.loading,
            busyText = "刷新中…",
            onAction = { mcp.refresh(pager) },
        )
        // 首次加载失败：就地错误 + 可行动指引（不弹 toast —— toast 只用于用户动作的结果）
        if (mcp.loadError.isNotEmpty()) {
            WarnLine(mcp.loadError + "（确认服务端地址可达后点「刷新」）", error = true)
        }
        // 空列表：解释"为什么什么都没有"，并给出让它出现的方法（说清楚不是坏了）
        if (mcp.loaded && mcp.devices.isEmpty()) {
            HintLine(
                mcp.note.ifEmpty {
                    "还没有任何设备上报过 MCP 工具。设备在唤醒建立连接时才会被探测（OTA 不算），" +
                        "让设备唤醒一次后再点「刷新」。"
                },
            )
        }
        // Compose 循环：devices 是 SnapshotStateList，刷新整表重建即重组
        for (device in mcp.devices) DeviceBlock(device)
    }
}

/** 一台设备：身份 + 在线状态 + 探测时间 + 工具清单。 */
@Composable
private fun DeviceBlock(device: DeviceToolsRow) {
    Column(modifier = Modifier.fillMaxWidth().padding(top = Spacing.md)) {
        // 设备块标题行：左设备号 │ 右偏离徽标
        Row(
            modifier = Modifier.fillMaxWidth(),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(Spacing.sm),
        ) {
            Text(
                text = device.deviceId.ifEmpty { "(设备未报 id)" },
                modifier = Modifier.weight(1f),
                style = Theme.typography.titleSmall,
                color = Theme.colors.foreground,
            )
            // 只在偏离时打标：在线是正常态不占视觉；断开才显示（数据是"上次探测"）
            if (!device.online) statusBadge(state = "idle", text = "已断开")
            // P1-10 指令闸门兼容标记：偏离态才占视觉（兼容 = 正常态不显示）
            if (device.commandGate == "will_degrade") {
                statusBadge(state = "busy", text = device.commandGateLabel)
            }
        }
        if (device.commandGate == "compatible") {
            // 兼容标记：对勾走内嵌矢量图标，文案里不再塞「✓」字符
            Row(
                modifier = Modifier.fillMaxWidth().padding(top = Spacing.xs),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(Spacing.xs),
            ) {
                GearIcon(token = IconToken.Check, size = 12.dp, tint = Theme.colors.mutedForeground)
                Text(
                    text = "${device.commandGateLabel}（说「退下」会保持连接）",
                    style = Theme.typography.caption,
                    color = Theme.colors.mutedForeground,
                )
            }
        }
        Text(
            text = device.subtitle,
            modifier = Modifier.fillMaxWidth().padding(top = Spacing.xs),
            style = Theme.typography.caption,
            color = Theme.colors.mutedForeground,
        )
        Text(
            text = device.toolSummary,
            modifier = Modifier.fillMaxWidth().padding(top = Spacing.xs),
            style = Theme.typography.caption,
            color = Theme.colors.mutedForeground,
        )
        if (device.error.isNotEmpty()) {
            WarnLine(
                "最近一次探测未完成：${device.error}" +
                    if (device.tools.isEmpty()) "" else "（下面显示的是上次成功探测的结果）",
            )
        }
        if (device.tools.isEmpty()) {
            HintLine(
                "这台设备没有上报任何工具：可能是探测失败（见上），或固件未注册 MCP 工具" +
                    "（板级 `InitializeTools()` 里没有 `AddTool`）。",
            )
        }
        // tools 是普通不可变 List（快照），普通 for 即可
        for (tool in device.tools) ToolBlock(tool)
    }
}

/** 一个工具：name（+ 仅管理员标记）/ 参数摘要 / description 原文。 */
@Composable
private fun ToolBlock(tool: McpToolRow) {
    Column(modifier = Modifier.fillMaxWidth().padding(top = Spacing.sm)) {
        Row(
            modifier = Modifier.fillMaxWidth(),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(Spacing.sm),
        ) {
            Text(
                text = tool.name,
                modifier = Modifier.weight(1f),
                style = Theme.typography.bodyMedium,
                color = Theme.colors.foreground,
            )
            // 固件 `AddUserOnlyTool`：默认不出现在 tools/list、大模型看不到（服务器按名字调用）。
            // 用 warn 色而不是中性色——它正是"为什么 AI 不会用这个工具"的答案。
            if (tool.userOnly) statusBadge(state = "busy", text = "仅管理员")
        }
        Text(
            text = tool.signature,
            modifier = Modifier.fillMaxWidth().padding(top = Spacing.xs),
            style = Theme.typography.caption,
            color = Theme.colors.mutedForeground,
        )
        if (tool.description.isNotEmpty()) HintLine(tool.description)
        dividerH()
    }
}
