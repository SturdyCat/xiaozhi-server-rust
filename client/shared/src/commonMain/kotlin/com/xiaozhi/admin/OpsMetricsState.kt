/**
 * 管理台「延迟」页的数据源（`GET /api/metrics`）。
 *
 * 服务端是 P0-5 事件总线的常驻订阅者（`app::metrics`）：会话只管 emit，聚合独立完成——
 * 本类只是这份快照的只读投影。**全内存态、进程重启清零**（口径与服务端 `note` 一致），
 * 页面必须把这点展示出来，否则 p50 看起来像"今日统计"。
 */

package com.xiaozhi.admin

import com.xiaozhi.admin.ui.*

import com.tencent.kuikly.core.base.PagerScope
import com.tencent.kuikly.core.nvi.serialization.json.JSONObject
import com.tencent.kuikly.core.reactive.handler.observable

// ============================================================
// 延迟 / 事件聚合
// ============================================================

/** `GET /api/metrics` 的数据源（事件总线订阅者写入的只读投影）。 */
class MetricsState(scope: PagerScope) : LoadableState(scope) {

    var note by scope.observable("")
    var summary by scope.observable("")

    var uptimeMs by scope.observable(0L)

    // ---- 会话 ----
    var sessionsStarted by scope.observable(0L)
    var sessionsEnded by scope.observable(0L)

    // ---- 整轮延迟 ----
    var turnsTotal by scope.observable(0L)
    var turnP50Ms by scope.observable(0L)
    var turnP95Ms by scope.observable(0L)
    var turnMaxMs by scope.observable(0L)
    var turnSamples by scope.observable(0L)

    // ---- TTS 下行 ----
    var ttsRounds by scope.observable(0L)
    var ttsFrames by scope.observable(0L)
    var ttsAudioMs by scope.observable(0L)
    var ttsInterrupted by scope.observable(0L)

    // ---- 异常 ----
    var errorCount by scope.observable(0L)
    var lastError by scope.observable("")

    override fun endpoint(baseUrl: String): String = "$baseUrl/api/metrics"

    override val errorLabel: String = "延迟数据加载失败"

    override fun apply(data: JSONObject) {
        note = data.text("note")
        uptimeMs = data.optLong("uptime_ms") ?: 0L

        val sessions = data.optJSONObject("sessions")
        sessionsStarted = sessions?.optLong("started") ?: 0L
        sessionsEnded = sessions?.optLong("ended") ?: 0L

        val turns = data.optJSONObject("turns")
        turnsTotal = turns?.optLong("total") ?: 0L
        turnP50Ms = turns?.optLong("elapsed_p50_ms") ?: 0L
        turnP95Ms = turns?.optLong("elapsed_p95_ms") ?: 0L
        turnMaxMs = turns?.optLong("elapsed_max_ms") ?: 0L
        turnSamples = turns?.optLong("samples") ?: 0L

        val tts = data.optJSONObject("tts")
        ttsRounds = tts?.optLong("rounds") ?: 0L
        ttsFrames = tts?.optLong("frames") ?: 0L
        ttsAudioMs = tts?.optLong("audio_ms") ?: 0L
        ttsInterrupted = tts?.optLong("interrupted") ?: 0L

        val errors = data.optJSONObject("errors")
        errorCount = errors?.optLong("count") ?: 0L
        lastError = errors?.text("last") ?: ""

        summary = if (turnsTotal == 0L && sessionsStarted == 0L) {
            "还没有会话事件（唤醒设备说一句话后这里就有数）"
        } else {
            "运行 ${humanDuration(uptimeMs)} · $sessionsStarted 次会话 · $turnsTotal 轮"
        }
        loaded = true
        loadError = ""
    }
}
