package com.xiaozhi.admin

import com.xiaozhi.admin.ui.*

import com.tencent.kuikly.core.base.ViewContainer
import com.tencent.kuikly.core.directives.vfor
import com.tencent.kuikly.core.directives.vif
import com.tencent.kuikly.core.pager.Pager
import com.tencent.kuikly.core.views.Text

/**
 * 「EchoMuse 设备」页的渲染（纯展示，数据见 [EchoMuseState]）。
 *
 * 本文件只负责页面壳体（摘要头/错误/骨架/未启用与空列表提示/设备卡列表）；
 * 单台设备卡片（标题行 + 六 tab 内容区，未批准则是审批单页）见 `EchoMuseDeviceCard.kt`
 * 与 `EchoMuseDetailTabs.kt`。
 *
 * 呈现规则与运维页同一口径（见 `OpsPages.kt` 文件头）：
 * - `enabled=false` 优先于一切：显示"未启用"提示并说明怎么开，不渲染空列表；
 * - 只在偏离时打标：在线不打标，离线/未批准才占视觉；停留本页时订阅管理台 WS
 *   事件（`/api/admin/ws` 推送，变化才刷新；[isActive] 为假时退订，断线自动重连）；
 * - 重启/删除记录走**行内二次确认**（同 OTA 删除的 confirmKey 范式——都是不可逆动作）；
 * - WiFi 配置 202 只说明"指令已下发"，执行结果在设备灯环上，页面不假装知道结果。
 *
 * ⚠️ 渲染函数必须是 `ViewContainer` 扩展，并在目标容器闭包内**非限定**调用（见
 * `ConfigFormState` 文件头的 Kuikly DSL 绑定说明）。
 */
fun ViewContainer<*, *>.renderEchoMuse(
    state: EchoMuseState,
    toast: ToastState,
    ctx: Pager,
    isActive: () -> Boolean,
    /** 唤醒词面板「管理模型文件（oww-models/）」入口（跳「文件」页对应目录，AdminPage 接线）。 */
    onOpenModelFiles: () -> Unit = {},
) {
    state.ensureLiveUpdates(ctx, isActive)
    opsHeader(
        { if (state.loading && !state.loaded) "加载中…" else state.summaryText() },
        { state.loading },
    ) { state.refresh(ctx) }
    vif({ state.loadError.isNotEmpty() }) {
        Text {
            attr {
                fontSize(AdminType.micro)
                color(AdminColors.dangerTintText)
                text("${state.loadError}（确认管理令牌与服务端地址后点“刷新”）")
            }
        }
    }
    vif({ state.loading && !state.loaded }) { opsSkeleton() }

    // ===== 服务端未开启 [echomuse]：给出可行动的开启指引，不渲染空列表 =====
    vif({ state.loaded && !state.enabled }) {
        groupedCard("EchoMuse 未启用") {
            textNote(
                "服务端未开启 EchoMuse 能力。请到「配置 → EchoMuse」分区打开「启用 EchoMuse 设备入口」" +
                    "并重启服务端；之后 EchoMuse 设备会在连上服务端时登记到这里，经批准后即可远程管理" +
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
            echoMuseDeviceCard(row, state, toast, ctx, onOpenModelFiles)
        }

        textNote(
            "指令为异步下发：接口返回成功只代表服务端已把指令排队送给设备，执行结果以设备灯环为准。",
            micro = true,
        )
    }
}
