package com.xiaozhi.admin

import androidx.compose.runtime.Composable
import com.gearui.components.button.Button
import com.gearui.components.button.ButtonType
import com.tencent.kuikly.compose.ui.unit.dp
import com.xiaozhi.admin.contracts.IconToken
import com.xiaozhi.admin.icons.GearIcon
import com.xiaozhi.admin.kit.actionRow

/**
 * 原生扩展的 OWW 资产面板：只保留资产五态、刷新与 VAD 安装。
 * 读不到设备不等于缺失，不能把 unknown 当成需要重装。
 * 停留期间由 [EchoMuseDetailState.enterTab] 每 30 秒刷新，离开即停。
 * 版本、重启与诊断由官方 UI 承载，不在这里重复。
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

/**
 * 资产现状：VAD（`silero_vad.onnx`）与 ORT（`libonnxruntime.so`）各自的五态 + 设备剩余空间。
 *
 * 为什么这两件要单独管：缺 VAD 时设备的语音门退化成本底 RMS 门限，现场表现为
 * "按了键、说了话、没反应"——而设备日志里只会写 `no speech detected within timeout`，
 * 从管理台看不到根因，所以这里必须显式标出"缺哪件、是否装错版本、空间够不够"。
 */
@Composable
internal fun EmAssets(state: EchoMuseState, toast: ToastState) {
    val detail = state.detail
    val pager = LocalAdminPagerOrThrow()
    EmGroupLabel("OWW 资产（Silero VAD + onnxruntime）")
    if (!detail.owwLoaded) {
        EmNote(
            if (detail.owwLoading) "正在读取设备侧资产清单…"
            else "尚未读取：进本 tab 会自动读一次，停留期间每 30 秒刷新（离开即停）",
        )
    } else {
        val vad = { detail.owwState?.optJSONObject("vad")?.text("state").orEmpty() }
        val ort = { detail.owwState?.optJSONObject("ort")?.text("state").orEmpty() }
        EmInfoRow("VAD 模型（silero_vad.onnx）", emAssetText(vad()))
        EmInfoRow("推理运行时（libonnxruntime.so）", emAssetText(ort()))
        EmInfoRow(
            "设备剩余空间",
            run {
                val free = (detail.owwState?.opt("free_mb") as? Number)?.toLong()
                if (free == null) "$EM_DASH（设备未上报）" else "$free MB"
            },
        )
        if (emAssetNeedsInstall(vad())) {
            EmWarn(
                "VAD 不是最新或缺失：设备会退化成 RMS 门限判人声，表现为「按了键说了话没反应」。" +
                    "点下面的「重新安装 VAD」把服务端这份推过去（设备下一轮开轮自动生效，不用重启）。",
            )
        }
        if (emAssetNeedsInstall(ort())) {
            EmWarn(
                "onnxruntime 运行时缺失或不匹配：设备 dlopen 失败后会静默退回 RMS。这项由服务端在" +
                    "设备接入时自动补，若一直是这个状态请确认设备剩余空间（见上）与设备端目录权限。",
            )
        }
    }
    actionRow {
        Button(
            text = "刷新资产状态",
            onClick = { detail.readOwwState(pager) },
            type = ButtonType.OUTLINE,
            loading = detail.owwLoading,
            disabled = !(detail.approved && !detail.owwLoading),
            iconWidget = { GearIcon(token = IconToken.Refresh, size = 16.dp) },
        )
        Button(
            text = if (detail.busy == "oww/install-vad") "安装中…" else "重新安装 VAD",
            onClick = {
                detail.reinstallVad(pager) { ok, msg ->
                    toast.show(msg, if (ok) "ok" else "error")
                    if (ok) detail.readOwwState(pager)
                }
            },
            type = ButtonType.FILL,
            loading = detail.busy == "oww/install-vad",
            disabled = !(detail.canSystem && detail.idle),
            iconWidget = { GearIcon(token = IconToken.Save, size = 16.dp) },
        )
    }
    EmNote(
        "安装是同步执行的（装完才回包，可能十几秒）：先建目录，再经 shell 面把模型卷里的" +
            "typed 版 VAD 推到设备并核对 sha256，校验不符时设备上原文件保持不变。",
    )
}
