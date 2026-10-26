package com.xiaozhi.admin

import com.xiaozhi.admin.ui.*

import com.tencent.kuikly.core.base.ViewContainer
import com.tencent.kuikly.core.directives.vif
import com.tencent.kuikly.core.pager.Pager

/**
 * 更新 tab：**只读版本信息 + OWW 资产五态 + 维护三件事**。
 *
 * ⚠️ 这一页**没有、也不许有 OTA / 固件刷写 / 恢复出厂**：
 * 本服务端不提供固件升级通道（设备固件走官方渠道），"更新"在这里只有两件事——
 * 看清版本、把 OWW 资产（Silero VAD + onnxruntime）补齐。任何"升级固件""回滚版本"
 * "一键刷机"的文案都是假的，写上去就是给运维挖坑。
 *
 * 资产五态与 `GET …/oww/state` 逐字对应（`server/src/device/echomuse/oww_state.rs`）：
 * `installed` / `outdated` / `blocked` / `unknown` / `missing`。最关键的一条语义来自
 * 官方 `em_oww_sync`：**读不到设备 ≠ 缺失**——读不到一律 `unknown`，不能催用户"重装"。
 * 该接口在本 tab 停留期间每 30 s 自动读一次（[EchoMuseDetailState.enterTab] 起、
 * 离开即停），也可手动「刷新资产状态」。
 *
 * 维护三件事：重启设备（行内二次确认）、重新安装 VAD、复制诊断内容。
 */

/** 单件资产的五态 → 人话（字面量与后端 `AssetState::as_str` 一一对应）。 */
private fun emAssetText(state: String): String = when (state) {
    "installed" -> "已安装（与服务端待下发版本一致）"
    "outdated" -> "版本不一致（设备上那份与服务端待下发的摘要不同，建议重装）"
    "blocked" -> "缺失且空间不足（设备剩余低于 50 MB，现在推送大概率失败）"
    "missing" -> "设备上没有这份文件"
    else -> "无法判定（读不到设备，或服务端资产未就绪）"
}

/** 资产状态是否值得催"重装"（`installed`/`unknown` 不该催：一个是最新、一个是没读到）。 */
private fun emAssetNeedsInstall(state: String): Boolean = state == "outdated" || state == "missing"

internal fun ViewContainer<*, *>.echoMuseUpdatesTab(
    row: EchoMuseDeviceRow,
    state: EchoMuseState,
    toast: ToastState,
    ctx: Pager,
) {
    echoMuseVersions(row)
    echoMuseAssets(state, toast, ctx)
    echoMuseMaintenance(state, toast, ctx)
}

// ============================================================
// 版本（只读）
// ============================================================

/**
 * 版本信息全部**只读**：`version`/`board`/`base_os`/内核来自设备 register 上报，
 * 管理台没有任何写入口——想换固件请走官方渠道，本页只帮你看清"这块板现在是什么"。
 */
private fun ViewContainer<*, *>.echoMuseVersions(row: EchoMuseDeviceRow) {
    groupLabel("版本（只读）")
    infoRow({ "固件版本" }, { row.version.ifEmpty { EM_DASH } }, topGap = 0f)
    infoRow({ "板型" }, { row.board.ifEmpty { EM_DASH } })
    infoRow({ "基础系统" }, { row.baseOs.ifEmpty { EM_DASH } })
    infoRow(
        { "内核" },
        {
            when {
                row.kernelRelease.isEmpty() -> EM_DASH
                row.kernelArch.isEmpty() -> row.kernelRelease
                else -> "${row.kernelRelease}（${row.kernelArch}）"
            }
        },
    )
    textNote(
        "版本信息由设备接入时上报，只读。本服务端不提供固件升级 / 刷写 / 恢复出厂通道——" +
            "换固件请走官方渠道；这里能做的只有补齐 OWW 资产（见下）。",
        micro = true,
    )
}

// ============================================================
// OWW 资产（五态 + 重装 VAD）
// ============================================================

/**
 * 资产现状：VAD（`silero_vad.onnx`）与 ORT（`libonnxruntime.so`）各自的五态 + 设备剩余空间。
 *
 * 为什么这两件要单独管：缺 VAD 时设备的语音门退化成本底 RMS 门限，现场表现为
 * "按了键、说了话、没反应"——而设备日志里只会写 `no speech detected within timeout`，
 * 从管理台看不到根因，所以这里必须显式标出"缺哪件、是否装错版本、空间够不够"。
 */
