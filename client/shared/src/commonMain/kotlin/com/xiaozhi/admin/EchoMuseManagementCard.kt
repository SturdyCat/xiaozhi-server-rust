package com.xiaozhi.admin

import com.xiaozhi.admin.ui.*

import com.tencent.kuikly.core.base.ViewContainer
import com.tencent.kuikly.core.directives.vif
import com.tencent.kuikly.core.pager.Pager
import com.tencent.kuikly.core.views.Text

/** 设备卡的「设备管理」菜单面板：固定命令/固定日志源，不暴露 shell 或固件刷写功能。 */
fun ViewContainer<*, *>.echoMuseManagementPanel(
    row: EchoMuseDeviceRow,
    state: EchoMuseState,
    toast: ToastState,
    ctx: Pager,
) {
    val management = state.management
    fieldLabel("运行状态与输出音量")
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
        outputBlock(content = { management.runtimeText }, height = 200f)
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

        fieldLabel("emOS 系统管理")
        textNote(text = {
            when {
                !management.emos -> "系统信息、日志和重启仅支持 base_os 为 emos 的设备。"
                !management.online -> "设备离线：系统信息、日志和重启暂不可用。"
                else -> "仅提供系统信息、固定源日志与确认重启；无任意 shell 输入或 OTA。"
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
            outputBlock(content = { management.infoOutput.ifEmpty { "设备命令输出为空（退出码 0）" } }, height = 200f)
        }
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
        }
        vif({ management.logsLoaded }) {
            textNote(text = {
                "日志来源：${management.loadedLogSource}" +
                    if (management.logsHistorical) "（历史快照，连接状态已变化）" else "（手动读取快照）"
            }, micro = true)
            outputBlock(content = { management.logsOutput.ifEmpty { "设备命令输出为空（退出码 0）" } }, height = 200f)
        }
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
        vif({ management.busy.isNotEmpty() }) {
            textNote(text = { "操作进行中，请等待设备响应…" }, micro = true)
        }
        vif({ management.error.isNotEmpty() }) {
            textNote({ management.error }, { "error" }, micro = true)
        }
        vif({ management.result.isNotEmpty() }) {
            textNote(text = { management.result }, micro = true)
        }
        actionRow {
            secondaryButton("复制诊断内容", icon = IconToken.Copy) {
                if (copyToClipboard(management.diagnosticText)) toast.show("诊断内容已复制", "ok")
                else toast.show("复制失败：当前平台剪贴板不可用", "error")
            }
        }
        textNote("诊断内容可能包含设备和网络信息，仅向可信人员分享。", micro = true)
}
