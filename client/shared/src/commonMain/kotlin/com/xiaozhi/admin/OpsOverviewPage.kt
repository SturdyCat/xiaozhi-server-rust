package com.xiaozhi.admin

import androidx.compose.runtime.Composable
import com.gearui.components.grid.Grid
import com.xiaozhi.admin.kit.GearUiSpace
import com.xiaozhi.admin.kit.LocalAdminViewport

/**
 * 概览页（`overview`）—— Compose 版。
 *
 * ## 这一页回答什么
 *
 * "服务现在什么状态"：几台设备在线、有几条活会话、刚才有没有人在说话、上游是不是在报错。
 * 这是本项目此前**完全没有**的一页（旧管理页打开就是一堆配置字段）；参考项目
 * `manager-web` 的 `home.vue` 就是这个位置。
 *
 * ## 三层信息，从上到下
 *
 * 1. **就绪度横幅**（[HealthBanner]，P1-6）：仅偏离时渲染，可点击跳到对应配置项；
 * 2. **实时磁贴**（`Grid` + `Card`）：把 `/api/stats` 的计数摊成 5 块，数值右对齐；
 * 3. **能力总览**（[CapabilityOverviewCard]）：**直接复用「配置 → 总览」那张卡**
 *    （数据源 `/api/plugins`），不在本页第二次投影 —— 同一份事实投影两次，只会让
 *    "能力总览"在两处长成不一样。
 *
 * ## 口径必须写在页面上
 *
 * 这些数字全部是**内存态**（`crate::app::registry`）：自本次服务启动以来累计，重启清零。
 * 服务端随响应下发 `scope_note` / `note`，页面原样展示 —— 不写清楚的话
 * "在线 0 台"会被误读成"设备都掉线了"。
 */
@Composable
fun OpsOverviewPage(
    stats: StatsState,
    meta: PluginMetaState,
    health: EngineHealthState,
    onJumpSection: (String) -> Unit,
) {
    val ctx = LocalAdminPagerOrThrow()
    val viewport = LocalAdminViewport.current
    // 磁贴列数随视口：窄屏 2 列、宽屏 3 列（与断点同源，数值一字不改）
    val columns = if (viewport.isNarrow) 2 else 3

    OpsPageFrame(
        summary = stats.summary,
        loading = stats.loading,
        loaded = stats.loaded,
        loadError = stats.loadError,
        onRefresh = { stats.refresh(ctx) },
        header = { HealthBanner(health, onJumpSection) },
    ) {
        LabeledCard(title = "实时") {
            Grid(
                columns = columns,
                horizontalSpacing = GearUiSpace.sm,
                verticalSpacing = GearUiSpace.sm,
            ) {
                item {
                    OpsTile(
                        label = "在线设备",
                        value = "${stats.devicesOnline} / ${stats.devicesTotal}",
                        note = "右为见过的设备总数（自本次启动）",
                    )
                }
                item {
                    OpsTile(
                        label = "活会话",
                        value = "${stats.sessionsLive}",
                        note = if (stats.sessionsTest > 0) {
                            "含 ${stats.sessionsTest} 条测试台连接"
                        } else {
                            "设备每次唤醒通常新建一条连接"
                        },
                    )
                }
                item {
                    OpsTile(
                        label = "缓冲内轮次",
                        value = "${stats.turnsUser + stats.turnsAssistant}",
                        note = "用户 ${stats.turnsUser} · 助手 ${stats.turnsAssistant}（受内存环形上界影响，是下限）",
                    )
                }
                item {
                    OpsTile(
                        label = "已计量轮次",
                        value = "${stats.usageTurns}",
                        note = "上游返回 usage 的轮次数（令牌计量口径）",
                    )
                }
                item {
                    OpsTile(
                        label = "进程运行",
                        value = stats.uptimeText.ifEmpty { "—" },
                        note = "注册表是内存态：重启后累计值清零",
                    )
                }
            }
            if (stats.usageSummary.isNotEmpty()) {
                HintLine("令牌：${stats.usageSummary}")
            }
        }

        // 能力状态：复用既有卡片（数据源 /api/plugins），不在本页第二次实现
        CapabilityOverviewCard(meta)

        if (stats.scopeNote.isNotEmpty()) {
            HintLine(stats.scopeNote)
        }
        if (stats.version.isNotEmpty()) {
            HintLine("服务端版本 ${stats.version}")
        }
    }
}
