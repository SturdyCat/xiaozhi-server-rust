package com.xiaozhi.admin

import com.tencent.kuikly.core.nvi.serialization.json.JSONObject
import com.tencent.kuikly.core.pager.Pager
import com.xiaozhi.admin.state.stateVar
import kotlinx.serialization.json.*

/** Sub-controller of the single ConfigFormState draft; never submits configuration itself. */
class WorkflowEditorState(private val form: ConfigFormState) {
    var metadata: WorkflowMetadata? by stateVar(null)
        private set
    var bindings: List<WorkflowBinding> by stateVar(emptyList())
        private set
    var error by stateVar("")
        private set
    var loading by stateVar(false)
        private set
    var dirty by stateVar(false)
        private set
    private var baseline: List<WorkflowBinding> = emptyList()
    private var generation = 0L
    private var serial = 0
    private var configSeen = false
    private var base = ""
    val changeSummary: String get() = workflowChangeSummary(baseline, bindings)
    val customized: Boolean get() = configSeen || dirty
    val editable: Boolean get() = metadata != null && error.isEmpty() && form.editDeviceId.isEmpty()

    fun acceptConfig(config: JSONObject?) {
        generation++
        metadata = null
        loading = false
        error = ""
        dirty = false
        configSeen = config != null
        val parsed = if (config == null) emptyList() else try {
            parseWorkflowBindings(Json.parseToJsonElement(config.toString()) as JsonObject)
        } catch (e: Exception) {
            error = e.message ?: "工作流配置不可读，请升级客户端"
            emptyList()
        }
        bindings = parsed
        baseline = parsed
    }

    /** Metadata is revision-bound. It must not replace a loaded/edited configuration snapshot. */
    fun loadMetadata(ctx: Pager, baseUrl: String = form.serverBase) {
        val ticket = ++generation
        val access = ctx.adminAccessRevision()
        val token = ctx.adminRequestToken()
        val revision = form.revision
        base = baseUrl
        loading = true
        network(ctx).requestGet("$baseUrl/api/workflow", JSONObject()) { data, success, message, _ ->
            if (ticket != generation || base != baseUrl || form.serverBase != baseUrl || !ctx.isAdminAccessCurrent(access, token)) return@requestGet
            if (revision != form.revision) {
                loadMetadata(ctx, baseUrl)
                return@requestGet
            }
            loading = false
            if (!success) {
                metadata = null
                error = "工作流管理暂不可用：${message.ifBlank { "服务端可能尚未升级" }}。现有节点配置仍可使用。"
                return@requestGet
            }
            try {
                val next = parseWorkflowMetadata(data.toString())
                if (revision.isNotEmpty() && next.revision != revision) {
                    error = "流程配置已在其他窗口变更，请重新加载配置；当前草稿仍保留。"
                    metadata = null
                    return@requestGet
                }
                metadata = next
                if (!configSeen && !dirty) {
                    bindings = next.bindings
                    baseline = next.bindings
                }
                error = ""
            } catch (e: Exception) {
                metadata = null
                error = e.message ?: "无法读取工作流，客户端版本可能过旧"
            }
        }
    }

    fun putPatch(body: JSONObject) {
        if (dirty) body.put("workflow", JSONObject(workflowConfigJson(bindings).toString()))
    }

    fun markSaved(saved: List<WorkflowBinding>) {
        baseline = saved
        configSeen = true
        dirty = bindings != saved
        if (dirty) form.markDirty()
    }

    private fun replace(next: List<WorkflowBinding>) {
        if (!editable || next == bindings) return
        bindings = next
        dirty = next != baseline
        form.markDirty()
    }

    fun rows(positionId: String): List<WorkflowBinding> = bindings.filter { it.positionId == positionId }
        .sortedWith(compareBy<WorkflowBinding> { it.order }.thenBy { it.id })

    fun available(positionId: String): List<WorkflowPluginOption> = metadata?.plugins.orEmpty().filter { plugin ->
        positionId in plugin.allowedPositions && (plugin.repeatable || bindings.none { it.pluginId == plugin.id })
    }

    fun add(plugin: WorkflowPluginOption, positionId: String) {
        if (!editable || available(positionId).none { it.id == plugin.id }) return
        var id: String
        do { id = "${plugin.id}.custom.${++serial}" } while (bindings.any { it.id == id })
        replace(bindings + WorkflowBinding(id, plugin.id, positionId, true, (rows(positionId).maxOfOrNull { it.order } ?: -10) + 10,
            onError = plugin.onErrorOptions.firstOrNull() ?: "continue"))
    }

    fun remove(id: String) = replace(bindings.filterNot { it.id == id })
    fun toggle(id: String, enabled: Boolean) = replace(bindings.map { if (it.id == id) it.copy(enabled = enabled) else it })
    fun move(id: String, direction: Int) = replace(moveWorkflowBinding(bindings, id, direction))
    fun setFailure(id: String, value: String) {
        val row = bindings.firstOrNull { it.id == id } ?: return
        if (metadata?.plugins?.firstOrNull { it.id == row.pluginId }?.onErrorOptions?.contains(value) != true) return
        replace(bindings.map { if (it.id == id) it.copy(onError = value) else it })
    }

    fun setConditionKind(id: String, kind: String) {
        val row = bindings.firstOrNull { it.id == id } ?: return
        val plugin = metadata?.plugins?.firstOrNull { it.id == row.pluginId } ?: return
        if (plugin.conditionKinds.isNotEmpty() && kind !in plugin.conditionKinds) return
        val option = metadata?.conditions?.firstOrNull { it.kind == kind } ?: return
        val condition = buildJsonObject {
            put("kind", kind)
            for (field in option.fields.filter { it.required }) {
                when (field.type) {
                    "number", "float" -> put(field.key, 0.0)
                    "boolean" -> put(field.key, false)
                    else -> put(field.key, field.options.firstOrNull() ?: "")
                }
            }
        }
        replace(bindings.map { if (it.id == id) it.copy(condition = condition) else it })
    }

    fun setConditionField(id: String, key: String, value: String) {
        val row = bindings.firstOrNull { it.id == id } ?: return
        val kind = (row.condition["kind"] as? JsonPrimitive)?.content ?: "always"
        val field = metadata?.conditions?.firstOrNull { it.kind == kind }?.fields?.firstOrNull { it.key == key } ?: return
        val element = when {
            value.isEmpty() && !field.required -> null
            field.type in setOf("number", "float") -> value.toDoubleOrNull()?.takeIf { it.isFinite() }?.let { JsonPrimitive(it) } ?: JsonPrimitive(value)
            field.type == "boolean" -> JsonPrimitive(value.toBooleanStrictOrNull() ?: return)
            else -> JsonPrimitive(value)
        }
        val condition = JsonObject(if (element == null) row.condition - key else row.condition + (key to element))
        replace(bindings.map { if (it.id == id) it.copy(condition = condition) else it })
    }
}
