package com.xiaozhi.admin

import com.tencent.kuikly.core.base.ViewContainer
import com.tencent.kuikly.core.directives.vfor
import com.tencent.kuikly.core.directives.vif
import com.tencent.kuikly.core.pager.Pager
import com.tencent.kuikly.core.views.Text
import com.tencent.kuikly.core.views.View

/**
 * 引擎健康自检的两个渲染件（P1-6，全部复用 AdminTheme 组件层，不自造样式）：
 *
 * - [healthBanner]：**就绪度横幅**（概览页最顶部）。仅当存在偏离行时渲染，
 *   每行「{能力} · {降级|异常}」可点击 → 跳配置页对应 section；正常时不占任何视觉
 *   （延续全站"只在偏离时打标"铁律）。
 * - [engineHealthCard]：**能力健康卡**（配置页「总览」section、既有 `overviewCard` 之下）。
 *   11 行 × phase 一览：偏离行打 [statusBadge] + 原因展开 + 可点击跳 section；
 *   正常行弱化（textTertiary 一行带过）；行尾显示当前实现 id（数据同源 `/api/plugins`）。
 *
 * ⚠️ 渲染函数必须是 `ViewContainer` 扩展，并在目标容器闭包内**非限定**调用
 * （见 `ConfigFormState` 文件头的 Kuikly DSL 绑定说明）。
 */

/**
 * 就绪度横幅：仅当存在偏离（degraded/failed）行时渲染。
 *
 * 点击某行 → [onJumpSection]（配置页 section id；无对应 section 的行不可点）。
 * 加载失败不渲染横幅本体（健康卡内有就地错误 + 刷新），但横幅位置不留给空白。
 */
fun ViewContainer<*, *>.healthBanner(health: EngineHealthState, onJumpSection: (section: String) -> Unit) {
    vif({ health.loaded && health.deviations.size > 0 }) {
        groupedCard("引擎健康（${health.deviations.size} 项偏离）") {
            vfor({ health.deviations }) { row ->
                // 单一根容器包裹：vfor 项闭包只允许生成一个孩子（2.29.1 起严格校验，
                // healthBannerRow 在 reason 非空时会生成「行 + 原因」两个根节点，直接抛
                // "vfor creator闭包内必须需要且仅一个孩子节点"→ 整页白屏）
                View {
                    attr { flexDirectionColumn() }
                    healthBannerRow(row) { section -> onJumpSection(section) }
                }
            }
            textNote("点击条目可跳到对应配置项修复；探测为本地文件系统检查，服务端每次请求实时执行。", micro = true)
        }
    }
}

/** 横幅里的一行：能力名 + 徽标 + 原因摘要；可跳转的行整体可点。 */
private fun ViewContainer<*, *>.healthBannerRow(row: HealthRow, onJump: (section: String) -> Unit) {
    View {
        attr {
            flexDirectionRow()
            alignItemsCenter()
            paddingTop(AdminSpace.xs)
            paddingBottom(AdminSpace.xs)
        }
        // 可跳转的行给可点击语义（voice 这类无 section 的偏离行只展示）
        if (row.jumpable) {
            event { click { onJump(row.configSection) } }
        }
        Text {
            attr {
                fontSize(AdminType.body)
                fontWeightMedium()
                color(if (row.phase == "failed") AdminColors.dangerTintText else AdminColors.warningTintText)
                text(row.display)
            }
        }
        View { attr { width(AdminSpace.sm) } }
        statusBadge(
            state = { if (row.phase == "failed") "error" else "busy" },
            text = { if (row.phase == "failed") "异常" else "降级" },
        )
        View { attr { flex(1f) } }
        if (row.jumpable) {
            Text {
                attr {
                    fontSize(AdminType.micro)
                    color(AdminColors.accent)
                    text("去修复 ›")
                }
            }
        }
    }
    // 原因摘要（含「怎么修」）：横幅里也直接可见，不用点进健康卡
    if (row.reason.isNotEmpty()) {
        View {
            attr { paddingBottom(AdminSpace.xs) }
            Text {
                attr {
                    fontSize(AdminType.micro)
                    color(AdminColors.textTertiary)
                    text(row.reason)
                }
            }
        }
    }
}

/**
 * 能力健康卡：11 个 capability × phase 一览（配置页「总览」section）。
 *
 * 偏离行打徽标 + 原因展开 + 可点击跳 section；正常行弱化一行带过；
 * 行尾显示当前实现 id（呼应"capability 行可见其实现明细，数据同源"）。
 */
