/**
 * 管理台「用量」页的数据源（`GET /api/usage`）。
 *
 * 从 `OpsState.kt` 拆出（`AGENTS.md` §5.10 文件规模约定）。这个端点在服务端早就实现了，
 * 但管理页一直没有入口（客户端全目录 grep `usage` 零命中）——等于死接口，本页把它接出来。
 */

package com.xiaozhi.admin

import com.tencent.kuikly.core.base.PagerScope
import com.tencent.kuikly.core.nvi.serialization.json.JSONObject
import com.tencent.kuikly.core.pager.Pager
import com.tencent.kuikly.core.reactive.collection.ObservableList
import com.tencent.kuikly.core.reactive.handler.observable
import com.tencent.kuikly.core.reactive.handler.observableList

// ============================================================
// 用量
// ============================================================

/** 单个会话的令牌用量（`GET /api/usage` 的 `sessions[]`）。 */
class UsageSessionRow(
    val sessionId: String,
    val turns: Long,
    val input: Long,
    val output: Long,
    val pressure: Long,
    val total: Long,
)

/** `GET /api/usage` 的数据源（把早已存在、但管理页一直没有入口的接口接出来）。 */
class UsageState(private val scope: PagerScope) {

    var loading by scope.observable(false)
    var loaded by scope.observable(false)
    var loadError by scope.observable("")
    var note by scope.observable("")
    var summary by scope.observable("")

    var globalTurns by scope.observable(0L)
    var globalSummary by scope.observable("")
    var globalPressure by scope.observable(0L)

    val sessions: ObservableList<UsageSessionRow> by scope.observableList()

    private var base = ""

    fun load(ctx: Pager, baseUrl: String = "") {
        if (loading) return
        base = baseUrl
        loading = true
        loadError = ""
        getJson(ctx, "$baseUrl/api/usage") { ok, data, message ->
            loading = false
            if (!ok) {
                loadError = "用量数据加载失败: $message"
                return@getJson
            }
            apply(data)
        }
    }

    fun refresh(ctx: Pager) = load(ctx, base)

    private fun apply(data: JSONObject) {
        note = data.text("note")
        sessions.clear()
        val global = data.optJSONObject("global")
        globalTurns = global?.optLong("turns") ?: 0L
        globalPressure = global?.optJSONObject("usage")?.optLong("pressure") ?: 0L
        globalSummary = global?.optJSONObject("usage")?.let { u ->
            UsageTotals(
                turns = global.optLong("turns") ?: 0L,
                input = u.optLong("input") ?: 0L,
                output = u.optLong("output") ?: 0L,
                cacheRead = u.optLong("cache_read") ?: 0L,
                cacheWrite = u.optLong("cache_write") ?: 0L,
                pressure = u.optLong("pressure") ?: 0L,
                total = u.optLong("total") ?: 0L,
            ).summary
        } ?: ""

        val arr = data.array("sessions")
        if (arr != null) {
            for (i in 0 until arr.length()) {
                val s = arr.optJSONObject(i) ?: continue
                val u = s.optJSONObject("usage")
                sessions.add(
                    UsageSessionRow(
                        sessionId = s.text("session_id"),
                        turns = s.optLong("turns") ?: 0L,
                        input = u?.optLong("input") ?: 0L,
                        output = u?.optLong("output") ?: 0L,
                        pressure = u?.optLong("pressure") ?: 0L,
                        total = u?.optLong("total") ?: 0L,
                    ),
                )
            }
        }
        summary = if (globalTurns == 0L) {
            "还没有已计量的轮次"
        } else {
            "已计量 $globalTurns 轮 · 保留 ${sessions.size} 个会话"
        }
        loaded = true
        loadError = ""
    }
}
