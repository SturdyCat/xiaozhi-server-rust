/**
 * 管理台「对话记录」页的数据源（`GET /api/conversations`）。
 *
 * 从 `OpsState.kt` 拆出（`AGENTS.md` §5.10 文件规模约定）。服务端**不持久化**对话：
 * 这里是内存环形缓冲（每会话最多 40 条、最多 32 个会话），进程重启即清空。
 * 因此页面必须把 `truncated`/`dropped` 如实显示出来，否则用户会以为数据丢了。
 */

package com.xiaozhi.admin

import com.tencent.kuikly.core.base.PagerScope
import com.tencent.kuikly.core.nvi.serialization.json.JSONObject
import com.tencent.kuikly.core.pager.Pager
import com.tencent.kuikly.core.reactive.collection.ObservableList
import com.tencent.kuikly.core.reactive.handler.observable
import com.tencent.kuikly.core.reactive.handler.observableList

// ============================================================
// 对话记录
// ============================================================

/** 对话里的一条。 */
class TurnRow(val role: String, val text: String, val elapsedMs: Long?) {
    val isUser: Boolean get() = role == "user"

    /** 助手轮次带整轮耗时（LLM + 合成 + 下行），用来判断"这轮慢不慢"。 */
    val elapsedText: String
        get() = elapsedMs?.let { "整轮 ${it}ms" } ?: ""
}

/** 一条会话的最近对话（内存环形，可能被截断）。 */
class ConversationRow(
    val sessionId: String,
    val deviceId: String,
    val turnCount: Int,
    val truncated: Boolean,
    val dropped: Long,
    val turns: List<TurnRow>,
) {
    /** 副标题：设备 + 条数；被截断时**必须**明示，否则用户会以为这就是完整对话。 */
    val subtitle: String
        get() = buildString {
            append(deviceId.ifEmpty { "(无设备号)" })
            append(" · $turnCount 条")
            if (truncated) append(" · 更早的 $dropped 条已被挤掉")
        }
}

/** `GET /api/conversations` 的数据源（可按设备过滤）。 */
class ConversationsState(private val scope: PagerScope) {

    var loading by scope.observable(false)
    var loaded by scope.observable(false)
    var loadError by scope.observable("")
    var note by scope.observable("")
    var summary by scope.observable("")

    /** 当前过滤的设备号（空串 = 全部）；由页面上的设备下拉设置。 */
    var deviceFilter by scope.observable("")

    /** 过滤输入框的草稿（点「筛选」才生效，避免边打字边发请求）。 */
    var filterDraft by scope.observable("")

    val conversations: ObservableList<ConversationRow> by scope.observableList()

    private var base = ""

    fun load(ctx: Pager, baseUrl: String = "") {
        if (loading) return
        base = baseUrl
        loading = true
        loadError = ""
        val url = buildString {
            append(baseUrl)
            append("/api/conversations?limit=20")
            if (deviceFilter.isNotEmpty()) {
                append("&device_id=")
                append(deviceFilter)
            }
        }
        getJson(ctx, url) { ok, data, message ->
            loading = false
            if (!ok) {
                loadError = "对话记录加载失败: $message"
                return@getJson
            }
            apply(data)
        }
    }

    fun refresh(ctx: Pager) = load(ctx, base)

    /** 切换设备过滤后立即重拉。 */
    fun filterBy(ctx: Pager, deviceId: String) {
        deviceFilter = deviceId
        load(ctx, base)
    }

    private fun apply(data: JSONObject) {
        note = data.text("note")
        conversations.clear()
        val arr = data.array("conversations")
        if (arr == null) {
            loadError = "对话记录为空（服务端返回异常）"
            return
        }
        var turns = 0
        for (i in 0 until arr.length()) {
            val c = arr.optJSONObject(i) ?: continue
            val list = mutableListOf<TurnRow>()
            val raw = c.array("turns")
            if (raw != null) {
                for (j in 0 until raw.length()) {
                    val t = raw.optJSONObject(j) ?: continue
                    list.add(
                        TurnRow(
                            role = t.text("role"),
                            text = t.text("text"),
                            elapsedMs = t.optLong("elapsed_ms"),
                        ),
                    )
                }
            }
            turns += list.size
            conversations.add(
                ConversationRow(
                    sessionId = c.text("session_id"),
                    deviceId = c.text("device_id"),
                    turnCount = c.optInt("turn_count", list.size),
                    truncated = c.optBoolean("truncated", false),
                    dropped = c.optLong("dropped") ?: 0L,
                    turns = list,
                ),
            )
        }
        summary = if (conversations.isEmpty()) {
            "还没有对话记录"
        } else {
            "保留 ${conversations.size} 个会话 · $turns 条对话"
        }
        loaded = true
        loadError = ""
    }
}

