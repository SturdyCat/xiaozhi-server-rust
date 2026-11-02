package com.xiaozhi.admin

import kotlinx.serialization.json.*

/** Wire-only workflow projection. No UI/platform dependencies and no executable expressions. */
data class WorkflowBinding(
    val id: String,
    val pluginId: String,
    val positionId: String,
    val enabled: Boolean,
    val order: Int,
    val condition: JsonObject = buildJsonObject { put("kind", "always") },
    val onError: String = "continue",
) {
    fun toJson(): JsonObject = buildJsonObject {
        put("id", id); put("plugin_id", pluginId); put("position_id", positionId)
        put("enabled", enabled); put("order", order); put("condition", condition); put("on_error", onError)
    }
}

data class WorkflowPluginOption(
    val id: String,
    val label: String,
    val ownerPosition: String,
    val allowedPositions: List<String>,
    val configPaths: List<String>,
    val repeatable: Boolean,
    val onErrorOptions: List<String>,
    val conditionKinds: List<String>,
)

data class WorkflowConditionField(val key: String, val label: String, val type: String, val options: List<String>, val required: Boolean = false)
data class WorkflowConditionOption(val kind: String, val label: String, val fields: List<WorkflowConditionField>)
data class WorkflowMetadata(
    val version: Int,
    val revision: String,
    val mode: String,
    val plugins: List<WorkflowPluginOption>,
    val conditions: List<WorkflowConditionOption>,
    val bindings: List<WorkflowBinding>,
)

private fun JsonObject.text(key: String, fallback: String = "") = (get(key) as? JsonPrimitive)?.contentOrNull ?: fallback
private fun JsonObject.strings(key: String): List<String> = (get(key) as? JsonArray).orEmpty().mapNotNull {
    (it as? JsonPrimitive)?.contentOrNull
}

fun parseWorkflowBindings(config: JsonObject): List<WorkflowBinding> {
    require((config["version"] as? JsonPrimitive)?.intOrNull == 1) { "不支持的工作流版本，请升级客户端" }
    val rows = config["bindings"] as? JsonArray ?: error("工作流缺少挂载清单")
    val bindings = rows.map { value ->
        val row = value as? JsonObject ?: error("工作流挂载格式错误")
        WorkflowBinding(
            id = row.text("id"), pluginId = row.text("plugin_id"), positionId = row.text("position_id"),
            enabled = (row["enabled"] as? JsonPrimitive)?.booleanOrNull ?: true,
            order = (row["order"] as? JsonPrimitive)?.intOrNull ?: 0,
            condition = if ("condition" !in row) buildJsonObject { put("kind", "always") }
                else row["condition"] as? JsonObject ?: error("工作流条件格式错误，已停止编辑以保留原配置"),
            onError = row.text("on_error", "continue"),
        ).also { require(it.id.isNotBlank() && it.pluginId.isNotBlank() && it.positionId.isNotBlank()) { "挂载标识不能为空" } }
    }
    require(bindings.map { it.id }.distinct().size == bindings.size) { "工作流挂载标识重复" }
    return bindings
}

fun workflowConfigJson(bindings: List<WorkflowBinding>): JsonObject = buildJsonObject {
    put("version", 1); put("bindings", JsonArray(bindings.map { it.toJson() }))
}

fun parseWorkflowMetadata(text: String): WorkflowMetadata {
    val root = Json.parseToJsonElement(text) as? JsonObject ?: error("工作流响应格式错误")
    val version = (root["version"] as? JsonPrimitive)?.intOrNull ?: 0
    require(version == 1) { "不支持的工作流版本，请升级客户端" }
    val config = root["config"] as? JsonObject ?: error("工作流响应缺少配置")
    val plugins = (root["plugins"] as? JsonArray).orEmpty().map { value ->
        val row = value as? JsonObject ?: error("插件声明格式错误")
        WorkflowPluginOption(
            id = row.text("id"), label = row.text("label", row.text("id")), ownerPosition = row.text("owner_position"),
            allowedPositions = row.strings("allowed_positions"), configPaths = row.strings("config_paths"),
            repeatable = (row["repeatable"] as? JsonPrimitive)?.booleanOrNull ?: false,
            onErrorOptions = row.strings("on_error_options").ifEmpty { listOf("continue") },
            conditionKinds = row.strings("condition_kinds"),
        )
    }
    val conditions = (root["condition_kinds"] as? JsonArray).orEmpty().mapNotNull { value ->
        val row = value as? JsonObject ?: return@mapNotNull null
        val fields = (row["fields"] as? JsonArray).orEmpty().mapNotNull { field ->
            val f = field as? JsonObject ?: return@mapNotNull null
            WorkflowConditionField(f.text("key"), f.text("label", f.text("key")), f.text("type", "string"), f.strings("options"), (f["required"] as? JsonPrimitive)?.booleanOrNull ?: false)
        }
        WorkflowConditionOption(row.text("kind"), row.text("label", row.text("kind")), fields)
    }
    return WorkflowMetadata(version, root.text("revision"), root.text("mode", "local"), plugins, conditions, parseWorkflowBindings(config))
}

/** Reordering is scoped to one hook; dependencies are authoritatively checked on save. */
fun moveWorkflowBinding(bindings: List<WorkflowBinding>, id: String, direction: Int): List<WorkflowBinding> {
    val selected = bindings.firstOrNull { it.id == id } ?: return bindings
    val peers = bindings.filter { it.positionId == selected.positionId }.sortedWith(compareBy<WorkflowBinding> { it.order }.thenBy { it.id }).toMutableList()
    val from = peers.indexOfFirst { it.id == id }
    val to = from + direction
    if (to !in peers.indices || direction !in listOf(-1, 1)) return bindings
    val other = peers[to]; peers[to] = peers[from]; peers[from] = other
    val ranks = peers.mapIndexed { index, binding -> binding.id to index * 10 }.toMap()
    return bindings.map { binding -> ranks[binding.id]?.let { binding.copy(order = it) } ?: binding }
}

fun workflowChangeSummary(baseline: List<WorkflowBinding>, current: List<WorkflowBinding>): String {
    if (baseline == current) return ""
    val added = current.count { row -> baseline.none { it.id == row.id } }
    val removed = baseline.count { row -> current.none { it.id == row.id } }
    val changed = current.count { row -> baseline.any { it.id == row.id && it != row } }
    return listOfNotNull(
        added.takeIf { it > 0 }?.let { "新增 $it 项" },
        removed.takeIf { it > 0 }?.let { "移除 $it 项" },
        changed.takeIf { it > 0 }?.let { "调整 $it 项" },
    ).joinToString("，")
}
