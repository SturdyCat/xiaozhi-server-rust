package com.xiaozhi.admin

import androidx.compose.runtime.Composable
import androidx.compose.runtime.key
import androidx.compose.runtime.getValue
import androidx.compose.runtime.setValue
import androidx.compose.runtime.remember
import androidx.compose.runtime.mutableStateOf
import com.gearui.components.alert.Alert
import com.gearui.components.alert.AlertStatus
import com.gearui.components.button.Button
import com.gearui.components.button.ButtonType
import com.gearui.foundation.layout.Spacing
import com.gearui.foundation.primitives.Text
import com.gearui.theme.Theme
import com.tencent.kuikly.compose.foundation.layout.Arrangement
import com.tencent.kuikly.compose.foundation.layout.Column
import com.tencent.kuikly.compose.foundation.layout.Row
import com.tencent.kuikly.compose.foundation.layout.fillMaxWidth
import com.tencent.kuikly.compose.foundation.layout.padding
import com.tencent.kuikly.compose.ui.Modifier
import com.tencent.kuikly.compose.ui.unit.dp
import com.xiaozhi.admin.contracts.IconToken
import com.xiaozhi.admin.icons.GearIcon
import com.xiaozhi.admin.kit.HintLine
import com.xiaozhi.admin.kit.LabeledCard
import com.xiaozhi.admin.kit.labeledField
import com.xiaozhi.admin.kit.labeledDecimalField
import com.xiaozhi.admin.kit.labeledSelect
import com.xiaozhi.admin.kit.switchRow
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.contentOrNull

/** Manifest-driven hook editor. All modifications remain in the existing global draft. */
@Composable
fun WorkflowPluginPanel(form: ConfigFormState, meta: PluginMetaState) {
    val state = form.workflow
    val position = form.workflowPosition ?: return
    val pager = LocalAdminPagerOrThrow()
    val loaded = state.metadata
    LabeledCard(title = "${position.label} · 插件管理") {
        if (state.loading) HintLine("正在读取服务端插件清单…")
        if (state.error.isNotEmpty()) {
            Alert(title = "流程管理未就绪", description = state.error, status = AlertStatus.WARNING)
            Button(
                text = "重试读取",
                onClick = { state.loadMetadata(pager) },
                type = ButtonType.TEXT,
                iconWidget = { color -> GearIcon(token = IconToken.Refresh, size = 16.dp, tint = color) },
            )
        }
        if (form.editDeviceId.isNotEmpty()) {
            HintLine("流程结构仅能在全局配置中调整，设备个性化不覆盖插件编排。")
        }
        if (loaded?.mode == "aiui") {
            Alert(title = "当前使用云端托管流程", description = "本地 ASR / NLU / LLM / TTS 增强不代表在云端链路执行。下列编排保存后供本地流程使用。", status = AlertStatus.WARNING)
        }
        if (position.isCore) {
            HintLine("核心引擎由本节点配置选择；扩展插件仅按清单声明的位置挂载，不把核心引擎任意串联。")
        }
        if (state.changeSummary.isNotEmpty()) HintLine("未保存：${state.changeSummary}。使用顶部保存配置统一提交。")
        val rows = state.rows(position.id)
        if (rows.isEmpty()) HintLine("此位置没有已挂载插件，按当前流程继续。")
        for ((index, binding) in rows.withIndex()) {
            key(binding.id) {
                val plugin = loaded?.plugins?.firstOrNull { it.id == binding.pluginId }
                Column(Modifier.fillMaxWidth().padding(vertical = Spacing.sm)) {
                    Text(plugin?.label ?: binding.pluginId, style = Theme.typography.label, color = Theme.colors.foreground)
                    if (form.showAdvanced && state.editable && plugin != null) {
                        switchRow(label = "参与执行", checked = binding.enabled, onToggle = { state.toggle(binding.id, it) })
                        WorkflowBindingConditions(form, binding, plugin)
                        labeledSelect(
                            label = "异常处理", selectedId = binding.onError,
                            options = plugin.onErrorOptions.map { it to failureLabel(it) },
                            onSelect = { state.setFailure(binding.id, it) },
                            help = "仅提供此插件声明的安全失败策略，安全门控不能配置为忽略失败。",
                        )
                        Row(horizontalArrangement = Arrangement.spacedBy(Spacing.xs)) {
                            if (index > 0) Button(
                                text = "上移", onClick = { state.move(binding.id, -1) }, type = ButtonType.TEXT,
                                iconWidget = { color -> GearIcon(token = IconToken.ChevronUp, size = 16.dp, tint = color) },
                            )
                            if (index < rows.lastIndex) Button(
                                text = "下移", onClick = { state.move(binding.id, 1) }, type = ButtonType.TEXT,
                                iconWidget = { color -> GearIcon(token = IconToken.ChevronDown, size = 16.dp, tint = color) },
                            )
                            Button(
                                text = "移除挂载", onClick = { state.remove(binding.id) }, type = ButtonType.TEXT,
                                iconWidget = { color -> GearIcon(token = IconToken.Close, size = 16.dp, tint = color) },
                            )
                        }
                    } else {
                        HintLine(if (binding.enabled) "已挂载；实际执行还取决于功能开关、条件与运行模式" else "挂载已关闭")
                    }
                    if (plugin != null && plugin.ownerPosition.isNotBlank() && plugin.ownerPosition != position.id) {
                        val owner = PipelineWorkflow.forId(plugin.ownerPosition)
                        Button(
                            text = "前往${owner?.label ?: "所属位置"}管理",
                            onClick = { form.selectWorkflowPosition(plugin.ownerPosition) },
                            type = ButtonType.TEXT,
                            iconWidget = { color -> GearIcon(token = IconToken.ChevronRight, size = 16.dp, tint = color) },
                        )
                    }
                    val path = plugin?.configPaths?.firstOrNull()?.substringBefore('.')
                    if (path != null && ConfigSections.all.any { it.id == path }) {
                        Button(
                            text = "打开功能配置",
                            onClick = { form.openConfigSection(path) },
                            type = ButtonType.TEXT,
                            iconWidget = { color -> GearIcon(token = IconToken.Settings, size = 16.dp, tint = color) },
                        )
                    }
                }
            }
        }
        if (form.showAdvanced && state.editable) {
            val available = state.available(position.id)
            if (available.isNotEmpty()) {
                labeledSelect(
                    label = "添加兼容插件", selectedId = "", options = available.map { it.id to it.label },
                    onSelect = { id -> available.firstOrNull { it.id == id }?.let { state.add(it, position.id) } },
                    help = "仅显示当前服务端已注册且允许挂载于此位置的插件；新代码插件需重新构建服务端。",
                )
            }
            HintLine("执行顺序、依赖及条件将在保存时由服务端再次校验；变更对新会话生效。")
        } else if (!form.showAdvanced) {
            HintLine("简洁模式不编辑流程结构。启用高级模式后可添加、启停、排序与配置条件。")
        }
    }
    // Health is visible in either mode and is not evidence of execution in this turn.
    for (node in position.nodes) {
        sectionCapability(meta, node.sectionId)?.let { row ->
            if (row.phase in setOf("failed", "degraded")) {
                LabeledCard(title = "${node.label} · 需要处理") {
                    Alert(title = row.display, description = row.reason.ifBlank { "当前能力未就绪，请检查配置与依赖。" }, status = AlertStatus.WARNING)
                    Button(
                        text = "查看配置",
                        onClick = { form.openConfigSection(node.sectionId) },
                        type = ButtonType.TEXT,
                        iconWidget = { color -> GearIcon(token = IconToken.ChevronRight, size = 16.dp, tint = color) },
                    )
                }
            }
        }
    }
}

