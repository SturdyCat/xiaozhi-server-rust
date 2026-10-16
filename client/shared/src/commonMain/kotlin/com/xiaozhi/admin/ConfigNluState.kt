package com.xiaozhi.admin

import com.tencent.kuikly.core.base.PagerScope
import com.tencent.kuikly.core.nvi.serialization.json.JSONArray
import com.tencent.kuikly.core.nvi.serialization.json.JSONObject
import com.tencent.kuikly.core.reactive.collection.ObservableList
import com.tencent.kuikly.core.reactive.handler.observable
import com.tencent.kuikly.core.reactive.handler.observableList

/**
 * 「技能级联」（`[nlu]`）的表单状态。
 *
 * 为什么单独一个类：与 [CommandSectionState] 同一做法——`AGENTS.md` §5.10 文件规模约定，
 * `form.nlu.*` 成组出现，JSON 映射放在本文件的 `putJson` / `fill`，渲染见 `ConfigNluCard.kt`。
 *
 * 语义（服务端为唯一权威，见 `server/src/plugins/skill/mod.rs`）：
 * 识别文本按三级处理——①规则模板命中即本地执行；②规则未命中走本地轻量模型打分，
 * 达 `model_threshold` 且领先 `model_margin` 也直接执行；③都未命中才调云 LLM，
 * LLM 若识别为技能会以 `execute_skill` 工具调用回目录校验执行（槽位/动作白名单由服务端
 * 配置决定），否则正常聊天走 TTS。内置 `end_session` 来自 `[command]` 段，此处定义
 * 带槽位的自定义意图。
 *
 * ⚠️ `intents` / `slots` 是嵌套数组结构，表单里用**可增删的行编辑器**：
 * - intents 每行 = name / templates（多行文本，每行一个模板，`{slot}` 占位）/
 *   action.kind（reply / end_session / mcp）+ mcp 工具名 / reply；
 * - slots 每行 = name / values（多行文本，每行一个封闭值；留空 = 开放槽位）/ optional。
 * ⚠️ 模板/值表数组在读写时统一做多行文本转换（与 `[command].keywords` 同一约定）。
 */

/** 一条自定义意图（`[nlu].intents[i]`）的行内草稿。 */
class IntentDraft(scope: PagerScope) {
    /// 技能名：唯一，LLM 的 `execute_skill` 白名单也用它（`light` 等短名即可）。
    var name by scope.observable("")
    /// 模板列表：每行一个，如「把{room}的灯{action}」；`{slot}` 对应 slots 里同名项。
    var templates by scope.observable("")
    /// 动作：reply（只应答）| end_session（说 reply 后结束聆听）| mcp（派发设备工具）。
    var actionKind by scope.observable("reply")
    /// action.kind = "mcp:" 时派发的设备工具名（须与设备 MCP 工具页清单一致）。
    var mcpTool by scope.observable("")
    /// 命中后的应答语（mcp 技能先应答再派发）。
    var reply by scope.observable("")
    /// action 下拉菜单展开态（每行独立，放草稿里避免共享全局互斥键）。
    var actionMenuOpen by scope.observable(false)
}

/** 一条槽位定义（`[nlu].slots[i]`）的行内草稿。 */
class SlotDraft(scope: PagerScope) {
    /// 槽位名：对应模板里的 `{name}` 占位。
    var name by scope.observable("")
    /// 封闭值表：每行一个（用于校验与抽取）；留空 = 开放槽位（任意短语，仅限长校验）。
    var values by scope.observable("")
    /// true = 模板中该槽位可缺省；false = 缺槽位不命中。
    var optional by scope.observable("false")
}

class NluSectionState(private val scope: PagerScope) {

    var enabled by scope.observable("false")
    /// 轻量模型执行阈值（0~1，越高越保守；1.0 = 关闭模型级）。
    var modelThreshold by scope.observable("0.9")
    /// 最高分领先其他技能最高分的最小差值（防相近指令歧义误触发）。
    var modelMargin by scope.observable("0.1")

    /// 自定义意图行（`fill` 重建；`add/remove` 直接改列表保持响应式）。
    val intents: ObservableList<IntentDraft> by scope.observableList()
    /// 槽位定义行。
    val slots: ObservableList<SlotDraft> by scope.observableList()

    /// action.kind 下拉选项（官方 AlertDialog 要求 ObservableList；全部行共享同一份）。
    val actionOptions: ObservableList<Pair<String, String>> by scope.observableList()

    init {
        actionOptions.addAll(
            listOf(
                "reply" to "reply（只应答，不执行动作）",
                "end_session" to "end_session（说 reply 后结束聆听）",
                "mcp" to "mcp（应答后派发设备工具）",
            ),
        )
    }