fun ViewContainer<*, *>.engineHealthCard(health: EngineHealthState, ctx: Pager, onJumpSection: (section: String) -> Unit) {
    groupedCard("能力健康") {
        // 摘要行：左侧计数、右侧刷新（与 overviewCard 同款头行）
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
                            health.loadError.isNotEmpty() -> "无法读取健康自检"
                            health.summary.isEmpty() -> if (health.loading) "检测中…" else "—"
                            else -> health.summary
                        },
                    )
                }
            }
            secondaryButton(
                "重新检测",
                loading = { health.loading },
                loadingText = "检测中…",
            ) {
                health.refresh(ctx)
            }
        }
        // 首次加载失败：就地错误 + 可行动指引
        vif({ health.loadError.isNotEmpty() }) {
            Text {
                attr {
                    fontSize(AdminType.caption)
                    color(AdminColors.dangerTintText)
                    marginTop(AdminSpace.xs)
                    text(health.loadError + "（确认服务端地址可达后点「重新检测」）")
                }
            }
        }
        vif({ health.loaded && health.rows.size == 0 }) {
            Text {
                attr {
                    fontSize(AdminType.caption)
                    color(AdminColors.textTertiary)
                    marginTop(AdminSpace.xs)
                    text("服务端未返回任何能力（注册表为空？请查看服务端日志）。")
                }
            }
        }
        vfor({ health.rows }) { row ->
            // 单一根容器包裹：vfor 项闭包只允许生成一个孩子（healthCardRow 内部含分隔线，
            // 2.29.1 起严格校验会直接抛异常），行内布局（含行尾分隔线）保持不变
            View {
                healthCardRow(row, onJumpSection)
            }
        }
        textNote("健康 = 配置校验（plan）与本地探测（模型文件 / 记忆目录可写）的合并结论；与「能力总览」同源。", micro = true)
    }
}

/** 健康卡里的一行：偏离行展开（徽标 + 原因 + 跳转），正常行弱化一行。 */
private fun ViewContainer<*, *>.healthCardRow(row: HealthRow, onJump: (section: String) -> Unit) {
    if (row.isDeviation) {
        // 偏离行：能力名 + 徽标 + 当前实现 + 原因 + 可跳转
        View {
            attr {
                flexDirectionColumn()
                paddingTop(AdminSpace.sm)
                paddingBottom(AdminSpace.xs)
            }
            View {
                attr {
                    flexDirectionRow()
                    alignItemsCenter()
                }
                if (row.jumpable) {
                    event { click { onJump(row.configSection) } }
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
                if (row.activeImpl.isNotEmpty()) {
                    Text {
                        attr {
                            fontSize(AdminType.micro)
                            color(AdminColors.textTertiary)
                            marginRight(AdminSpace.sm)
                            text(row.activeImpl)
                        }
                    }
                }
                statusBadge(
                    state = { if (row.phase == "failed") "error" else "busy" },
                    text = { if (row.phase == "failed") "异常" else "降级" },
                )
            }
            // 原因必须显示（怎么修就在 last_error 里）
            if (row.reason.isNotEmpty()) {
                Text {
                    attr {
                        fontSize(AdminType.micro)
                        color(if (row.phase == "failed") AdminColors.dangerTintText else AdminColors.warningTintText)
                        marginTop(AdminSpace.xs)
                        text(row.reason)
                    }
                }
            }
            if (row.jumpable) {
                Text {
                    attr {
                        fontSize(AdminType.micro)
                        color(AdminColors.accent)
                        marginTop(AdminSpace.xs)
                        text("点击跳到「${sectionLabel(row.configSection)}」配置项 ›")
                    }
                }
            }
        }
        dividerH()
    } else {
        // 正常/关闭行：弱化一行（名称 + 状态词 + 实现尾注），不打徽标不展开
        View {
            attr {
                flexDirectionRow()
                alignItemsCenter()
                paddingTop(AdminSpace.xs)
                paddingBottom(AdminSpace.xs)
            }
            Text {
                attr {
                    flex(1f)
                    fontSize(AdminType.body)
                    color(AdminColors.textTertiary)
                    text(row.display)
                }
            }
            Text {
                attr {
                    fontSize(AdminType.micro)
                    color(AdminColors.textTertiary)
                    marginRight(AdminSpace.sm)
                    text(row.activeImpl)
                }
            }
            Text {
                attr {
                    fontSize(AdminType.micro)
                    color(AdminColors.textTertiary)
                    text(
                        when (row.phase) {
                            "active" -> "正常"
                            "disabled" -> "已关闭"
                            else -> row.phase
                        },
                    )
                }
            }
        }
        dividerH()
    }
}

/** section id → 管理台导航标签（跳转提示文案用；ConfigSections 是唯一权威）。 */
private fun sectionLabel(sectionId: String): String =
    ConfigSections.all.firstOrNull { it.id == sectionId }?.label ?: sectionId
