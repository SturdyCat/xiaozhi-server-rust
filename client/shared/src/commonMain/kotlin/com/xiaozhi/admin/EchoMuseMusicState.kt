package com.xiaozhi.admin

import com.xiaozhi.admin.ui.*

import com.tencent.kuikly.core.base.PagerScope
import com.tencent.kuikly.core.nvi.serialization.json.JSONObject
import com.tencent.kuikly.core.pager.Pager
import com.tencent.kuikly.core.reactive.collection.ObservableList
import com.tencent.kuikly.core.reactive.handler.observable
import com.tencent.kuikly.core.reactive.handler.observableList

/**
 * 音乐面状态（配置 tab 的 **Stage 08**）：局域网 DLNA 服务器的扫描/浏览/播放/停止。
 *
 * 音源**仅 DLNA**：SSDP 发现、设备描述解析、SOAP 递归浏览、拉流解码、逐帧推流全部在
 * 服务端完成——管理台只「选服务器 → 浏览 → 选曲目 → 点播放」，不接触任何 DLNA 协议细节。
 * 播放中说话只会 `duck` 压低，不暂停；「停止」走 `music_flush` 丢弃设备缓冲（不可恢复）。
 *
 * 为什么单独成一个状态对象（而不是留在 [EchoMuseState] 里）：`EchoMuseState.kt` 已到
 * 600 行上限，而音乐面是**自成一体的写动作集合**（四个动作 + 两层下拉 + 两张清单），
 * 与列表/审批/配置下发没有共享状态。
 *
 * ⚠️ 刻意**不共用** [EchoMuseState.actionBusy]：那个锁保护的是"同一台设备上的管理写动作"，
 * 音乐是服务端侧的流媒体动作（不占设备 shell 槽），混用只会让「扫描 DLNA」把「保存配置」
 * 按钮卡成 loading。基址/令牌用 lambda 取快照，切服务器后旧回包按 `base()` 变化丢弃。
 */
