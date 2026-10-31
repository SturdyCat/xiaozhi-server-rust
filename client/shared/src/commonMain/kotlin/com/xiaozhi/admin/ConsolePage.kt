package com.xiaozhi.admin

import androidx.compose.runtime.Composable
import com.gearui.foundation.primitives.ScrollView
import com.gearui.foundation.primitives.Text
import com.gearui.theme.Theme
import com.tencent.kuikly.compose.foundation.layout.Column
import com.tencent.kuikly.compose.foundation.layout.Row
import com.tencent.kuikly.compose.foundation.layout.fillMaxSize
import com.tencent.kuikly.compose.foundation.layout.fillMaxWidth
import com.tencent.kuikly.compose.foundation.layout.height
import com.tencent.kuikly.compose.foundation.layout.padding
import com.tencent.kuikly.compose.ui.Alignment
import com.tencent.kuikly.compose.ui.Modifier
import com.tencent.kuikly.compose.ui.unit.dp
import com.xiaozhi.admin.kit.GearTerminal
import com.xiaozhi.admin.kit.GearTerminalText
import com.xiaozhi.admin.kit.GearTerminalTone
import com.xiaozhi.admin.kit.LocalAdminSpacing
import com.xiaozhi.admin.kit.actionRow
import com.xiaozhi.admin.kit.labeledSelect
import com.xiaozhi.admin.kit.pageScroller

/** 终端可视高度（迁移前 `outputBlock(height = 420f)` 的原值，不放大也不缩小）。 */
private val CONSOLE_TERMINAL_HEIGHT = 420.dp

/**
 * 控制台页（`console`）—— Compose 版：服务端全局实时日志流（`GET /api/logs`）。
 *
 * 与官方 dashboard 的 CONSOLE / 命令行语义一致：**只读**终端（服务端只推日志、不接收命令），
 * 对应此前"服务端已有实现、客户端从未建页面消费"的那条流。
 *
 * ## ⚠️ K2 保留项：终端块必须保持深色
 *
 * 正文用 [GearTerminal] / [GearTerminalText]（`kit/GearTerminal.kt`）：底色 `#1C1F18`、
 * 前景 `#C8D4B0`、等宽 13sp。**不许主题化、不许浅色化** —— 浅色底上 ANSI 日志不可读
 * （级别词与目标名在灰度上几乎相等），而且用户在浅色管理台里点进"控制台"时的预期就是深色终端。
 * 色值收敛在 kit 里，本页**不出现任何色值字面量**。
 *
 * ## 截断策略（双上限，语义未改）
 *
 * 可见缓冲上限 2000 行（`CONSOLE_MAX_LINES`，超出丢最旧）；暂停期间另有一份"暂停缓冲"，
 * 上限 2000×2 行，继续后一次性并入。两个上限都在 [ConsoleState] 里，页面只负责显示
 * `console.display`（已按级别筛过、按上限截断）。
 *
 * ## 级别筛选与暂停都在客户端生效
 *
 * 不重连、不影响服务端：暂停只冻结画面（连接保持），继续时补上暂停期间的行。
 */
@Composable
fun ConsolePage(console: ConsoleState, toast: ToastState) {
    pageScroller {
        // 头行：标题 + 连接状态 + 「重连」（已连接时禁点，避免不必要的重连抖动）
        Row(
            modifier = Modifier.fillMaxWidth(),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Column(modifier = Modifier.weight(1f)) {
                Text(
                    text = "控制台",
                    style = Theme.typography.titleSmall,
                    color = Theme.colors.foreground,
                )
                Text(
                    text = console.status,
                    style = Theme.typography.caption,
                    color = if (console.connected) Theme.colors.success else Theme.colors.mutedForeground,
                )
            }
            OpsOutlineButton(
                text = "重连",
                onClick = { console.connect(console.apiBase, console.apiToken) },
                enabled = !console.connected,
            )
        }
        HintLine(
            "来源为服务端全局日志流（GET /api/logs，INFO 及以上）。连接后先补最近 500 行历史再实时追加；" +
                "级别筛选与暂停在客户端生效，不影响服务端。",
        )

        LabeledCard(title = "控制台") {
            labeledSelect(
                label = "级别",
                selectedId = console.levelFilter,
                options = console.levelOptions,
                onSelect = { console.setLevel(it) },
            )
            actionRow {
                OpsOutlineButton(
                    text = if (console.paused) "继续" else "暂停",
                    onClick = { console.togglePause() },
                    enabled = console.connected,
                )
                OpsOutlineButton(
                    text = "复制",
                    onClick = {
                        if (copyToClipboard(console.display)) {
                            toast.show("日志已复制到剪贴板", "ok")
                        } else {
                            toast.show("复制失败：当前平台剪贴板不可用", "error")
                        }
                    },
                    enabled = console.display.isNotEmpty(),
                )
                OpsOutlineButton(
                    text = "清空",
                    onClick = { console.clear() },
                    enabled = console.display.isNotEmpty(),
                )
            }
            if (console.paused) {
                HintLine("已暂停：画面冻结，新日志暂存，点「继续」后一次性补上。")
            }
            // ⚠️ 深色终端块（K2）：内容区自己滚动，整块高度固定 420dp
            GearTerminal(
                modifier = Modifier
                    .fillMaxWidth()
                    .height(CONSOLE_TERMINAL_HEIGHT)
                    .padding(top = LocalAdminSpacing.current.sm),
            ) {
                ScrollView(modifier = Modifier.fillMaxSize()) {
                    GearTerminalText(
                        text = console.display.ifEmpty {
                            "（等待日志…连接正常后这里会实时滚动服务器运行日志；若长时间为空请点「重连」）"
                        },
                        tone = if (console.display.isEmpty()) GearTerminalTone.Dim else GearTerminalTone.Fg,
                    )
                }
            }
            HintLine("只有等宽字体才能让日志列对齐，故终端块固定 13sp Monospace；行上限 2000（超出丢最旧）。")
        }
    }
}
