package com.xiaozhi.admin

import com.xiaozhi.admin.ui.*

import com.tencent.kuikly.core.base.ViewContainer
import com.tencent.kuikly.core.directives.vif
import com.tencent.kuikly.core.pager.Pager

/**
 * 设备卡的「设备管理」菜单面板：固定命令/固定日志源，不暴露 shell 或固件刷写功能。
 *
 * 布局走 [splitColumn]（上下）：
 *
 * ```
 * ┌──────────────────────────────────────────────────┐
 * │ 状态与音量 │ 系统信息 │ 日志 │ 维护                  │ ← tabBar（官方 Tabs 封装，无 PageList）
 * ├──────────────────────────────────────────────────┤ ← 1px 分隔线（splitColumn 自动插）
 * │ 当前分区内容（随 tab 用 vif 切换）                  │
 * │ …                                                │
 * ├──────────────────────────────────────────────────┤
 * │ 操作进行中 / 错误 / 结果（三个 tab 共用同一处反馈）    │
 * └──────────────────────────────────────────────────┘
 * ```
 *
 * 为什么拆 tab：日志是本面板里唯一「会反复盯着看」的东西，和音量/重启这类一次性操作混在
 * 一页里需要一路往下滚；单独一档后日志输出可以给到更高的可视高度（[LOG_OUTPUT_HEIGHT]）。
 *
 * ⚠️ 分区判断必须用 vif 读 observable（构建期 when 求值会切 tab 不刷新），
 * 且渲染函数在目标容器闭包内**非限定**调用。
 */

/** 日志输出块高度：比其它诊断块高，尽量一屏看完一段崩溃/断连现场。 */
private const val LOG_OUTPUT_HEIGHT = 320f

/** 其它诊断输出块高度（运行状态 / 系统信息）。 */
private const val DIAG_OUTPUT_HEIGHT = 220f

/** 面板 tab 文案（下标语义见 [EM_MANAGE_TAB_STATUS] 等常量）。 */
private val EM_MANAGE_TABS = listOf("状态与音量", "系统信息", "日志", "维护")

/** 设备卡的「设备管理」面板（`EchoMuseDeviceCard.kt` 按菜单项分发进来的右栏内容）。 */
fun ViewContainer<*, *>.echoMuseManagementPanel(
    row: EchoMuseDeviceRow,
    state: EchoMuseState,
    toast: ToastState,
    ctx: Pager,
) {
    val management = state.management
    splitColumn(trailingGap = AdminSpace.sm) {
        auto {
            tabBar(
                tabs = EM_MANAGE_TABS,
                selectedIndex = { management.tabIndex },
                onSelect = { management.tabIndex = it },
                // 104 宽留给「状态与音量」这类 5 字标签（tab 必须等宽，见 tabBar 文档）
                itemWidth = 104f,
            )
        }
        auto {
            vif({ management.tabIndex == EM_MANAGE_TAB_STATUS }) { manageStatusTab(management, ctx) }
            vif({ management.tabIndex == EM_MANAGE_TAB_SYSTEM }) { manageSystemTab(management, ctx) }
            vif({ management.tabIndex == EM_MANAGE_TAB_LOGS }) { manageLogsTab(management, toast, ctx) }
            vif({ management.tabIndex == EM_MANAGE_TAB_MAINT }) { manageMaintenanceTab(management, toast, ctx) }
        }
    }
    // 操作反馈统一在 tab 之外：切到哪个分区都能看到上一次操作的结果/错误
    vif({ management.busy.isNotEmpty() }) {
        textNote(text = { "操作进行中，请等待设备响应…" }, micro = true)
    }
    vif({ management.error.isNotEmpty() }) {
        textNote({ management.error }, { "error" }, micro = true)
    }
    vif({ management.result.isNotEmpty() }) {
        textNote(text = { management.result }, micro = true)
    }
}

// ============================================================
// 分区一：运行状态与输出音量
// ============================================================

private fun ViewContainer<*, *>.manageStatusTab(management: EchoMuseManagementState, ctx: Pager) {
    textNote(text = {
        if (management.online) "在线仅表示控制连接存在，并非心跳或实时监测；以下状态需手动读取。"
        else "设备离线：运行状态只保留历史上报，不代表当前值；音量不可下发。"
    }, micro = true)
    actionRow {
        secondaryButton(
            "读取运行状态",
            enabled = { management.approved && management.idle },
            loading = { management.busy == "runtime" },
            loadingText = "读取中…",
            icon = IconToken.Refresh,
        ) { management.readRuntime(ctx) }
    }
    outputBlock(content = { management.runtimeText }, height = DIAG_OUTPUT_HEIGHT)
    textNote("物理静音为只读上报，与输出音量为 0 是不同状态。未上报的字段显示为未知。", micro = true)
    vif({ !management.emos || !management.speaker }) {
        textNote(text = {
            if (!management.emos) "首版音量管理仅支持 emOS 设备。"
            else "设备未上报 speaker 能力，输出音量不可下发。"
        }, micro = true)
    }
    labeledIntField(
        "输出音量（0～127）",
        { management.volumeDraft },
        { management.volumeDraft = it },
        "填写整数；读取运行状态不会覆盖此草稿",
    )
    actionRow {
        primaryButton(
            "下发音量",
            enabled = { management.canVolume && management.idle },
            loading = { management.busy == "volume" },
            loadingText = "下发中…",
            icon = IconToken.Speaker,
        ) { management.setVolume(ctx) }
    }
    textNote("请求成功仅代表排队，实际上报音量请再次读取运行状态核对。", micro = true)
}

