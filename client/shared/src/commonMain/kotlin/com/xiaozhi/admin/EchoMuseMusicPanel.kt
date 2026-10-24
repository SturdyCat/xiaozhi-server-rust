package com.xiaozhi.admin

import com.xiaozhi.admin.ui.*

import com.tencent.kuikly.core.base.ViewContainer
import com.tencent.kuikly.core.directives.vif
import com.tencent.kuikly.core.pager.Pager

/**
 * 「音乐」面板（设备卡菜单 [ECHOMUSE_MENU_MUSIC]，EchoMuseDeviceCard 分发）。
 *
 * 音乐面控制（对齐官方 `audio_mix`），音源**仅 DLNA**：服务端扫描/浏览局域网媒体
 * 服务器，选曲后由服务端拉流→解码→经同一下行平面逐帧下发（0x04/0x05，播放中说话
 * 只 `duck` 压低不暂停；「停止」走 `music_flush` 丢弃设备缓冲，不可恢复）。
 *
 * 三个动作（scan/browse/play_url）全部是服务端职责，管理台只调接口、不接触任何
 * DLNA 协议细节。播放/停止需要设备在线（data 链就绪）；扫描/浏览不依赖设备在线。
 * 提示文案由 [EchoMuseState.dlnaScan]/[EchoMuseState.dlnaBrowse]/[EchoMuseState.dlnaPlay]/
 * [EchoMuseState.stopMusic] 给出。
 */
internal fun ViewContainer<*, *>.echoMuseMusicPanel(
    row: EchoMuseDeviceRow,
    state: EchoMuseState,
    toast: ToastState,
    ctx: Pager,
) {
    textNote(
        "音乐在独立平面与语音混音：说话时自动压低（duck）、说完恢复，不打断播放。" +
            "音源是局域网 DLNA 媒体服务器的曲目——扫描、浏览、拉流解码与推流全部由服务端完成。",
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
