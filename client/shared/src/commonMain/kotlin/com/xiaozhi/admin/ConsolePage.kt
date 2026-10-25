package com.xiaozhi.admin

import com.xiaozhi.admin.ui.*

import com.tencent.kuikly.core.base.ViewContainer
import com.tencent.kuikly.core.directives.vif
import com.tencent.kuikly.core.pager.Pager
import com.tencent.kuikly.core.views.Text

/**
 * 管理台「控制台」页（官方 dashboard 的 CONSOLE / 命令行）：实时显示服务端全局日志流
 * （`GET /api/logs`）。含连接状态、级别筛选、暂停冻结、复制、清空。数据源 [ConsoleState]。
 *
 * 与官方「命令行」语义一致：这是只读的实时日志终端（服务端只推日志、不接收命令），
 * 对应我们此前**已有服务端实现但客户端从未建页面消费**的 `/api/logs` 流。
 *
 * ⚠️ 渲染函数必须是 `ViewContainer` 扩展，并在目标容器闭包内**非限定**调用。
 */
fun ViewContainer<*, *>.renderConsole(
    console: ConsoleState,
    toast: ToastState,
    ctx: Pager,
) {
    headerRow(
        title = "控制台",
        tone = HeaderTone.BODY,
        meta = {
            Text {
                attr {
                    fontSize(AdminType.caption)
                    color(if (console.connected) AdminColors.accentTintText else AdminColors.textSecondary)
                    text(console.status)
                }
            }
        },
        actions = {
            secondaryButton(
                "重连",
                enabled = { !console.connected },
                icon = IconToken.Refresh,
            ) { console.connect(console.apiBase, console.apiToken) }
        },
    )

    textNote(
        text = {
            "来源为服务端全局日志流（GET /api/logs，INFO 及以上）。连接后先补最近 500 行历史再实时追加；" +
                "级别筛选与暂停在客户端生效，不影响服务端。"
        },
        micro = true,
    )

    groupedCard("控制台") {
        actionRow {
            dropdownField(
                "级别",
                { console.levelOptions.firstOrNull { it.first == console.levelFilter }?.second ?: "全部级别" },
                { console.levelOptions },
                { console.levelFilter },
                { console.levelOpen },
                { console.levelOpen = !console.levelOpen },
                { console.setLevel(it) },
            )
            hGap(AdminSpace.sm)
            secondaryButton(
                if (console.paused) "继续" else "暂停",
                enabled = { console.connected },
            ) { console.togglePause() }
            hGap(AdminSpace.sm)
            secondaryButton(
                "复制",
                enabled = { console.display.isNotEmpty() },
                icon = IconToken.Copy,
            ) {
                if (copyToClipboard(console.display)) toast.show("日志已复制到剪贴板", "ok")
                else toast.show("复制失败：当前平台剪贴板不可用", "error")
            }
            hGap(AdminSpace.sm)
            secondaryButton(
                "清空",
                enabled = { console.display.isNotEmpty() },
            ) { console.clear() }
        }

        vif({ console.paused }) {
            textNote("已暂停：画面冻结，新日志暂存，点「继续」后一次性补上。", micro = true)
        }

        vGap(AdminSpace.sm)
        outputBlock(
            content = {
                console.display.ifEmpty {
                    "（等待日志…连接正常后这里会实时滚动服务器运行日志；若长时间为空请点「重连」）"
                }
            },
            height = 420f,
        )
    }
}