// ============================================================
// 分区二：系统信息
// ============================================================

private fun ViewContainer<*, *>.manageSystemTab(management: EchoMuseManagementState, ctx: Pager) {
    textNote(text = {
        when {
            !management.emos -> "系统信息仅支持 base_os 为 emos 的设备。"
            !management.online -> "设备离线：系统信息暂不可用。"
            else -> "手动读取一次快照；不会自动刷新。"
        }
    }, micro = true)
    actionRow {
        secondaryButton(
            "读取系统信息",
            enabled = { management.canSystem && management.idle },
            loading = { management.busy == "system/info" },
            loadingText = "读取中…",
            icon = IconToken.Info,
        ) { management.readInfo(ctx) }
    }
    vif({ management.infoLoaded }) {
        textNote(text = {
            if (management.infoHistorical) "系统信息：历史快照（连接状态已变化）" else "系统信息：手动读取快照"
        }, micro = true)
        outputBlock(content = { management.infoOutput.ifEmpty { "设备命令输出为空（退出码 0）" } }, height = DIAG_OUTPUT_HEIGHT)
    }
}

// ============================================================
// 分区三：日志（固定白名单来源，默认设备自身的 server.log）
// ============================================================

private fun ViewContainer<*, *>.manageLogsTab(
    management: EchoMuseManagementState,
    toast: ToastState,
    ctx: Pager,
) {
    textNote(text = {
        when {
            !management.emos -> "日志仅支持 base_os 为 emos 的设备。"
            !management.online -> "设备离线：日志需设备在线才能读取（不用连数据线，但设备必须在网）。"
            else -> "来源为固定白名单（不含任意路径），每次读取设备上日志文件的尾部 48 KB。"
        }
    }, micro = true)
    dropdownField(
        "日志来源（固定白名单）",
        { management.logSources.firstOrNull { it.first == management.logSource }?.second ?: "未知" },
        { management.logSources },
        { management.logSource },
        { management.sourceOpen },
        { management.sourceOpen = !management.sourceOpen },
        { management.logSource = it },
    )
    actionRow {
        secondaryButton(
            "读取日志",
            enabled = { management.canSystem && management.idle },
            loading = { management.busy.startsWith("system/logs") },
            loadingText = "读取中…",
            icon = IconToken.Text,
        ) { management.readLogs(ctx) }
        hGap(AdminSpace.sm)
        secondaryButton(
            "复制日志",
            enabled = { management.logsLoaded },
            icon = IconToken.Copy,
        ) {
            if (copyToClipboard(management.logsOutput)) toast.show("日志已复制到剪贴板", "ok")
            else toast.show("复制失败：当前平台剪贴板不可用", "error")
        }
    }
    vif({ management.logsLoaded }) {
        textNote(text = {
            "日志来源：${management.loadedLogSource}" +
                if (management.logsHistorical) "（历史快照，连接状态已变化）" else "（手动读取快照）"
        }, micro = true)
        outputBlock(
            content = { management.logsOutput.ifEmpty { "设备命令输出为空（退出码 0）" } },
            height = LOG_OUTPUT_HEIGHT,
        )
    }
}

// ============================================================
// 分区四：维护（重启 + 诊断导出）
// ============================================================

private fun ViewContainer<*, *>.manageMaintenanceTab(
    management: EchoMuseManagementState,
    toast: ToastState,
    ctx: Pager,
) {
    textNote(text = {
        when {
            !management.emos -> "重启仅支持 base_os 为 emos 的设备。"
            !management.online -> "设备离线：重启暂不可用。"
            else -> "仅提供确认重启；无任意 shell 输入或 OTA。"
        }
    }, micro = true)
    actionRow {
        vif({ !management.confirmReboot }) {
            secondaryButton("重启设备", enabled = { management.canSystem && management.idle }, icon = IconToken.Power) {
                management.confirmReboot = true
            }
        }
        vif({ management.confirmReboot }) {
            primaryButton(
                "确认重启",
                danger = { true },
                enabled = { management.canSystem && management.idle },
                loading = { management.busy == "system/reboot" },
                loadingText = "发送中…",
                icon = IconToken.Power,
            ) { management.reboot(ctx) }
            hGap(AdminSpace.sm)
            secondaryButton("取消", enabled = { management.idle }, icon = IconToken.Close) { management.confirmReboot = false }
        }
    }
    vif({ management.confirmReboot }) {
        textNote("重启会中断当前语音和控制连接；接口响应不能证明设备已实际重启。确认继续？", tone = "warn", micro = true)
    }
    actionRow {
        secondaryButton("复制诊断内容", icon = IconToken.Copy) {
            if (copyToClipboard(management.diagnosticText)) toast.show("诊断内容已复制", "ok")
            else toast.show("复制失败：当前平台剪贴板不可用", "error")
        }
    }
    textNote("诊断内容可能包含设备和网络信息，仅向可信人员分享。", micro = true)
}
