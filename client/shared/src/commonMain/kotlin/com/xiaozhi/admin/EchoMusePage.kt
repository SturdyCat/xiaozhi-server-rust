package com.xiaozhi.admin

import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import com.gearui.components.alert.Alert
import com.gearui.components.alert.AlertStatus
import com.gearui.components.button.Button
import com.gearui.components.button.ButtonType
import com.gearui.components.empty.EmptyState
import com.gearui.foundation.primitives.ScrollView
import com.gearui.foundation.primitives.Text
import com.gearui.theme.Theme
import com.tencent.kuikly.compose.foundation.layout.Arrangement
import com.tencent.kuikly.compose.foundation.layout.Column
import com.tencent.kuikly.compose.foundation.layout.Row
import com.tencent.kuikly.compose.foundation.layout.fillMaxSize
import com.tencent.kuikly.compose.foundation.layout.fillMaxWidth
import com.tencent.kuikly.compose.foundation.layout.padding
import com.tencent.kuikly.compose.ui.Alignment
import com.tencent.kuikly.compose.ui.Modifier
import com.xiaozhi.admin.kit.GearUiSpace

/**
 * 「EchoMuse 设备」页（Compose 版，T10 重写）。
 *
 * ## 职责边界（一个函数只做一件事）
 *
 * ```
 * EchoMusePage              本文件：摘要头 + 刷新 + 错误/加载/未启用/空列表 + 设备卡列表
 *   └─ EchoMuseDeviceCard   EchoMuseDeviceCard.kt：卡片壳 + 身份行 + 删除（红 + 二次确认）
 *       └─ EchoMuseDetailTabs  EchoMuseDetailTabs.kt：六 tab 容器 + 审批（改名 sheet）+ 跨 tab 反馈
 *           ├─ EchoMuseStatusTab     状态
 *           ├─ EchoMuseActivityTab   活动
 *           ├─ EchoMuseConfigTab     配置（+ 8 个 Stage）
 *           ├─ EchoMuseConsoleTab    控制台（常暗，K2）
 *           ├─ EchoMuseUpdatesTab    更新
 *           └─ EchoMuseLogsTab       日志
 * ```
 *
 * ## 呈现规则（与运维页同一口径，PRD 的「诚实展示」）
 *
 * - `enabled = false` 优先于一切：明确说"服务端没开这个能力、怎么开"，**不渲染空列表**
 *   ——空列表会被读成"我还没有设备"，而真相是"这个能力根本没打开"；
 * - 只在偏离时打标：在线不打标（[com.xiaozhi.admin.kit.statusBadge] 的四态语义里
 *   在线是正常态），离线 / 未批准才占视觉；
 * - 列表是请求时刻的快照，停留本页期间靠管理台 WS（`/api/admin/ws` 的
 *   `echomuse_devices_changed`）推送刷新，[isActive] 为假时退订；
 * - 指令是**异步下发**：接口返回成功只代表服务端已排队，执行结果以设备灯环为准——
 *   这条口径写在页脚，不藏在文档里。
 *
 * ## 为什么实时订阅放在 `LaunchedEffect(active)` 而不是裸调用
 *
 * 旧 DSL 的 `renderEchoMuse` 每次渲染都调 `ensureLiveUpdates`（它内部按"驻留态变化沿"
 * 幂等）。Compose 里在组合期直接调状态方法会产生副作用，故改成"按驻留态键控的效应"：
 * 进出本页各触发一次，语义与旧实现等价，且不随任意重组反复进出。
 *
 * ⚠️ 运行时可验证项（六 tab 切换、删除确认、深色终端、BLE 连接态）本环境无法执行，
 * 见交付说明的「需本地验证」清单。
 */
