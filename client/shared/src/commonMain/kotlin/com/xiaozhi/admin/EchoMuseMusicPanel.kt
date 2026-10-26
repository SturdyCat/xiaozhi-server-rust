package com.xiaozhi.admin

import com.xiaozhi.admin.ui.*

import com.tencent.kuikly.core.base.ViewContainer
import com.tencent.kuikly.core.directives.vif
import com.tencent.kuikly.core.pager.Pager

/**
 * 配置 tab 的 **Stage 08 区块内容**（标题与"为什么官方没有这一组"的说明在
 * `EchoMuseConfigStages.kt::stageMusic`，本文件只画控件）。
 *
 * 音乐面控制（对齐官方 `audio_mix`），音源**仅 DLNA**：服务端扫描/浏览局域网媒体
 * 服务器，选曲后由服务端拉流→解码→经同一下行平面逐帧下发（0x04/0x05，播放中说话
 * 只 `duck` 压低不暂停；「停止」走 `music_flush` 丢弃设备缓冲，不可恢复）。
 *
 * 三个动作（scan/browse/play_url）全部是服务端职责，管理台只调接口、不接触任何
 * DLNA 协议细节。播放/停止需要设备在线（data 链就绪）；扫描/浏览不依赖设备在线。
 * 提示文案由 [EchoMuseState.dlnaScan]/[EchoMuseState.dlnaBrowse]/[EchoMuseState.dlnaPlay]/
 * [EchoMuseState.stopMusic] 给出。
 *
 * 本组**不写任何 config 键**，所以「下发配置」对这里无效——音乐是动作面板，不是设置。
 */
internal fun ViewContainer<*, *>.echoMuseMusicStage(
    row: EchoMuseDeviceRow,
    state: EchoMuseState,
    toast: ToastState,
    ctx: Pager,
) {
    val music = state.music
    textNote(
        "音乐在独立平面与语音混音：说话时自动压低（duck）、说完恢复，不打断播放。" +
            "音源是局域网 DLNA 媒体服务器的曲目——扫描、浏览、拉流解码与推流全部由服务端完成。",
        micro = true,
    )
    dropdownField(
        "DLNA 媒体服务器",
        {
            music.serverSelection.ifEmpty {
                if (music.serversLoaded) "未发现服务器" else "（尚未扫描）"
            }
        },
        { music.servers }, { music.serverSelection }, { music.serverOpen },
        { music.serverOpen = !music.serverOpen }, { music.selectServer(it) },
        help = "扫描基于 SSDP 组播（约 4 秒）；容器部署收不到组播时需 host 网络部署。",
    )
    actionRow {
        secondaryButton(
            "扫描局域网",
            icon = IconToken.Refresh,
            loading = { music.busy },
        ) {
            music.scan(ctx, row.deviceId) { ok, msg ->
                toast.show(msg, if (ok) "ok" else "error")
            }
        }
    }
    vif({ music.serverSelection.isNotEmpty() }) {
        dropdownField(
            "曲目",
            {
                music.trackSelection.ifEmpty {
                    if (music.tracksLoaded) "没有曲目" else "（尚未浏览）"
                }
            },
            { music.tracks }, { music.trackSelection }, { music.trackOpen },
            { music.trackOpen = !music.trackOpen }, { music.trackSelection = it },
            help = "清单由服务端递归浏览该服务器的音频目录生成；切换服务器后需重新浏览。",
        )
        actionRow {
            secondaryButton(
                "浏览曲目",
                icon = IconToken.Refresh,
                loading = { music.busy },
            ) {
                music.browse(ctx, row.deviceId) { ok, msg ->
                    toast.show(msg, if (ok) "ok" else "error")
                }
            }
            hGap(AdminSpace.sm)
            primaryButton(
                "播放所选曲目",
                enabled = { row.online && music.trackSelection.isNotEmpty() },
                loading = { music.busy },
                loadingText = "排队中…",
                icon = IconToken.Play,
            ) {
                music.play(ctx, row.deviceId) { ok, msg ->
                    toast.show(msg, if (ok) "ok" else "error")
                }
            }
            hGap(AdminSpace.sm)
            secondaryButton(
                "停止并清空缓冲",
                enabled = { row.online },
                loading = { music.busy },
                icon = IconToken.Stop,
            ) {
                music.stop(ctx, row.deviceId) { ok, msg ->
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