    fun addIntent() {
        intents.add(IntentDraft(scope))
    }

    fun removeIntent(draft: IntentDraft) {
        intents.remove(draft)
    }

    fun addSlot() {
        slots.add(SlotDraft(scope))
    }

    fun removeSlot(draft: SlotDraft) {
        slots.remove(draft)
    }

    /** 多行文本 → JSON 数组（每行一项；空行/空白项丢弃）。 */
    private fun lines(s: String): JSONArray {
        val arr = JSONArray()
        s.split("\n").map { it.trim() }.filter { it.isNotEmpty() }.forEach { arr.put(it) }
        return arr
    }

    /** JSON 数组 → 多行文本（空白项丢弃）。 */
    private fun joinLines(arr: JSONArray?): String {
        if (arr == null) return ""
        val out = mutableListOf<String>()
        for (i in 0 until arr.length()) {
            val v = (arr.optString(i) ?: "").trim()
            if (v.isNotEmpty()) out.add(v)
        }
        return out.joinToString("\n")
    }

    /** 单条意图 → `{name, templates[], action:{kind, reply}}`；mcp 拼回 `mcp:<tool>`。 */
    private fun intentJson(d: IntentDraft): JSONObject {
        val kind = if (d.actionKind == "mcp") "mcp:${d.mcpTool.trim()}" else d.actionKind
        return JSONObject().apply {
            put("name", d.name.trim())
            put("templates", lines(d.templates))
            put(
                "action",
                JSONObject().apply {
                    put("kind", kind)
                    put("reply", d.reply)
                },
            )
        }
    }

    /** 单条槽位 → `{name, values[], optional}`。 */
    private fun slotJson(s: SlotDraft): JSONObject = JSONObject().apply {
        put("name", s.name.trim())
        put("values", lines(s.values))
        put("optional", s.optional.toBooleanStrictOrNull() ?: false)
    }

    /**
     * 写入保存请求体（`[nlu]` 整段回传）。
     *
     * 与其它段一样**必须整段回传**：服务端是部分更新语义，前端不回传就等于
     * "这个段在 UI 里不可配置"（旧客户端漏传整段曾被静默重置过）。
     */
    fun putJson(root: JSONObject) {
        val intentArr = JSONArray()
        for (d in intents) intentArr.put(intentJson(d))
        val slotArr = JSONArray()
        for (s in slots) slotArr.put(slotJson(s))
        root.put(
            "nlu",
            JSONObject().apply {
                put("enabled", enabled.toBooleanStrictOrNull() ?: false)
                // 留空/非法 = 回落服务端默认；服务端会在刷新时做范围校验（0~1）。
                put("model_threshold", modelThreshold.trim().toDoubleOrNull() ?: 0.9)
                put("model_margin", modelMargin.trim().toDoubleOrNull() ?: 0.1)
                put("intents", intentArr)
                put("slots", slotArr)
            },
        )
    }

    /** 用服务端 `/api/config` 回包填充（缺键保留当前草稿；数组如实重建）。 */
    fun fill(obj: JSONObject) {
        obj.optJSONObject("nlu")?.let { n ->
            enabled = n.optBoolean("enabled", enabled.toBooleanStrictOrNull() ?: false).toString()
            modelThreshold = n.optDouble("model_threshold", modelThreshold.trim().toDoubleOrNull() ?: 0.9).toString()
            modelMargin = n.optDouble("model_margin", modelMargin.trim().toDoubleOrNull() ?: 0.1).toString()
            n.optJSONArray("intents")?.let { arr ->
                intents.clear()
                for (i in 0 until arr.length()) {
                    val o = arr.optJSONObject(i) ?: continue
                    val a = o.optJSONObject("action")
                    val kind = a?.optString("kind", "reply") ?: "reply"
                    val d = IntentDraft(scope)
                    d.name = o.optString("name", "")
                    d.templates = joinLines(o.optJSONArray("templates"))
                    if (kind.startsWith("mcp:")) {
                        d.actionKind = "mcp"
                        d.mcpTool = kind.removePrefix("mcp:")
                    } else {
                        d.actionKind = if (kind == "end_session") "end_session" else "reply"
                    }
                    d.reply = a?.optString("reply", "") ?: ""
                    intents.add(d)
                }
            }
            n.optJSONArray("slots")?.let { arr ->
                slots.clear()
                for (i in 0 until arr.length()) {
                    val o = arr.optJSONObject(i) ?: continue
                    val s = SlotDraft(scope)
                    s.name = o.optString("name", "")
                    s.values = joinLines(o.optJSONArray("values"))
                    s.optional = o.optBoolean("optional", false).toString()
                    slots.add(s)
                }
            }
        }
    }
}
