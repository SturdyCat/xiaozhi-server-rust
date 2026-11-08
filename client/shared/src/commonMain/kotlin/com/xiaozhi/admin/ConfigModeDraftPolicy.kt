package com.xiaozhi.admin

import androidx.compose.runtime.Composable
import com.gearui.components.alert.Alert
import com.gearui.components.alert.AlertStatus
import com.gearui.components.dialog.ConfirmDialog
import com.tencent.kuikly.core.nvi.serialization.json.JSONObject
import com.xiaozhi.admin.kit.HintLine
import com.xiaozhi.admin.state.stateVar
import kotlinx.serialization.json.*

/** Only compares the existing form draft. Never owns field values, submits, or restores defaults. */
class ConfigModeDraftPolicy(private val form: ConfigFormState) {
    private var baseline: JsonObject? = null
    private var approvedOnce = ""
    private var pendingFingerprint = ""
    private var pendingAction: (() -> Unit)? = null
    var pendingSummary by stateVar("")
        private set
    private var baselineVersion by stateVar(0)

    fun capture(body: JSONObject) {
        baseline = Json.parseToJsonElement(body.toString()) as JsonObject
        baselineVersion++
        cancel()
    }

    /** Capture exactly the request that succeeded, not edits made while it was in flight. */
    fun saved(body: JSONObject) = capture(body)

    fun hiddenSummary(body: JSONObject = form.buildConfigDraft()): String {
        baselineVersion // observe baseline replacement
        val previous = baseline ?: return ""
        val current = Json.parseToJsonElement(body.toString()) as JsonObject
        val paths = changedConfigPaths(previous, current).filter { path ->
            path != "expected_revision" && !path.startsWith("workflow") && (form.schema.version == 0 || !form.schema.advancedCoverageComplete ||
                form.schema.fieldAdvanced(path.substringBeforeLast('.', ""), path.substringAfterLast('.')))
        } + if (form.workflow.dirty) listOf("workflow.bindings") else emptyList()
        if (paths.isEmpty()) return ""
        val sections = paths.map { it.substringBefore('.') }.distinct().map { section ->
            if (section == "workflow") "流程编排" else ConfigSections.all.firstOrNull { it.id == section }?.label ?: "扩展配置"
        }.distinct()
        return "${sections.joinToString("、")}（${paths.size} 项）"
    }

    /** Same body must be explicitly confirmed; changes while the dialog is open require re-confirmation. */
    fun allowSave(body: JSONObject, onConfirmed: () -> Unit): Boolean {
        val fingerprint = body.toString()
        if (approvedOnce == fingerprint) {
            approvedOnce = ""
            return true
        }
        approvedOnce = ""
        if (form.showAdvanced) return true
        val summary = hiddenSummary(body)
        if (summary.isEmpty()) return true
        pendingFingerprint = fingerprint
        pendingAction = onConfirmed
        pendingSummary = summary
        return false
    }

    fun confirm() {
        val action = pendingAction ?: return
        approvedOnce = pendingFingerprint
        pendingAction = null
        pendingSummary = ""
        pendingFingerprint = ""
        action()
        approvedOnce = ""
    }

    fun cancel() {
        pendingAction = null
        pendingSummary = ""
        pendingFingerprint = ""
        approvedOnce = ""
    }
}

@Composable
fun ConfigModeSaveConfirmation(form: ConfigFormState) {
    ConfirmDialog(
        visible = form.modeDraft.pendingSummary.isNotEmpty(),
        title = "保存包含高级配置的改动",
        message = "本次保存还包含当前简洁模式中未展开的改动：${form.modeDraft.pendingSummary}。确认后会连同当前配置一起保存；取消不会丢弃草稿。",
        confirmText = "确认保存", cancelText = "保留草稿",
        onConfirm = { form.modeDraft.confirm() }, onCancel = { form.modeDraft.cancel() },
    )
}

@Composable
fun ConfigModeHiddenDraftNotice(form: ConfigFormState) {
    if (form.showAdvanced) return
    val summary = form.modeDraft.hiddenSummary()
    if (summary.isNotEmpty()) {
        Alert(title = "有尚未保存的高级改动", description = "$summary。切回简洁模式不会清除；保存前将再次确认。", status = AlertStatus.WARNING)
    } else if (form.workflow.customized) {
        Alert(title = "保留当前流程配置", description = "流程结构仍按已保存配置执行。切换高级模式可查看和调整，模式切换不会恢复默认。", status = AlertStatus.DEFAULT)
    } else {
        HintLine("简洁模式只折叠高级参数，现有模型、调优与扩展配置继续保留。")
    }
}
