package com.xiaozhi.admin

import com.xiaozhi.admin.ui.*

import com.tencent.kuikly.core.base.ViewContainer
import com.tencent.kuikly.core.directives.vfor
import com.tencent.kuikly.core.directives.vif
import com.tencent.kuikly.core.pager.Pager
import com.tencent.kuikly.core.views.Text
import com.tencent.kuikly.core.views.View

/**
 * 「MCP 工具」卡片（配置页的 `mcp` tab）：`GET /api/mcp/tools` 的可读投影。
 *
 * 为什么要有这一页：MCP 工具清单**只存在于固件里**（`main/boards/<板>/<板>.cc` 的
 * `InitializeTools()` 调 `AddTool`），服务器不问就不知道。此前想知道"这台设备能被 AI 指挥
 * 做什么"只能翻设备串口日志或读固件源码；现在会话开始时服务器会通过 MCP
 * `initialize` + `tools/list` 拉一次，这里直接显示。
 *
 * 呈现规则（与「总览」页一致）：
 * - **只在偏离时打标**：在线不打标（只有"已断开"才显示），正常工具不打标；
 * - **失败必须显示原因**（`error` 行内展开），并说清"下面这些是上次成功的结果"；
 * - **空列表给可行动的解释**（用服务端的 `note`：设备空闲时不建连，所以刚部署时是空的）；
 * - 每个工具显示 **name / 参数摘要 / description 原文**——description 就是喂给大模型的提示词，
 *   显示原文才能解释"为什么 AI 会这么用这个工具"。
 *
 * ⚠️ 渲染函数必须是 `ViewContainer` 扩展，并在目标容器闭包内**非限定**调用（见
 * `ConfigFormState` 文件头的 Kuikly DSL 绑定说明）。
 */
fun ViewContainer<*, *>.mcpToolsCard(mcp: McpToolsState, ctx: Pager) {
    groupedCard("MCP 工具（ESP 设备上报）") {
        // 摘要行：左侧计数、右侧刷新（手动触发 = 禁用/加载态由按钮自身处理）
        View {
            attr {
                flexDirectionRow()
                alignItemsCenter()
            }
            Text {
                attr {
                    flex(1f)
                    fontSize(AdminType.body)
                    color(AdminColors.textSecondary)
                    text(
                        when {
                            mcp.loadError.isNotEmpty() -> "无法读取设备工具清单"
                            mcp.summary.isEmpty() -> "加载中…"
                            else -> mcp.summary
                        },
                    )
                }
            }
            secondaryButton(
                "刷新",
                loading = { mcp.loading },
                loadingText = "刷新中…",
            ) {
                mcp.refresh(ctx)
            }
        }
        // 首次加载失败：就地错误 + 可行动指引（不弹 toast——toast 只用于用户动作的结果）
        vif({ mcp.loadError.isNotEmpty() }) {
            textNote(mcp.loadError + "（确认服务端地址可达后点「刷新」）", tone = "error", micro = true)
        }
        // 空列表：解释"为什么什么都没有"，并给出让它出现的方法（说清楚不是坏了）
        vif({ mcp.loaded && mcp.devices.size == 0 }) {
            textNote(
                mcp.note.ifEmpty {
                    "还没有任何设备上报过 MCP 工具。设备在**唤醒建立连接时**才会被探测（OTA 不算），" +
                        "让设备唤醒一次后再点「刷新」。"
                },
                micro = true,
            )
        }
        // devices 是 ObservableList（刷新后整表重建），用 vfor 保持响应式
        vfor({ mcp.devices }) { row ->
            deviceBlock(row)
        }
    }
}

/** 一台设备：身份 + 在线状态 + 探测时间 + 工具清单。 */
private fun ViewContainer<*, *>.deviceBlock(row: DeviceToolsRow) {
    View {
        attr {
            flexDirectionColumn()
            marginTop(AdminSpace.md)
        }
        // 设备块标题行：左设备号 │ 右偏离徽标（统一 headerRow）
        headerRow(
            title = row.deviceId.ifEmpty { "(设备未报 id)" },
            actions = {
                // 只在偏离时打标：在线是正常态，不占视觉；断开才显示（数据是"上次探测"）
                if (!row.online) {
                    statusBadge(state = { "idle" }, text = { "已断开" })
                }
                // P1-10 指令闸门兼容标记：偏离态才占视觉（兼容 = 正常态不显示）
                if (row.commandGate == "will_degrade") {
                    statusBadge(state = { "busy" }, text = { row.commandGateLabel })
                }
            },
            tone = HeaderTone.PRIMARY,
            bottomGap = AdminSpace.xs,
        )
        if (row.commandGate == "compatible") {
            // 兼容标记：对勾走矢量图标，文案里不再塞「✓」字符
            View {
                attr { flexDirectionRow(); alignItemsCenter() }
                appIcon({ IconToken.Check }, size = 12f, tint = { AdminColors.textSecondary })
                hGap(4f)
                captionLine("${row.commandGateLabel}（说「退下」会保持连接）")
            }
        }
        captionLine(row.subtitle)
        Text {
            attr {
                fontSize(AdminType.caption)
                color(AdminColors.textTertiary)
                text(row.toolSummary)
            }
        }
        if (row.error.isNotEmpty()) {
            textNote(
                "最近一次探测未完成：${row.error}" +
                    if (row.tools.isEmpty()) "" else "（下面显示的是上次成功探测的结果）",
                tone = "warn",
                micro = true,
            )
        }
        if (row.tools.isEmpty()) {
            textNote(
                "这台设备没有上报任何工具：可能是探测失败（见上），或固件未注册 MCP 工具" +
                    "（板级 `InitializeTools()` 里没有 `AddTool`）。",
                micro = true,
            )
        }
        // 快照列表（不可变）用普通 for：vfor 是给可观察列表用的，这里重建整块即可
        for (tool in row.tools) {
            toolBlock(tool)
        }
    }
}

/** 一个工具：name（+ 仅管理员标记）/ 参数摘要 / description 原文。 */
private fun ViewContainer<*, *>.toolBlock(tool: McpToolRow) {
    View {
        attr {
            flexDirectionColumn()
            paddingTop(AdminSpace.sm)
        }
        View {
            attr {
                flexDirectionRow()
                alignItemsCenter()
            }
            Text {
                attr {
                    flex(1f)
                    fontSize(AdminType.body)
                    color(AdminColors.textPrimary)
                    text(tool.name)
                }
            }
            if (tool.userOnly) {
                // 固件 `AddUserOnlyTool`：默认不出现在 tools/list、大模型看不到（服务器按名字调用）。
                // 用 warn 色而不是中性色——它正是"为什么 AI 不会用这个工具"的答案。
                statusBadge(state = { "busy" }, text = { "仅管理员" })
            }
        }
        Text {
            attr {
                fontSize(AdminType.micro)
                color(AdminColors.textTertiary)
                text(tool.signature)
            }
        }
        if (tool.description.isNotEmpty()) {
            captionLine(tool.description)
        }
        dividerH()
    }
}