class EchoMuseMusicState(
    private val scope: PagerScope,
    private val base: () -> String,
    private val token: () -> String,
) {

    /** 局域网 DLNA 媒体服务器（value=location 设备描述地址，label=友好名）。 */
    var servers by scope.observableList<Pair<String, String>>()
        private set
    var serversLoaded by scope.observable(false)
        private set
    var serverOpen by scope.observable(false)

    /** 已选服务器（location；非空时展示曲目区）。 */
    var serverSelection by scope.observable("")

    /** 当前服务器的音频曲目（value=直链 url，label=标题（艺术家 · 专辑））。 */
    var tracks by scope.observableList<Pair<String, String>>()
        private set
    var tracksLoaded by scope.observable(false)
        private set
    var trackOpen by scope.observable(false)

    /** 已选曲目（直链 url）。 */
    var trackSelection by scope.observable("")

    /** 任一音乐动作进行中（按钮 loading 用）。 */
    var busy by scope.observable(false)
        private set

    /** 选中服务器（下拉 onSelect）：切换服务器必须清空上一台的曲目，避免串台。 */
    fun selectServer(location: String) {
        serverSelection = location
        tracks.clear()
        tracksLoaded = false
        trackSelection = ""
    }

    /** 可播放：设备在线 + 已选曲目（data 链就绪由服务端判定，这里只挡明显不成立的情况）。 */
    fun canPlay(online: Boolean): Boolean = online && trackSelection.isNotEmpty() && !busy

    /**
     * 扫描局域网 DLNA 媒体服务器（POST music {action:scan}）。SSDP 发现与设备描述解析都在
     * 服务端完成（约 4s 窗口），不依赖设备在线；容器部署收不到组播时服务端回包带 hint，
     * 原样透出给用户（"没发现服务器"与"组播被拦"是两件事）。
     */
    fun scan(ctx: Pager, deviceId: String, onDone: (ok: Boolean, msg: String) -> Unit) {
        if (busy) return
        busy = true
        post(ctx, deviceId, JSONObject().apply { put("action", "scan") }) { ok, _, reason, data ->
            busy = false
            if (!ok) {
                onDone(false, "扫描失败：${reason.ifEmpty { "HTTP 异常" }}")
                return@post
            }
            val found = data?.array("servers")?.let { arr ->
                (0 until arr.length()).mapNotNull { i ->
                    val e = arr.optJSONObject(i) ?: return@mapNotNull null
                    val location = e.text("location")
                    if (location.isEmpty()) null else location to e.text("name")
                }
            } ?: emptyList()
            servers.clear()
            servers.addAll(found)
            serversLoaded = true
            if (found.isEmpty()) {
                onDone(true, data?.text("hint")?.takeIf { it.isNotEmpty() } ?: "局域网未发现 DLNA 媒体服务器")
            } else {
                onDone(true, "发现 ${found.size} 台 DLNA 媒体服务器，请在下拉中选择")
            }
        }
    }

    /** 浏览已选服务器的音频曲目（POST music {action:browse}；SOAP 递归在服务端，大库可能数秒）。 */
    fun browse(ctx: Pager, deviceId: String, onDone: (ok: Boolean, msg: String) -> Unit) {
        val location = serverSelection
        if (location.isEmpty()) {
            onDone(false, "请先扫描并选择一台 DLNA 媒体服务器")
            return
        }
        if (busy) return
        busy = true
        post(ctx, deviceId, JSONObject().apply {
            put("action", "browse")
            put("location", location)
        }) { ok, _, reason, data ->
            busy = false
            if (!ok) {
                onDone(false, "浏览失败：${reason.ifEmpty { "HTTP 异常" }}")
                return@post
            }
            val found = data?.array("tracks")?.let { arr ->
                (0 until arr.length()).mapNotNull { i ->
                    val e = arr.optJSONObject(i) ?: return@mapNotNull null
                    val url = e.text("url")
                    if (url.isEmpty()) return@mapNotNull null
                    val title = e.text("title").ifEmpty { url }
                    val sub = listOf(e.text("artist"), e.text("album")).filter { it.isNotEmpty() }.joinToString(" · ")
                    url to if (sub.isEmpty()) title else "$title（$sub）"
                }
            } ?: emptyList()
            tracks.clear()
            tracks.addAll(found)
            tracksLoaded = true
            if (found.isEmpty()) {
                onDone(true, "该服务器没有列出音频曲目")
            } else {
                onDone(true, "已列出 ${found.size} 个曲目，请选择播放")
            }
        }
    }

    /** 播放所选曲目（POST music {action:play_url}；服务端拉流→解码→复用音乐通道推流）。 */
    fun play(ctx: Pager, deviceId: String, onDone: (ok: Boolean, msg: String) -> Unit) {
        val url = trackSelection
        if (url.isEmpty()) {
            onDone(false, "请先浏览并选择要播放的曲目")
            return
        }
        if (busy) return
        busy = true
        post(ctx, deviceId, JSONObject().apply {
            put("action", "play_url")
            put("url", url)
        }) { ok, code, reason, _ ->
            busy = false
            if (ok) {
                onDone(true, "已排队播放：服务端正在拉流解码并推送到设备，说话会自动压低音乐")
            } else {
                onDone(false, when (code) {
                    409 -> "设备离线或 data 链未就绪，播放未开始"
                    400 -> "播放失败：${reason.ifEmpty { "曲目不可用" }}"
                    else -> "播放失败：${reason.ifEmpty { "HTTP $code" }}"
                })
            }
        }
    }

    /** 停止音乐并丢弃设备缓冲（POST music {action:stop}；不可恢复，恢复播放请重新点播放）。 */
    fun stop(ctx: Pager, deviceId: String, onDone: (ok: Boolean, msg: String) -> Unit) {
        if (busy) return
        busy = true
        post(ctx, deviceId, JSONObject().apply { put("action", "stop") }) { ok, code, reason, _ ->
            busy = false
            if (ok) {
                onDone(true, "已停止播放并清空设备音乐缓冲")
            } else {
                onDone(false, when (code) {
                    409 -> "设备离线，无需停止"
                    else -> "停止失败：${reason.ifEmpty { "HTTP $code" }}"
                })
            }
        }
    }

    /**
     * 音乐面唯一的 HTTP 出口。基址在发送时取快照：回包前若已切服务器（`base()` 变了），
     * 结果直接丢弃——否则旧服务器的曲目清单会被当成本服务器设备的。
     */
    private fun post(
        ctx: Pager,
        deviceId: String,
        body: JSONObject,
        onDone: (ok: Boolean, code: Int?, reason: String, data: JSONObject?) -> Unit,
    ) {
        val origin = base()
        val url = "$origin/api/echomuse/devices/${echoMuseUrlComponent(deviceId)}/music"
        network(ctx).httpRequest(url, true, body, echoMuseHeaders(token())) { data, success, errorMsg, resp ->
            if (base() != origin) return@httpRequest
            val code = resp.statusCode
            val ok = success && (code == null || code in 200..299)
            val reason = data.text("error").takeIf { it.isNotEmpty() }
                ?: data.text("message").takeIf { it.isNotEmpty() }
                ?: errorMsg
            onDone(ok, code, reason, data)
        }
    }
}
