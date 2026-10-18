package com.xiaozhi.admin

import com.tencent.kuikly.core.base.ViewContainer
import com.tencent.kuikly.core.directives.vfor
import com.tencent.kuikly.core.directives.vif
import com.tencent.kuikly.core.pager.Pager
import com.tencent.kuikly.core.views.Slider
import com.tencent.kuikly.core.views.Text
import com.tencent.kuikly.core.views.View

/**
 * 「EchoMuse 设备」页的渲染（纯展示，数据见 [EchoMuseState]）。
 *
 * 呈现规则与运维页同一口径（见 `OpsPages.kt` 文件头）：
 * - `enabled=false` 优先于一切：显示"未启用"提示并说明怎么开，不渲染空列表；
 * - 只在偏离时打标：在线不打标，离线/未批准才占视觉；
 * - 重启走**行内二次确认**（同 OTA 删除的 confirmKey 范式——重启是打断设备服务的动作）；
 * - WiFi 配置 202 只说明"指令已下发"，执行结果在设备灯环上，页面不假装知道结果。
 *
 * ⚠️ 渲染函数必须是 `ViewContainer` 扩展，并在目标容器闭包内**非限定**调用（见
 * `ConfigFormState` 文件头的 Kuikly DSL 绑定说明）。
 */
fun ViewContainer<*, *>.renderEchoMuse(
    state: EchoMuseState,
    toast: ToastState,
    ctx: Pager,
) {
    opsHeader(
        { if (state.loading && !state.loaded) "加载中…" else state.summaryText() },
        { state.loading },
    ) { state.refresh(ctx) }
    opsError(state.loadError)
    vif({ state.loading && !state.loaded }) { opsSkeleton() }

    // ===== 服务端未开启 [echomuse]：给出可行动的开启指引，不渲染空列表 =====
    vif({ state.loaded && !state.enabled }) {
        groupedCard("EchoMuse 未启用") {
            textNote(
                "服务端未开启 EchoMuse 能力（[echomuse] 配置段）。开启并重启服务端后，" +
                    "EchoMuse 设备会在连上服务端时登记到这里，经批准后即可远程管理" +
                    "（重启、下发 WiFi 配置）。",
                micro = true,
            )
        }
    }

    vif({ state.enabled }) {
        vif({ state.loaded && state.devices.size == 0 }) {
            textNote(
                "还没有 EchoMuse 设备登记。设备首次连上服务端后会出现在这里（未批准状态），" +
                    "批准后才能远程下发指令。",
                micro = true,
            )
        }

        vfor({ state.devices }) { row ->
            echoMuseDeviceCard(row, state, toast, ctx)
        }

        textNote(
            "指令为异步下发：接口返回成功只代表服务端已把指令排队送给设备，执行结果以设备灯环为准。",
            micro = true,
        )
    }
}