@Composable
fun EchoMusePage(
    state: EchoMuseState,
    toast: ToastState,
    isActive: () -> Boolean,
    /** 唤醒词面板「管理模型文件（oww-models/）」入口（跳「文件」页对应目录，AdminPage 接线）。 */
    onOpenModelFiles: () -> Unit = {},
) {
    val pager = LocalAdminPagerOrThrow()
    val active = isActive()
    LaunchedEffect(active) { state.ensureLiveUpdates(pager) { active } }

    Column(modifier = Modifier.fillMaxSize()) {
        EmPageHeader(state = state, onRefresh = { state.refresh(pager) })
        ScrollView(modifier = Modifier.weight(1f).fillMaxWidth()) {
            Column(
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(horizontal = GearUiSpace.pagePadding, vertical = GearUiSpace.md),
            ) {
                EmPageBody(
                    state = state,
                    toast = toast,
                    onRefresh = { state.refresh(pager) },
                    onOpenModelFiles = onOpenModelFiles,
                )
            }
        }
    }
}

/**
 * 页头：摘要 + 刷新。
 *
 * 摘要文案由 [EchoMuseState.summaryText] 给（`共 N 台 · M 台在线` / `EchoMuse 未启用`），
 * 不在渲染层拼——它同时被页脚与调试提示复用，两处各拼一次必然漂移。
 */
@Composable
private fun EmPageHeader(state: EchoMuseState, onRefresh: () -> Unit) {
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .padding(start = GearUiSpace.pagePadding, end = GearUiSpace.pagePadding, top = GearUiSpace.md),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(GearUiSpace.sm),
    ) {
        Text(
            text = if (state.loading && !state.loaded) "加载中…" else state.summaryText(),
            modifier = Modifier.weight(1f),
            style = Theme.typography.titleMedium,
            color = Theme.colors.foreground,
        )
        Button(
            text = "刷新",
            onClick = onRefresh,
            type = ButtonType.OUTLINE,
            loading = state.loading,
            disabled = state.loading,
        )
    }
}

/**
 * 页体：错误 → 加载 → 未启用 → 空列表 → 设备卡列表。
 *
 * 顺序是有意的：错误横幅在"未启用"之上（读列表失败时不能只显示"未启用"，
 * 那是把网络问题说成配置问题）。
 */
@Composable
private fun EmPageBody(
    state: EchoMuseState,
    toast: ToastState,
    onRefresh: () -> Unit,
    onOpenModelFiles: () -> Unit,
) {
    if (state.loadError.isNotEmpty()) {
        Alert(
            title = "设备列表加载失败",
            description = "${state.loadError}（确认管理令牌与服务端地址后点「刷新」）",
            status = AlertStatus.DANGER,
            modifier = Modifier.fillMaxWidth().padding(bottom = GearUiSpace.sm),
        )
    }
    if (state.loading && !state.loaded) {
        EmNote("正在读取 EchoMuse 设备列表…")
    }

    // 服务端未开启 [echomuse]：给出可行动的开启指引，不渲染空列表
    if (state.loaded && !state.enabled) {
        LabeledCard(title = "EchoMuse 未启用") {
            EmNote(
                "服务端未开启 EchoMuse 能力。请到「配置 → EchoMuse」分区打开「启用 EchoMuse 设备入口」" +
                    "并重启服务端；之后 EchoMuse 设备会在连上服务端时登记到这里，经批准后即可远程管理" +
                    "（重启、下发 WiFi 配置）。",
            )
        }
        return
    }
    if (!state.enabled) return

    if (state.loaded && state.devices.isEmpty()) {
        EmptyState(
            message = "还没有 EchoMuse 设备登记",
            description = "设备首次连上服务端后会出现在这里（未批准状态），批准后才能远程下发指令。",
            actionText = "刷新",
            onAction = onRefresh,
            modifier = Modifier.fillMaxWidth(),
        )
    }

    for (row in state.devices) {
        EchoMuseDeviceCard(
            row = row,
            state = state,
            toast = toast,
            onOpenModelFiles = onOpenModelFiles,
        )
    }

    EmNote("指令为异步下发：接口返回成功只代表服务端已把指令排队送给设备，执行结果以设备灯环为准。")
}
