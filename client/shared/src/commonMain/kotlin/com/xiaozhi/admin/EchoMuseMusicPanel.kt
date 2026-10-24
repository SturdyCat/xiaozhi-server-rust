package com.xiaozhi.admin

import com.xiaozhi.admin.ui.*

import com.tencent.kuikly.core.base.ViewContainer
import com.tencent.kuikly.core.directives.vif
import com.tencent.kuikly.core.pager.Pager

/**
 * 「音乐」面板（设备卡菜单 [ECHOMUSE_MENU_MUSIC]，EchoMuseDeviceCard 分发）。
 *
 * 音乐面控制（对齐官方 `audio_mix`），两类音源共用同一下行平面（逐帧下发 0x04/0x05，
 * 播放中说话只 `duck` 压低不暂停；「停止」走 `music_flush` 丢弃设备缓冲，不可恢复）：
 * - 本地 WAV：服务端数据目录 `music/` 下的 16-bit PCM WAV（「文件」页上传）；
 * - DLNA：服务端扫描/浏览局域网媒体服务器，选曲后服务端拉流→解码→推流（本机零协议）。
 *
 * 文件清单来自 `GET /api/files?path=music`（进面板/点刷新时拉取；**刻意安静**——目录
 * 缺失保持空清单不报错），选不到的路径（如自定义 `music_dir`、子目录）走手动输入兜底。
 * DLNA 三个动作（scan/browse/play_url）全部是服务端职责，管理台只调接口。
 * 离线/未就绪服务端 409，提示文案由 [EchoMuseState.playMusic]/[EchoMuseState.stopMusic]
 * 与 `dlnaScan`/`dlnaBrowse`/`dlnaPlay` 给出。
 */
internal fun ViewContainer<*, *>.echoMuseMusicPanel(
    row: EchoMuseDeviceRow,
    state: EchoMuseState,
    toast: ToastState,
    ctx: Pager,
    onOpenMusicFiles: () -> Unit = {},
) {
    textNote(
        "音乐在独立平面与语音混音：说话时自动压低（duck）、说完恢复，不打断播放。" +
            "音源可选服务端 music/ 目录下的 16-bit PCM WAV（自动重采样 48k），" +
            "或局域网 DLNA 媒体服务器的曲目（扫描与拉流解码全在服务端）。",
        micro = true,
    )
    dropdownField(
        "选择音频文件（music/）",
        {
            state.musicSelection.ifEmpty {
                if (state.musicFilesLoaded) "未选择" else "（清单未加载）"
            }
        },
        { state.musicFiles }, { state.musicSelection }, { state.musicFileOpen },
        { state.musicFileOpen = !state.musicFileOpen }, { state.musicSelection = it },
        help = "清单来自服务端数据目录 music/；上传新文件请点下方「管理音乐文件」。",
    )
    // 手动兜底：自定义 music_dir / 子目录路径（相对 music_dir，服务端拒绝越界与绝对路径）
    labeledField(
        "或手动输入路径（相对 music_dir）",
        { state.musicManualPath }, { state.musicManualPath = it },
        "如 wake/theme.wav（留空用上方选择）",
    )
    actionRow {
        secondaryButton("刷新清单", icon = IconToken.Refresh) { state.musicFilesRefresh(ctx) }
        hGap(AdminSpace.sm)
        secondaryButton("管理音乐文件（music/）", icon = IconToken.Folder) { onOpenMusicFiles() }
    }
    actionRow {
        primaryButton(
            "播放",
            enabled = { row.online },
            loading = { state.actionBusy },
            loadingText = "下发中…",
            icon = IconToken.Play,
        ) {
            state.playMusic(ctx, row.deviceId) { ok, msg ->
                toast.show(msg, if (ok) "ok" else "error")
            }
        }
        hGap(AdminSpace.sm)
        secondaryButton(
            "停止并清空缓冲",
            enabled = { row.online },
            loading = { state.actionBusy },
            icon = IconToken.Stop,
        ) {
            state.stopMusic(ctx, row.deviceId) { ok, msg ->
                toast.show(msg, if (ok) "ok" else "error")
            }
        }
    }
    vif({ state.musicFilesLoaded && state.musicFiles.isEmpty() }) {
        textNote(
            "music/ 目录还没有音频文件：到「文件」页上传 16-bit PCM WAV（或调整服务端 " +
                "[echomuse].music_dir 后手动输入路径）。",
            micro = true,
        )
    }
    // ── DLNA 音源：扫描/浏览/拉流/推流全在服务端，本机只选服务器与曲目 ──
    groupLabel("DLNA 音源（局域网媒体服务器）")
    textNote(
        "扫描、浏览与推流全部由服务端完成：曲目从局域网 DLNA 媒体服务器直接取流，" +
            "解码后经同一音乐平面下发（与 WAV 音源、语音混音行为一致）。",
        micro = true,
    )
    dropdownField(
        "DLNA 媒体服务器",
        {
            state.dlnaServerSelection.ifEmpty {
                if (state.dlnaServersLoaded) "未发现服务器" else "（尚未扫描）"
            }
        },
        { state.dlnaServers }, { state.dlnaServerSelection }, { state.dlnaServerOpen },
        { state.dlnaServerOpen = !state.dlnaServerOpen }, { state.dlnaSelectServer(it) },
        help = "扫描基于 SSDP 组播（约 4 秒）；容器部署收不到组播时需 host 网络部署。",
    )
    actionRow {
        secondaryButton(
            "扫描局域网",
            icon = IconToken.Refresh,
            loading = { state.actionBusy },
        ) {
            state.dlnaScan(ctx, row.deviceId) { ok, msg ->
                toast.show(msg, if (ok) "ok" else "error")
            }
        }
    }
    vif({ state.dlnaServerSelection.isNotEmpty() }) {
        dropdownField(
            "曲目",
            {
                state.dlnaTrackSelection.ifEmpty {
                    if (state.dlnaTracksLoaded) "没有曲目" else "（尚未浏览）"
                }
            },
            { state.dlnaTracks }, { state.dlnaTrackSelection }, { state.dlnaTrackOpen },
            { state.dlnaTrackOpen = !state.dlnaTrackOpen }, { state.dlnaTrackSelection = it },
            help = "清单由服务端递归浏览该服务器的音频目录生成；切换服务器后需重新浏览。",
        )
        actionRow {
            secondaryButton(
                "浏览曲目",
                icon = IconToken.Refresh,
                loading = { state.actionBusy },
            ) {
                state.dlnaBrowse(ctx, row.deviceId) { ok, msg ->
                    toast.show(msg, if (ok) "ok" else "error")
                }
            }
            hGap(AdminSpace.sm)
            primaryButton(
                "播放所选曲目",
                enabled = { row.online && state.dlnaTrackSelection.isNotEmpty() },
                loading = { state.actionBusy },
                loadingText = "排队中…",
                icon = IconToken.Play,
            ) {
                state.dlnaPlay(ctx, row.deviceId) { ok, msg ->
                    toast.show(msg, if (ok) "ok" else "error")
                }
            }
        }
    }
    vif({ !row.online }) {
        textNote("设备离线：播放/停止需要设备在线（data 链就绪）才能接收音频帧。", micro = true)
    }
    textNote(
        "「停止并清空缓冲」不可恢复——设备里已缓冲的音乐会被丢弃，恢复播放请重新点播放；" +
            "语音轮次只会压低音乐，永远不会清缓冲。",
        micro = true,
    )
}
