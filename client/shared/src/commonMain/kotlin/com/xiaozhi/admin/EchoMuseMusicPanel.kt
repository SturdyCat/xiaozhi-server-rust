package com.xiaozhi.admin

import androidx.compose.runtime.Composable
import com.gearui.components.button.Button
import com.gearui.components.button.ButtonType
import com.tencent.kuikly.compose.ui.unit.dp
import com.xiaozhi.admin.contracts.IconToken
import com.xiaozhi.admin.icons.GearIcon
import com.xiaozhi.admin.kit.actionRow
import com.xiaozhi.admin.kit.labeledSelect

/**
 * 配置 tab 的 **Stage 08 区块内容**（标题与"为什么官方没有这一组"的说明在
 * `EchoMuseConfigStages.kt::StageMusic`，本文件只画控件）。
 *
 * 音乐面控制（对齐官方 `audio_mix`），音源**仅 DLNA**：服务端扫描/浏览局域网媒体
 * 服务器，选曲后由服务端拉流→解码→经同一下行平面逐帧下发（0x04/0x05，播放中说话
 * 只 `duck` 压低不暂停；「停止」走 `music_flush` 丢弃设备缓冲，不可恢复）。
 *
 * 三个动作（scan/browse/play_url）全部是服务端职责，管理台只调接口、不接触任何
 * DLNA 协议细节。播放/停止需要设备在线（data 链就绪）；扫描/浏览不依赖设备在线。
 * 提示文案由 [EchoMuseMusicState.scan]/[EchoMuseMusicState.browse]/[EchoMuseMusicState.play]/
 * [EchoMuseMusicState.stop] 给出。
 *
 * 本组**不写任何 config 键**，所以「下发配置」对这里无效——音乐是动作面板，不是设置。
 */
@Composable
internal fun EchoMuseMusicStage(row: EchoMuseDeviceRow, state: EchoMuseState, toast: ToastState) {
    val music = state.music
    val pager = LocalAdminPagerOrThrow()
    EmNote(
        "音乐在独立平面与语音混音：说话时自动压低（duck）、说完恢复，不打断播放。" +
            "音源是局域网 DLNA 媒体服务器的曲目——扫描、浏览、拉流解码与推流全部由服务端完成。",
    )
    labeledSelect(
        label = "DLNA 媒体服务器",
        selectedId = music.serverSelection,
        options = music.servers,
        onSelect = { music.selectServer(it) },
        help = "扫描基于 SSDP 组播（约 4 秒）；容器部署收不到组播时需 host 网络部署。",
        placeholder = if (music.serversLoaded) "未发现服务器" else "（尚未扫描）",
    )
    actionRow {
        Button(
            text = "扫描局域网",
            onClick = {
                music.scan(pager, row.deviceId) { ok, msg -> toast.show(msg, if (ok) "ok" else "error") }
            },
            type = ButtonType.OUTLINE,
            loading = music.busy,
            iconWidget = { GearIcon(token = IconToken.Refresh, size = 16.dp) },
        )
    }
    if (music.serverSelection.isNotEmpty()) {
        labeledSelect(
            label = "曲目",
            selectedId = music.trackSelection,
            options = music.tracks,
            onSelect = { music.trackSelection = it },
            help = "清单由服务端递归浏览该服务器的音频目录生成；切换服务器后需重新浏览。",
            placeholder = if (music.tracksLoaded) "没有曲目" else "（尚未浏览）",
        )
        actionRow {
            Button(
                text = "浏览曲目",
                onClick = {
                    music.browse(pager, row.deviceId) { ok, msg -> toast.show(msg, if (ok) "ok" else "error") }
                },
                type = ButtonType.OUTLINE,
                loading = music.busy,
                iconWidget = { GearIcon(token = IconToken.Refresh, size = 16.dp) },
            )
            Button(
                text = if (music.busy) "排队中…" else "播放所选曲目",
                onClick = {
                    music.play(pager, row.deviceId) { ok, msg -> toast.show(msg, if (ok) "ok" else "error") }
                },
                type = ButtonType.FILL,
                loading = music.busy,
                disabled = !(row.online && music.trackSelection.isNotEmpty() && !music.busy),
                iconWidget = { GearIcon(token = IconToken.Play, size = 16.dp) },
            )
            Button(
                text = "停止并清空缓冲",
                onClick = {
                    music.stop(pager, row.deviceId) { ok, msg -> toast.show(msg, if (ok) "ok" else "error") }
                },
                type = ButtonType.OUTLINE,
                loading = music.busy,
                disabled = !row.online,
                iconWidget = { GearIcon(token = IconToken.Stop, size = 16.dp) },
            )
        }
    }
    if (!row.online) {
        EmNote("设备离线：播放/停止需要设备在线（data 链就绪）才能接收音频帧。")
    }
    EmNote(
        "「停止并清空缓冲」不可恢复——设备里已缓冲的音乐会被丢弃，恢复播放请重新点播放；" +
            "语音轮次只会压低音乐，永远不会清缓冲。",
    )
}
