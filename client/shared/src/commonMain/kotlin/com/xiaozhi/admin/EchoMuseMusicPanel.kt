package com.xiaozhi.admin

import com.xiaozhi.admin.ui.*

import com.tencent.kuikly.core.base.ViewContainer
import com.tencent.kuikly.core.directives.vif
import com.tencent.kuikly.core.pager.Pager

/**
 * 「音乐」面板（设备卡菜单 [ECHOMUSE_MENU_MUSIC]，EchoMuseDeviceCard 分发）。
 *
 * 音乐面控制（对齐官方 `audio_mix`）：音源 = 服务端数据目录 `music/` 下的 16-bit PCM
 * WAV（「文件」页上传），经 `POST …/music` 逐帧下发（0x04/0x05 独立平面）。播放中说话
 * 只 `duck` 压低不暂停；「停止」走 `music_flush` 丢弃设备缓冲（不可恢复）。
 *
 * 文件清单来自 `GET /api/files?path=music`（进面板/点刷新时拉取；**刻意安静**——目录
 * 缺失保持空清单不报错），选不到的路径（如自定义 `music_dir`、子目录）走手动输入兜底。
 * 离线/未就绪服务端 409，提示文案由 [EchoMuseState.playMusic]/[EchoMuseState.stopMusic] 给出。
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
            "音源是服务端 music/ 目录下的 16-bit PCM WAV（单/双声道、任意采样率均可，自动重采样到 48k）。",
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
    vif({ !row.online }) {
        textNote("设备离线：播放/停止需要设备在线（data 链就绪）才能接收音频帧。", micro = true)
    }
    textNote(
        "「停止并清空缓冲」不可恢复——设备里已缓冲的音乐会被丢弃，恢复播放请重新点播放；" +
            "语音轮次只会压低音乐，永远不会清缓冲。",
        micro = true,
    )
}
