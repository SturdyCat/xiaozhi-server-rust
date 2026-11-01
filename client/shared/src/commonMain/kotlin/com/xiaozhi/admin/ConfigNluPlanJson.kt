package com.xiaozhi.admin

import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive

/** Pure action round-trip shared by the unique form and JVM tests; never a save implementation. */
internal fun nluActionJson(source: String, kind: String, reply: String, plan: String): String {
    val fields = (Json.parseToJsonElement(source) as JsonObject).toMutableMap()
    fields["kind"] = JsonPrimitive(kind)
    fields["reply"] = JsonPrimitive(reply)
    if (kind != "plan") fields.remove("plan")
    else if (nluPlanError(plan).isEmpty()) fields["plan"] = Json.parseToJsonElement(plan)
    // Invalid input keeps the last loaded plan for comparisons only; the save guard rejects it.
    return JsonObject(fields).toString()
}

/** Syntax/shape guard only. Capability names, conditions and argument semantics are server-owned. */
internal fun nluPlanError(raw: String): String {
    if (raw.encodeToByteArray().size > 32768) return "能力组合 JSON 超过 32768 字节，请缩小配置。"
    val value = try { Json.parseToJsonElement(raw) } catch (_: Exception) {
        return "能力组合不是合法 JSON；输入已保留，修正后才能保存。"
    }
    val plan = value as? JsonObject ?: return "能力组合必须是 JSON 对象。"
    val conditions = plan["conditions"]
    if (conditions != null && conditions !is JsonArray) return "条件必须是数组。"
    if (conditions is JsonArray && conditions.size > 16) return "条件最多 16 条。"
    val steps = plan["steps"] as? JsonArray ?: return "请提供步骤数组 steps。"
    if (steps.isEmpty() || steps.size > 16) return "步骤需要 1～16 条，不能只保存空的能力组合。"
    if (steps.any { it !is JsonObject }) return "每个步骤必须是含 api 与 args 的对象。"
    return ""
}
