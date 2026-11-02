package com.xiaozhi.admin

import com.tencent.kuikly.core.base.PagerScope
import com.tencent.kuikly.core.nvi.serialization.json.JSONArray
import com.tencent.kuikly.core.nvi.serialization.json.JSONObject
import com.xiaozhi.admin.state.stateVar
import com.xiaozhi.admin.state.stateListOf

/** A skill's training corpus and configured action. Templates are data, never a matcher. */
class IntentDraft(scope: PagerScope) {
    var name by stateVar("")
    var templates by stateVar("")
    var actionKind by stateVar("reply")
    var mcpTool by stateVar("")
    var reply by stateVar("")
    var planJson by stateVar("{\"conditions\":[],\"steps\":[]}")
    // Immutable source material preserves extension fields; it is not another editable draft.
    private var sourceIntent = "{}"
    private var sourceAction = "{}"

    fun load(source: JSONObject) {
        sourceIntent = source.toString()
        name = source.optString("name", "")
        templates = nluJoinLines(source.optJSONArray("templates"))
        val action = source.optJSONObject("action") ?: JSONObject()
        sourceAction = action.toString()
        val kind = action.optString("kind", "reply")
        actionKind = if (kind.startsWith("mcp:")) "mcp" else kind
        mcpTool = if (kind.startsWith("mcp:")) kind.removePrefix("mcp:") else ""
        reply = action.optString("reply", "")
        planJson = action.optJSONObject("plan")?.toString() ?: "{\"conditions\":[],\"steps\":[]}"
    }

    fun validationError(): String = if (actionKind == "plan") nluPlanError(planJson) else ""

    fun toJson(): JSONObject = JSONObject(sourceIntent).apply {
        put("name", name.trim())
        put("templates", nluLines(templates))
        val kind = if (actionKind == "mcp") "mcp:${mcpTool.trim()}" else actionKind
        put("action", JSONObject(nluActionJson(sourceAction, kind, reply, planJson)))
    }
}

class SlotDraft(scope: PagerScope) {
    var name by stateVar("")
    var values by stateVar("")
    var optional by stateVar("false")
}

/** The single [nlu] form: real model recognition plus catalogued skill definitions. */
class NluSectionState(private val scope: PagerScope) {
    var enabled by stateVar("true")
    var bert by stateVar(NluBertDraft())
    val model = NluModelState()
    val catalog = NluSkillCatalogState()
    val intents: MutableList<IntentDraft> = stateListOf()
    val slots: MutableList<SlotDraft> = stateListOf()
    val actionOptions: MutableList<Pair<String, String>> = stateListOf()

    init {
        actionOptions.addAll(listOf(
            "plan" to "能力组合（条件与步骤）",
            "reply" to "只应答（兼容已有技能）",
            "end_session" to "结束聆听（兼容已有技能）",
            "mcp" to "设备工具（兼容已有技能）",
        ))
    }

    fun addIntent() { intents.add(IntentDraft(scope)) }
    fun removeIntent(draft: IntentDraft) { intents.remove(draft) }
    fun addSlot() { slots.add(SlotDraft(scope)) }
    fun removeSlot(draft: SlotDraft) { slots.remove(draft) }

    fun validationError(): String {
        for (draft in intents) {
            val error = draft.validationError()
            if (error.isNotEmpty()) return "技能「${draft.name.ifBlank { "未命名" }}」：$error"
        }
        return ""
    }

    fun putJson(root: JSONObject) {
        val intentArr = JSONArray()
        for (draft in intents) intentArr.put(draft.toJson())
        val slotArr = JSONArray()
        for (slot in slots) slotArr.put(JSONObject().apply {
            put("name", slot.name.trim())
            put("values", nluLines(slot.values))
            put("optional", slot.optional.toBooleanStrictOrNull() ?: false)
        })
        root.put("nlu", JSONObject().apply {
            put("enabled", enabled.toBooleanStrictOrNull() ?: true)
            bert.putJson(this)
            put("intents", intentArr)
            put("slots", slotArr)
        })
    }

    fun fill(obj: JSONObject) {
        obj.optJSONObject("nlu")?.let { n ->
            enabled = n.boolText("enabled", enabled, true)
            bert = bert.read(n)
            model.invalidate()
            catalog.invalidate()
            n.optJSONArray("intents")?.let { arr ->
                intents.clear()
                for (i in 0 until arr.length()) {
                    val source = arr.optJSONObject(i) ?: continue
                    val draft = IntentDraft(scope)
                    draft.load(source)
                    // Never silently turn a future/unknown action into a reply action.
                    if (actionOptions.none { it.first == draft.actionKind }) actionOptions.add(draft.actionKind to "其他动作（保留原配置）")
                    intents.add(draft)
                }
            }
            n.optJSONArray("slots")?.let { arr ->
                slots.clear()
                for (i in 0 until arr.length()) {
                    val source = arr.optJSONObject(i) ?: continue
                    val slot = SlotDraft(scope)
                    slot.name = source.optString("name", "")
                    slot.values = nluJoinLines(source.optJSONArray("values"))
                    slot.optional = source.boolText("optional", slot.optional, false)
                    slots.add(slot)
                }
            }
        }
    }
}

private fun nluLines(value: String): JSONArray = JSONArray().apply {
    value.split('\n').map(String::trim).filter(String::isNotEmpty).forEach { put(it) }
}

private fun nluJoinLines(values: JSONArray?): String {
    if (values == null) return ""
    return (0 until values.length()).map { values.optString(it).orEmpty().trim() }.filter(String::isNotEmpty).joinToString("\n")
}