@Composable
private fun WorkflowBindingConditions(form: ConfigFormState, row: WorkflowBinding, plugin: WorkflowPluginOption) {
    val state = form.workflow
    val kind = (row.condition["kind"] as? JsonPrimitive)?.contentOrNull ?: "always"
    val options = state.metadata?.conditions.orEmpty().filter { plugin.conditionKinds.isEmpty() || it.kind in plugin.conditionKinds }
    if (options.isEmpty()) {
        HintLine("当前客户端无法编辑此插件条件；原条件会完整保留。")
        return
    }
    labeledSelect(label = "执行条件", selectedId = kind, options = options.map { it.kind to it.label }, onSelect = { state.setConditionKind(row.id, it) })
    if (kind == "confidence") HintLine("当前识别链路不提供可信置信度，此条件不会命中；不会把未知分数当作通过。")
    val selected = options.firstOrNull { it.kind == kind }
    if (selected == null) HintLine("当前条件需更新客户端后编辑，未修改时保留原值。")
    for (field in selected?.fields.orEmpty()) {
        val value = (row.condition[field.key] as? JsonPrimitive)?.contentOrNull.orEmpty()
        when {
            field.type == "boolean" -> switchRow(label = field.label, checked = value == "true", onToggle = { state.setConditionField(row.id, field.key, it.toString()) })
            field.options.isNotEmpty() -> labeledSelect(label = field.label, selectedId = value, options = field.options.map { it to it }, onSelect = { state.setConditionField(row.id, field.key, it) })
            field.type in setOf("number", "float") -> {
                // Keep incomplete lexical input (e.g. "0.") only while its canonical
                // value still matches the authoritative draft; reloads must not show stale text.
                var edit by remember(row.id, kind, field.key) { mutableStateOf<Pair<String, String>?>(null) }
                val input = edit?.takeIf { it.second == value }?.first ?: value
                labeledDecimalField(label = field.label, value = input, onValueChange = { text ->
                    state.setConditionField(row.id, field.key, text)
                    val canonical = (state.bindings.firstOrNull { it.id == row.id }?.condition?.get(field.key) as? JsonPrimitive)?.contentOrNull.orEmpty()
                    edit = text to canonical
                })
                if (input.isNotEmpty() && input.toDoubleOrNull()?.isFinite() != true) HintLine("请填写有效数字；无效输入会由服务端拒绝，不会保存为默认值。")
            }
            else -> labeledField(label = field.label, value = value, onValueChange = { state.setConditionField(row.id, field.key, it) })
        }
    }
}

private fun failureLabel(value: String): String = when (value) {
    "continue" -> "记录原因并继续"
    "stop" -> "停止当前轮次"
    else -> value
}