private fun ViewContainer<*, *>.echoMuseAssets(state: EchoMuseState, toast: ToastState, ctx: Pager) {
    val detail = state.detail
    groupLabel("OWW 资产（Silero VAD + onnxruntime）")
    vif({ !detail.owwLoaded }) {
        detailLine(
            {
                if (detail.owwLoading) "正在读取设备侧资产清单…"
                else "尚未读取：进本 tab 会自动读一次，停留期间每 30 秒刷新（离开即停）"
            },
            topGap = 0f,
        )
    }
    vif({ detail.owwLoaded }) {
        val vad = { detail.owwState?.optJSONObject("vad")?.text("state").orEmpty() }
        val ort = { detail.owwState?.optJSONObject("ort")?.text("state").orEmpty() }
        infoRow({ "VAD 模型（silero_vad.onnx）" }, { emAssetText(vad()) }, topGap = 0f)
        infoRow({ "推理运行时（libonnxruntime.so）" }, { emAssetText(ort()) })
        infoRow(
            { "设备剩余空间" },
            {
                val free = (detail.owwState?.opt("free_mb") as? Number)?.toLong()
                if (free == null) "$EM_DASH（设备未上报）" else "$free MB"
            },
        )
        vif({ emAssetNeedsInstall(vad()) }) {
            textNote(
                "VAD 不是最新或缺失：设备会退化成 RMS 门限判人声，表现为「按了键说了话没反应」。" +
                    "点下面的「重新安装 VAD」把服务端这份推过去（设备下一轮开轮自动生效，不用重启）。",
                tone = "warn",
                micro = true,
            )
        }
        vif({ emAssetNeedsInstall(ort()) }) {
            textNote(
                "onnxruntime 运行时缺失或不匹配：设备 dlopen 失败后会静默退回 RMS。这项由服务端在" +
                    "设备接入时自动补，若一直是这个状态请确认设备剩余空间（见上）与设备端目录权限。",
                tone = "warn",
                micro = true,
            )
        }
    }
    actionRow {
        secondaryButton(
            "刷新资产状态",
            enabled = { detail.approved && !detail.owwLoading },
            loading = { detail.owwLoading },
            icon = IconToken.Refresh,
        ) { detail.readOwwState(ctx) }
        hGap(AdminSpace.sm)
        primaryButton(
            "重新安装 VAD",
            enabled = { detail.canSystem && detail.idle },
            loading = { detail.busy == "oww/install-vad" },
            loadingText = "安装中…",
            icon = IconToken.Save,
        ) {
            detail.reinstallVad(ctx) { ok, msg ->
                toast.show(msg, if (ok) "ok" else "error")
                if (ok) detail.readOwwState(ctx)
            }
        }
    }
    textNote(
        "安装是同步执行的（装完才回包，可能十几秒）：先建目录，再经 shell 面把模型卷里的" +
            "typed 版 VAD 推到设备并核对 sha256，校验不符时设备上原文件保持不变。",
        micro = true,
    )
}

// ============================================================
// 维护（重启 / 复制诊断）
// ============================================================

/**
 * 维护三件事的落点（原「设备管理 → 维护」整块归位）：
 * - **重启设备**：二次确认后才发；`dispatched` 只代表请求已发出，不代表设备真的重启了；
 * - **复制诊断内容**：设备事实 + 运行状态 + 资产状态 + 系统信息 + 已读到的日志；
 * - **重新安装 VAD** 在上面的资产块里（它属于资产维护，不属于重启）。
 */
private fun ViewContainer<*, *>.echoMuseMaintenance(state: EchoMuseState, toast: ToastState, ctx: Pager) {
    val detail = state.detail
    groupLabel("维护")
    textNote(
        text = {
            when {
                !detail.emos -> "重启仅支持 base_os 为 emos 的设备。"
                !detail.online -> "设备离线：重启暂不可用。"
                else -> "仅提供确认重启；无任意 shell 输入、无固件刷写、无恢复出厂。"
            }
        },
        micro = true,
    )
    actionRow {
        vif({ !detail.confirmReboot }) {
            secondaryButton("重启设备", enabled = { detail.canSystem && detail.idle }, icon = IconToken.Power) {
                detail.confirmReboot = true
            }
        }
        vif({ detail.confirmReboot }) {
            primaryButton(
                "确认重启",
                danger = { true },
                enabled = { detail.canSystem && detail.idle },
                loading = { detail.busy == "system/reboot" },
                loadingText = "发送中…",
                icon = IconToken.Power,
            ) { detail.reboot(ctx) }
            hGap(AdminSpace.sm)
            secondaryButton("取消", enabled = { detail.idle }, icon = IconToken.Close) {
                detail.confirmReboot = false
            }
        }
    }
    vif({ detail.confirmReboot }) {
        textNote("重启会中断当前语音和控制连接；接口响应不能证明设备已实际重启。确认继续？", tone = "warn", micro = true)
    }

    actionRow {
        secondaryButton("复制诊断内容", icon = IconToken.Copy) {
            // 诊断 = 设备事实 + 运行状态 + 资产状态 + 系统信息 + 已读到的日志
            val text = listOf(detail.diagnosticText, state.logs.diagnosticSection)
                .filter { it.isNotEmpty() }
                .joinToString("\n\n")
            if (copyToClipboard(text)) toast.show("诊断内容已复制", "ok")
            else toast.show("复制失败：当前平台剪贴板不可用", "error")
        }
    }
    textNote("诊断内容可能包含设备和网络信息，仅向可信人员分享。", micro = true)
}
