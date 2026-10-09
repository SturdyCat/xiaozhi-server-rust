package com.xiaozhi.admin

import com.tencent.kuikly.core.base.ViewContainer
import com.tencent.kuikly.core.directives.vfor
import com.tencent.kuikly.core.directives.vif
import com.tencent.kuikly.core.pager.Pager
import com.tencent.kuikly.core.reactive.collection.ObservableList
import com.tencent.kuikly.core.views.Text
import com.tencent.kuikly.core.views.View

/**
 * 「能力总览」卡片（配置页第一个 tab）：`GET /api/plugins` 的可读投影。
 *
 * 呈现规则（`docs/plugin-architecture-unification.md` §5.3）：
 * - **只在偏离时打标**：`active` 行不打徽标（避免一片绿色噪音）；
 * - **降级/异常显示原因**（`last_error`），行内直接展开——这一条是刻意超过 DSH 的
 *   （DSH 的行只显示阶段，原因只在 Host 日志里）；
 * - 首次加载失败 → **就地错误 + 「重试」**，不弹 toast（toast 只用于用户动作的结果）；
 * - 计数摘要只显示非零项。
 *
 * ⚠️ 渲染函数必须是 `ViewContainer` 扩展，并在目标容器闭包内**非限定**调用（见
 * `ConfigFormState` 文件头的 Kuikly DSL 绑定说明）。
 */
fun ViewContainer<*, *>.overviewCard(meta: PluginMetaState, ctx: Pager) {
    groupedCard("能力总览") {
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
                            meta.loadError.isNotEmpty() -> "无法读取能力清单"
                            meta.summary.isEmpty() -> "加载中…"
                            else -> meta.summary
                        },
                    )
                }
            }
            secondaryButton(
                "刷新",
                loading = { meta.loading },
                loadingText = "刷新中…",
            ) {
                meta.refresh(ctx)
            }
        }
        // 首次加载失败：就地错误 + 可行动指引（服务端全部能力状态都查不到时最可能是没连上）
        vif({ meta.loadError.isNotEmpty() }) {
            Text {
                attr {
                    fontSize(AdminType.caption)
                    color(AdminColors.dangerTintText)
                    marginTop(AdminSpace.xs)
                    text(meta.loadError + "（确认服务端地址可达后点「刷新」）")
                }
            }
        }
        vif({ meta.loaded && meta.requiredRows.size == 0 && meta.optionalRows.size == 0 }) {
            Text {
                attr {
                    fontSize(AdminType.caption)
                    color(AdminColors.textTertiary)
                    marginTop(AdminSpace.xs)
                    text("服务端未返回任何能力（注册表为空？请查看服务端日志）。")
                }
            }
        }
        capabilityGroup("必需能力（缺失即启动失败）", meta.requiredRows)
        capabilityGroup("可选能力（失败只降级）", meta.optionalRows)
    }
}

/** 一个分组：标题 + 行列表（空分组整体不渲染）。 */
private fun ViewContainer<*, *>.capabilityGroup(
    title: String,
    rows: ObservableList<CapabilityRow>,
) {
    vif({ rows.size > 0 }) {
        Text {
            attr {
                fontSize(AdminType.micro)
                fontWeightMedium()
                color(AdminColors.textTertiary)
                marginTop(AdminSpace.md)
                marginBottom(AdminSpace.xs)
                text(title)
            }
        }
        vfor({ rows }) { row ->
            capabilityRow(row)
        }
    }
}

/**
 * 行：能力名 + 状态徽标（仅偏离时） / 实现名 + 生效语义 / 原因（降级、异常时）。
 * 行间用 1px 分隔线，与卡片内其它行保持一致。
 */
private fun ViewContainer<*, *>.capabilityRow(row: CapabilityRow) {
    View {
        attr {
            flexDirectionColumn()
        }
        View {
            attr {
                flexDirectionRow()
                alignItemsCenter()
                paddingTop(AdminSpace.sm)
                paddingBottom(AdminSpace.xs)
            }
            Text {
                attr {
                    flex(1f)
                    fontSize(AdminType.body)
                    fontWeightMedium()
                    color(AdminColors.textPrimary)
                    text(row.display)
                }
            }
            // 只在偏离时打标（正常=「运行中」不打徽标）
            pluginStatusBadge(row.phase, row.enabled)
        }
        Text {
            attr {
                fontSize(AdminType.caption)
                color(AdminColors.textSecondary)
                text(
                    if (row.implDisplay.isEmpty()) row.hotHint
                    else if (row.hotHint.isEmpty()) row.implDisplay
                    else "${row.implDisplay} · ${row.hotHint}"
                )
            }
        }
        // ⚠️ 降级/异常必须显示原因：用户看到「异常」却查不到原因，是最容易招致"这功能没用"的形态
        vif({ row.reason.isNotEmpty() }) {
            Text {
                attr {
                    fontSize(AdminType.micro)
                    color(if (row.phase == "failed") AdminColors.dangerTintText else AdminColors.warningTintText)
                    marginTop(AdminSpace.xs)
                    text(row.reason)
                }
            }
        }
        dividerH()
    }
}
