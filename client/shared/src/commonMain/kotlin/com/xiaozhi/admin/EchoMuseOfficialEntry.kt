package com.xiaozhi.admin

import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import com.gearui.components.button.Button
import com.gearui.components.button.ButtonTheme
import com.gearui.components.button.ButtonType
import com.gearui.foundation.primitives.Text
import com.gearui.theme.Theme
import com.tencent.kuikly.compose.foundation.layout.Arrangement
import com.tencent.kuikly.compose.foundation.layout.Column
import com.tencent.kuikly.compose.foundation.layout.fillMaxWidth
import com.tencent.kuikly.compose.foundation.layout.padding
import com.tencent.kuikly.compose.ui.Modifier
import com.xiaozhi.admin.kit.GearUiSpace

/**
 * 「官方设备管理页」入口卡（web-only）。
 *
 * ## 为什么是"另开一个页面"而不是在本页复刻
 *
 * 设备管理的六 tab 是按 EchoMuse 官方 `dashboard.jsx` **原样 vendor** 的 React 页面
 * （`client/apps/h5App/em/`，由 Rust 静态目录同源托管在 `/em/`）。它是**另一个文档**，
 * Kuikly 的路由只能打开 `@Page`，所以这里只提供入口；页面内的数据由 `em/static/adapter.js`
 * 映射到本服务端 `/api/echomuse/…`。
 *
 * ## 与下方旧页面的关系
 *
 * 卡片下面的 [EchoMusePage] 原有六 tab **整体保留**：它承载本项目与官方差异较大的专有
 * 能力（DLNA 音乐、BLE GATT 面板、唤醒词资产 VAD 安装、WiFi 三步闭环）。官方页负责
 * "和官方一致"的那部分，两者数据同源、并存不冲突。
 *
 * ## 零状态
 *
 * 本卡片只读页面已有字段（[AdminPage.apiBase] 与 `Pager.adminRequestToken()`），不新增
 * 任何响应式状态——`verifyComposeStateMigration` 的计数因此保持不变。
 */
@Composable
fun EmOfficialEntryCard(page: AdminPage) {
    val base = page.apiBase
    val token = page.adminRequestToken()
    val url = remember(base, token) {
        val query = if (token.isEmpty()) "" else "?token=" + echoMuseUrlComponent(token)
        "$base/em/$query"
    }

    LabeledCard(title = "官方设备管理（网页版）") {
        Column(
            modifier = Modifier.fillMaxWidth(),
            verticalArrangement = Arrangement.spacedBy(GearUiSpace.sm),
        ) {
            Text(
                text = "按 EchoMuse 官方 dashboard 源码移植的设备卡列表与六 tab 详情" +
                    "（状态 / 活动 / 配置 / 控制台 / 更新 / 日志）。" +
                    "官方界面里本服务端没有的数据项显示「—」。",
                style = Theme.typography.bodyMedium,
                color = Theme.colors.mutedForeground,
            )
            Button(
                text = "打开官方设备管理页",
                onClick = {
                    if (!openAdminUrl(url, true)) {
                        page.toast.show(
                            "浏览器拦截了新标签页：请直接访问 " + (if (base.isEmpty()) "" else base) + "/em/" +
                                (if (token.isEmpty()) "" else "?token=…"),
                            "warn",
                        )
                    }
                },
                type = ButtonType.FILL,
                theme = ButtonTheme.PRIMARY,
                modifier = Modifier.padding(top = GearUiSpace.xs),
            )
            Text(
                text = "本项目专有能力（DLNA 音乐、BLE GATT、唤醒词资产安装、WiFi 三步闭环）" +
                    "保留在下方设备卡中。",
                style = Theme.typography.bodySmall,
                color = Theme.colors.mutedForeground,
            )
        }
    }
}
