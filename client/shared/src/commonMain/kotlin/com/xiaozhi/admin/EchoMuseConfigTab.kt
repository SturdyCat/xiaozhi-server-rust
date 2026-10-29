package com.xiaozhi.admin

import androidx.compose.runtime.Composable

/**
 * 配置 tab 的**组合根**：网络连接 → Stage 01–08 → 底部「下发配置」。
 *
 * 本文件只有 8 行调用，刻意不承载任何控件：
 *
 * | 段 | 实现文件 |
 * |---|---|
 * | 网络连接 + 下发配置 | `EchoMuseConfigConnectivity.kt` |
 * | Stage 01–08 | `EchoMuseConfigStages.kt`（Stage 08 的内容在 `EchoMuseMusicPanel.kt`） |
 * | Stage 06 的 GATT 工具 | `EchoMuseBleCard.kt` |
 *
 * ## 单一写路径（与旧实现同一约束）
 *
 * Stage 01–07 的每个开关/输入框都只是改**同一份草稿**（[EchoMuseConfigDrafts]）；
 * 整个 tab 只有两个写动作：
 * 1. 「下发配置」→ `POST …/settings`（部分更新，数值留空 = 不发该键）；
 * 2. WiFi 那三块 → `command`（`wifi` / `wifi_scan` / `wifi_commit`），它们的语义是**指令**，
 *    不是配置键，所以不走 settings。
 *
 * 也**不采用**"每个分区各自一份草稿、各自提交"的模式（旧 `compose/AdminConfigDraft.kt` 已随
 * Compose 壳统一而删除，此处仅记录设计取舍）：
 * 设备 settings 是部分更新语义，一份草稿 + 一次提交才能保证"用户看到的整页就是发出去的内容"。
 */
@Composable
internal fun EchoMuseConfigTab(
    row: EchoMuseDeviceRow,
    state: EchoMuseState,
    toast: ToastState,
    onOpenModelFiles: () -> Unit = {},
) {
    EchoMuseConnectivity(row = row, state = state, toast = toast)

    StagePlayback(state = state)
    StageWakeWord(state = state, onOpenModelFiles = onOpenModelFiles)
    StageMicrophones(state = state)
    StageRing(state = state)
    StageAdvanced(state = state)
    StageBluetooth(row = row, state = state)
    StageSendspin(state = state)
    StageMusic(row = row, state = state, toast = toast)

    EchoMuseConfigSubmit(row = row, state = state, toast = toast)
}