/** 一台 EchoMuse 设备卡片：状态 + 元信息 + 批准 / 重启 / WiFi 配置。 */
private fun ViewContainer<*, *>.echoMuseDeviceCard(
    row: EchoMuseDeviceRow,
    state: EchoMuseState,
    toast: ToastState,
    ctx: Pager,
) {
    groupedCard(row.title) {
        View {
            attr {
                flexDirectionRow()
                alignItemsCenter()
                paddingBottom(AdminSpace.xs)
            }
            Text {
                attr {
                    flex(1f)
                    fontSize(AdminType.caption)
                    color(AdminColors.textSecondary)
                    text(if (row.label.isNotEmpty() && row.label != row.deviceId) row.deviceId else row.subtitle)
                }
            }
            // 只在偏离时打标：在线是正常态不打标；离线/未批准才提示
            if (!row.online) {
                statusBadge(state = { "idle" }, text = { "离线" })
            } else if (!row.approved) {
                statusBadge(state = { "busy" }, text = { "待批准" })
            }
        }
        if (row.subtitle.isNotEmpty()) {
            Text {
                attr {
                    fontSize(AdminType.caption)
                    color(AdminColors.textTertiary)
                    text(row.subtitle)
                }
            }
        }
        if (row.metaText.isNotEmpty()) {
            textNote(row.metaText, micro = true)
        }

        // ===== 未批准：批准表单（可填 label）=====
        if (!row.approved) {
            actionRow {
                secondaryButton(
                    if (state.approveTarget == row.deviceId) "收起" else "批准",
                ) { state.toggleApprove(row.deviceId) }
            }
            vif({ state.approveTarget == row.deviceId }) {
                labeledField(
                    "设备名称（label，可留空）",
                    { state.approveDraft },
                    { state.approveDraft = it },
                    "如 客厅音箱",
                )
                actionRow {
                    primaryButton(
                        "确认批准",
                        loading = { state.actionBusy },
                        loadingText = "批准中…",
                    ) {
                        state.approve(ctx, row.deviceId) { ok, msg ->
                            toast.show(msg, if (ok) "ok" else "error")
                            if (ok) state.refresh(ctx)
                        }
                    }
                }
            }
        }

        // ===== 已批准：重启（行内二次确认）+ WiFi 配置 =====
        if (row.approved) {
            actionRow {
                vif({ state.confirmRebootId == row.deviceId }) {
                    primaryButton(
                        "确认重启",
                        danger = { true },
                        enabled = { row.online },
                        loading = { state.actionBusy },
                        loadingText = "下发中…",
                    ) {
                        state.reboot(ctx, row.deviceId) { ok, msg ->
                            toast.show(msg, if (ok) "ok" else "error")
                        }
                    }
                    View { attr { width(AdminSpace.sm) } }
                    secondaryButton("取消") { state.confirmRebootId = "" }
                }
                vif({ state.confirmRebootId != row.deviceId }) {
                    secondaryButton("重启", enabled = { row.online }) { state.confirmRebootId = row.deviceId }
                    View { attr { width(AdminSpace.sm) } }
                    secondaryButton(
                        if (state.wifiTarget == row.deviceId) "收起" else "WiFi 配置",
                        enabled = { row.online },
                    ) { state.toggleWifi(row.deviceId) }
                    View { attr { width(AdminSpace.sm) } }
                    secondaryButton(
                        if (state.settingsTarget == row.deviceId) "收起设置" else "唤醒词设置",
                    ) { state.toggleSettings(row.deviceId) }
                }
            }
            vif({ !row.online }) {
                textNote("设备离线：重启 / WiFi 配置指令需要设备在线才能接收，唤醒设备后可用。", micro = true)
            }
            vif({ state.wifiTarget == row.deviceId }) {
                labeledField("WiFi 名称（SSID）", { state.wifiSsid }, { state.wifiSsid = it }, "路由器名称")
                labeledField("WiFi 密码（PSK）", { state.wifiPsk }, { state.wifiPsk = it }, "密码")
                actionRow {
                    primaryButton(
                        "下发 WiFi 配置",
                        loading = { state.actionBusy },
                        loadingText = "下发中…",
                    ) {
                        state.sendWifi(ctx, row.deviceId) { ok, msg ->
                            toast.show(msg, if (ok) "ok" else "error")
                        }
                    }
                }
            }

            // ===== 唤醒词设置：离线也可编辑保存（服务端持久化，设备下次接入自动推送）=====
            vif({ state.settingsTarget == row.deviceId }) {
                echoMuseWakeSettings(state, toast, ctx)
            }
        }
    }
}

/**
 * 唤醒词设置表单（展开区内渲染；草稿在 [EchoMuseState]，展开时按设备快照填充）。
 *
 * 与重启/WiFi 不同：本表单**离线也可用**——服务端持久化后设备下次接入自动推送，
 * 202 = 设备在线已即时下发，200 = 已保存待上线。
 */
private fun ViewContainer<*, *>.echoMuseWakeSettings(
    state: EchoMuseState,
    toast: ToastState,
    ctx: Pager,
) {
    switchRow("响应唤醒词（关闭后按键仍可用）", state.wakeWordEnabled) { state.wakeWordEnabled = !state.wakeWordEnabled }
    switchRow("设备本地唤醒识别", state.owwOnDevice) { state.owwOnDevice = !state.owwOnDevice }
    labeledField("唤醒词模型", { state.owwModel }, { state.owwModel = it }, "如 alexa")

    // 灵敏度滑杆：0~1，步进 0.05（官方 Slider 无 step，拖动时量化）
    fieldLabel("唤醒灵敏度阈值（越大越不易误触）")
    View {
        attr {
            flexDirectionRow()
            alignItemsCenter()
            marginTop(AdminSpace.xs)
        }
        Slider {
            attr {
                flex(1f)
                height(28f)
                currentProgress(state.owwThreshold.toFloat())
                progressColor(AdminColors.accent)
                trackColor(AdminColors.divider)
                thumbColor(AdminColors.accent)
            }
            event {
                progressDidChanged { p ->
                    state.owwThreshold = (p * 20).let { kotlin.math.round(it) / 20.0 }
                }
            }
        }
        Text {
            attr {
                marginLeft(AdminSpace.sm)
                width(48f)
                fontSize(AdminType.caption)
                color(AdminColors.textSecondary)
                text((kotlin.math.round(state.owwThreshold * 100) / 100.0).toString())
            }
        }
    }

    switchRow("唤醒提示音", state.wakeSound) { state.wakeSound = !state.wakeSound }
    labeledField("提示音音量", { state.wakeSoundLevel }, { state.wakeSoundLevel = it }, "0.5")
    labeledField("麦克风编号（0=中心，0~6）", { state.wakeMic }, { state.wakeMic = it }, "0")

    actionRow {
        primaryButton(
            "保存唤醒词设置",
            loading = { state.actionBusy },
            loadingText = "保存中…",
        ) {
            state.saveSettings(ctx, state.settingsTarget) { ok, msg ->
                toast.show(msg, if (ok) "ok" else "error")
            }
        }
    }
    textNote(
        "设备在线时设置即时下发；离线时保存到服务端，设备下次接入自动生效（结果看设备灯环）。",
        micro = true,
    )
}
