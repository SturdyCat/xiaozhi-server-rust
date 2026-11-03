package com.xiaozhi.admin

import com.tencent.kuikly.core.base.PagerScope
import com.tencent.kuikly.core.nvi.serialization.json.JSONArray
import com.tencent.kuikly.core.nvi.serialization.json.JSONObject
import com.xiaozhi.admin.state.stateVar
import com.xiaozhi.admin.state.stateListOf

/** 控制域草稿取值白名单；与服务端 `Domain` 枚举（device/server/iot）一一对应。 */
val NLU_DOMAIN_IDS: List<String> = listOf("device", "server", "iot")

/** A skill's training corpus and configured action. Templates are data, never a matcher. */
class IntentDraft(scope: PagerScope) {
    var name by stateVar("")
    /// 控制域（device/server/iot）：只影响分组展示与文档，不进训练输入。
    var domain by stateVar("server")
    var templates by stateVar("")
    var actionKind by stateVar("reply")
    var mcpTool by stateVar("")
    var reply by stateVar("")
    var planJson by stateVar("{\"conditions\":[],\"steps\":[]}")
    /// 加载时的保存态技能名（训练标签基线）：用于检测"改名 → 旧模型标签失效"。
    var sourceName by stateVar("")
    // Immutable source material preserves extension fields; it is not another editable draft.
    private var sourceIntent = "{}"
    private var sourceAction = "{}"

    fun load(source: JSONObject) {
        sourceIntent = source.toString()
        name = source.optString("name", "")
        sourceName = name
        domain = when (val raw = source.optString("domain", "server")) {
            in NLU_DOMAIN_IDS -> raw
            else -> "server" // 旧服务端/旧保存态缺 domain 时缺省服务端域，与服务端缺省规则一致。
        }
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

    /** 技能名 = 训练标签；与保存态基线不同即视为改名（旧模型标签失效，需重新训练）。 */
    fun isRenamed(): Boolean = sourceName.isNotEmpty() && name.trim() != sourceName

    fun toJson(): JSONObject = JSONObject(sourceIntent).apply {
        put("name", name.trim())
        put("domain", if (domain in NLU_DOMAIN_IDS) domain else "server")
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

    /** 控制域选项：按执行者分域（设备=固件执行 / 服务器=服务端执行 / IoT=第三方设备 · tuya）。 */
    val domainOptions: List<Pair<String, String>> = listOf(
        "server" to "服务器域（服务端执行）",
        "device" to "设备域（固件执行）",
        "iot" to "IoT 域（第三方设备 · tuya）",
    )

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

    /**
     * 相对保存态被改名的技能（训练标签变更）。改名不拦截保存，但保存后旧模型标签失效，
     * 必须重新训练并验收新模型——调用方用这条文案做强提示（P0-4）；无改名时返回空串。
     */
    fun renamedSkillsNote(): String =
        intents.filter { it.isRenamed() }
            .joinToString("、") { "「${it.sourceName}」→「${it.name.trim().ifEmpty { "未命名" }}」" }

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
