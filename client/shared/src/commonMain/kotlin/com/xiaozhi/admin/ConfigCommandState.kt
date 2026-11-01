package com.xiaozhi.admin

import com.tencent.kuikly.core.base.PagerScope
import com.tencent.kuikly.core.nvi.serialization.json.JSONArray
import com.tencent.kuikly.core.nvi.serialization.json.JSONObject
import com.xiaozhi.admin.state.stateVar

/** end_session catalog/training configuration. Deprecated matching fields are not editable. */
class CommandSectionState(scope: PagerScope) {
    var enabled by stateVar("true")
    var keywords by stateVar("退下\n闭嘴\n关闭")
    var reply by stateVar("好的，我先退下了。")

    fun putJson(root: JSONObject) {
        root.put("command", JSONObject().apply {
            put("enabled", enabled.toBooleanStrictOrNull() ?: true)
            put("keywords", JSONArray().apply {
                keywords.split('\n').map(String::trim).filter(String::isNotEmpty).forEach { put(it) }
            })
            put("reply", reply)
        })
        // Partial-update merge retains legacy match_mode/max_chars without inventing controls.
    }

    fun fill(obj: JSONObject) {
        obj.optJSONObject("command")?.let { command ->
            enabled = command.boolText("enabled", enabled, true)
            command.optJSONArray("keywords")?.let { values ->
                keywords = (0 until values.length()).map { values.optString(it).orEmpty().trim() }
                    .filter(String::isNotEmpty).joinToString("\n")
            }
            reply = command.optString("reply", reply)
        }
    }
}
