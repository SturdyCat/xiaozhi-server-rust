package com.xiaozhi.admin

import com.tencent.kuikly.core.nvi.serialization.json.JSONObject
import com.tencent.kuikly.core.pager.Pager
import com.tencent.kuikly.core.timer.setTimeout
import com.xiaozhi.admin.state.stateVar

/** BERT fields belong to the existing config form, never a second save payload. */
data class NluBertDraft(
    val enabled: String = "false",
    val model: String = "",
    val vocab: String = "",
    val labels: String = "",
    val threshold: String = "0.75",
    val threads: String = "1",
    val maxLen: String = "64",
    val labelMap: String = "{}",
) {
    fun read(n: JSONObject) = copy(
        enabled = n.boolText("bert_enabled", enabled, false),
        model = n.optString("bert_model", model),
        vocab = n.optString("bert_vocab", vocab),
        labels = n.optString("bert_labels", labels),
        threshold = n.doubleText("bert_threshold", threshold, 0.75),
        threads = n.intText("bert_num_threads", threads, 1),
        maxLen = n.intText("bert_max_len", maxLen, 64),
        labelMap = n.optJSONObject("bert_label_map")?.toString() ?: labelMap,
    )

    fun putJson(n: JSONObject) {
        n.put("bert_enabled", enabled == "true")
        n.put("bert_model", model)
        n.put("bert_vocab", vocab)
        n.put("bert_labels", labels)
        n.put("bert_threshold", threshold.toDoubleOrNull() ?: 0.75)
        n.put("bert_num_threads", threads.toIntOrNull() ?: 1)
        n.put("bert_max_len", maxLen.toIntOrNull() ?: 64)
        n.put("bert_label_map", JSONObject(labelMap))
    }
}

/**
 * 界面展示用的底座名。**纯展示**、不参与任何请求：服务端只有唯一底座
 * （`nlu_build/base.rs` 的 `BASE_MODEL`），构建请求里不再有 `base_model` 字段
 * （服务端 `BuildRequest` 是 `deny_unknown_fields`，多传一个键直接 400）。
 * 这里的名字只用于文案，改错了最多是标签过时，不会训出另一个底座。
 */
internal const val NLU_BASE_MODEL_LABEL = "minirbt-h288"

data class NluCandidate(
    val buildId: String,
    val sourceRevision: String,
    val sha256: String,
    val config: NluBertDraft,
    /**
     * 该候选训练时用的底座规范名（服务端 manifest 的只读记录）。
     * **空串 = 历史 manifest 没有记录底座**（不要猜成当前底座，界面会单独说明）。
     */
    val baseModel: String = "",
)

data class NluModelView(
    val busy: Boolean = false,
    val phase: String = "",
    val message: String = "",
    val log: String = "",
    val candidate: NluCandidate? = null,
    val testText: String = "",
    val expectedIntent: String = "",
    val testBusy: Boolean = false,
    val testMessage: String = "",
    val testedKey: String = "",
    val verified: Boolean = false,
)

/** Saved-config build → candidate-only inference → explicit adoption via ConfigFormState.save. */
class NluModelState {
    var view by stateVar(NluModelView())
        private set
    private var generation = 0
    private var requestBase = ""
    private var accessRevision = -1L
    private var requestToken = ""

    fun invalidate() {
        generation++
        // 底座不再可选：整份 view 都是服务端派生值，重置无副作用。
        view = NluModelView()
    }

    private fun bind(ctx: Pager, form: ConfigFormState): Int {
        if (requestBase != form.serverBase || !ctx.isAdminAccessCurrent(accessRevision, requestToken)) invalidate()
        requestBase = form.serverBase
        accessRevision = ctx.adminAccessRevision()
        requestToken = ctx.adminRequestToken()
        return generation
    }

    private fun current(ctx: Pager, form: ConfigFormState, ticket: Int) =
        ticket == generation && form.serverBase == requestBase &&
            ctx.isAdminAccessCurrent(accessRevision, requestToken) && form.editDeviceId.isEmpty()

    private fun error(data: JSONObject?, fallback: String): String =
        data?.optString("error", "").orEmpty().ifEmpty { fallback }

    fun start(ctx: Pager, form: ConfigFormState) {
        if (form.dirty || form.workflow.dirty || form.saving || form.conflict || form.revision.isEmpty() ||
            form.editDeviceId.isNotEmpty() || view.busy || view.testBusy) return
        val ticket = bind(ctx, form)
        view = view.copy(busy = true, phase = "running", message = "正在按已保存配置启动训练…", candidate = null, verified = false, testMessage = "")
        val sourceRevision = form.revision
        val body = JSONObject().apply {
            put("expected_revision", sourceRevision)
            // 不传 base_model：服务端只有唯一底座，且 `BuildRequest` 是
            // `deny_unknown_fields`——多传一个键直接 400。
        }
        rpcCall(ctx, "${form.serverBase}/api/nlu/model/build", "POST", body) { ok, data, status, message ->
            if (!current(ctx, form, ticket)) return@rpcCall
            if (!ok || data.optString("state", "") != "running" ||
                !data.optBoolean("started", false) ||
                data.optString("build_id", "").isEmpty() || data.optString("source_revision", "") != sourceRevision) {
                view = view.copy(busy = false, phase = "failed", message = "启动失败：${error(data, message.ifEmpty { "HTTP $status" })}")
                return@rpcCall
            }
            view = view.copy(message = "训练已启动。只使用已保存的技能配置；构建成功不会自动启用。")
            status(ctx, form, ticket, 0)
        }
    }

    /** Manual refresh can reattach after a page reload or a long-running build. */
    fun refresh(ctx: Pager, form: ConfigFormState) {
        if (view.busy || view.testBusy || form.editDeviceId.isNotEmpty()) return
        val ticket = bind(ctx, form)
        view = view.copy(busy = true, message = "正在读取训练状态…")
        status(ctx, form, ticket, 0)
    }

    private fun status(ctx: Pager, form: ConfigFormState, ticket: Int, attempts: Int) {
        if (!current(ctx, form, ticket)) return
        if (attempts >= 360) {
            view = view.copy(busy = false, message = "已停止等待，服务器训练可能仍在继续。请稍后手动刷新状态。")
            return
        }
        // GET uses the same management auth as POST; no credentials in URL/log output.
        rpcCall(ctx, "${form.serverBase}/api/nlu/model/build", "GET", null) { ok, data, status, message ->
            if (!current(ctx, form, ticket)) return@rpcCall
            if (!ok) {
                view = view.copy(busy = false, message = "状态读取失败：${error(data, message.ifEmpty { "HTTP $status" })}")
                return@rpcCall
            }
            val phase = data.optString("state", "")
            val candidate = parseCandidate(data.optJSONObject("artifact"), form.nlu.bert)
            view = view.copy(
                busy = phase == "running", phase = phase, log = data.optString("log_tail", ""),
                candidate = candidate,
                verified = view.verified && candidate != null && candidate == view.candidate,
                message = when (phase) {
                    "running" -> "训练中；使用保存态版本 ${data.optString("source_revision", "未知")}，尚未启用。"
                    "success" -> if (candidate == null) "构建已结束，但服务端没有完整的候选版本信息。请更新服务端；不能直接启用。" else "候选模型已生成，尚未启用。请先独立测试新技能与拒识样例。"
                    "failed" -> "训练失败：${error(data, "请查看日志") }"
                    else -> "暂无训练任务。先保存技能，再构建候选模型。"
                },
            )
            if (phase == "running") ctx.setTimeout(5000) { status(ctx, form, ticket, attempts + 1) }
        }
    }

    fun editText(value: String) { view = view.copy(testText = value, verified = false) }
    fun editExpected(value: String) { view = view.copy(expectedIntent = value, verified = false) }

    fun test(ctx: Pager, form: ConfigFormState) {
        val candidate = view.candidate ?: return
        val text = view.testText.trim()
        val expected = view.expectedIntent.trim()
        if (view.testBusy || view.busy || text.isEmpty() || expected.isEmpty()) return
        val ticket = bind(ctx, form)
        if (view.candidate?.buildId != candidate.buildId) return
        val key = testKey(candidate, text, expected)
        view = view.copy(testBusy = true, verified = false, testMessage = "正在直接运行候选 ONNX 模型；不执行技能、不改变全局开关…")
        val body = JSONObject().apply { put("build_id", candidate.buildId); put("text", text); put("expected_intent", expected) }
        // ONNX 推理 + 分词，60s 本地超时（rpc 帧自动带 62s 服务端执行超时）。
        rpcCall(ctx, "${form.serverBase}/api/nlu/model/test", "POST", body, timeoutMs = 60_000) { ok, data, status, message ->
            if (!current(ctx, form, ticket)) return@rpcCall
            if (!ok || data.optString("state", "") != "ok") {
                view = view.copy(testBusy = false, testMessage = "候选测试失败：${error(data, message.ifEmpty { "HTTP $status" })}")
                return@rpcCall
            }
            val valid = data.optString("build_id", "") == candidate.buildId &&
                data.optString("model_sha256", "") == candidate.sha256 &&
                data.optString("source_revision", "") == candidate.sourceRevision &&
                data.optString("expected_intent", "") == expected &&
                data.optString("measurement", "") == "real_onnx_inference" && data.has("executed") && !data.optBoolean("executed", true)
            val accepted = data.optBoolean("accepted", false)
            val matched = data.optBoolean("expected_match", false)
            val unchanged = key == testKey(candidate, view.testText.trim(), view.expectedIntent.trim())
            val label = data.optString("intent", "").ifEmpty { data.optString("raw_label", "未知") }
            val result = if (!valid) "候选版本或推理证据不一致，不能采用。" else
                "预测：$label；置信度 ${data.optDouble("confidence", 0.0)}；阈值 ${data.optDouble("threshold", 0.0)}；" +
                    (if (accepted) "达到接受条件" else "拒识或未达阈值") + "；" +
                    (if (matched) "与预期一致" else "与预期不一致") +
                    "。分词 ${data.optLong("tokenize_ms", 0)} ms，推理 ${data.optLong("infer_ms", 0)} ms。未执行技能。"
            view = view.copy(testBusy = false, testMessage = result, testedKey = key,
                verified = valid && unchanged && accepted && matched)
        }
    }

    fun canAdopt(ctx: Pager, form: ConfigFormState): Boolean {
        val c = view.candidate ?: return false
        return current(ctx, form, generation) && view.verified && view.testedKey == testKey(c, view.testText.trim(), view.expectedIntent.trim()) &&
            c.sourceRevision == form.revision && !form.dirty && !form.workflow.dirty && !form.saving &&
            !form.conflict && !view.busy && !view.testBusy && form.editDeviceId.isEmpty()
    }

    fun adopt(ctx: Pager, form: ConfigFormState) {
        if (!canAdopt(ctx, form)) return
        form.nlu.bert = view.candidate!!.config
        form.markDirty()
        view = view.copy(message = "已将已测候选放入当前配置，正在通过原配置保存流程提交。取消确认或保存失败会保留草稿，不代表生效。")
        form.save(ctx, form.serverBase)
    }

    private fun testKey(c: NluCandidate, text: String, expected: String) =
        listOf(c.buildId, c.sourceRevision, c.sha256, text, expected).joinToString("") { "${it.length}:$it" }

    private fun parseCandidate(artifact: JSONObject?, previous: NluBertDraft): NluCandidate? {
        if (artifact == null) return null
        val id = artifact.optString("build_id", "")
        val revision = artifact.optString("source_revision", "")
        val sha = artifact.optString("model_sha256", "")
        val config = artifact.optJSONObject("expected_config") ?: return null
        // 底座是**可选读**字段：既有的 8 个 expected_config 键继续做强校验，
        // base_model 绝不进这份 required 列表——旧产物没有该键，加了就会把一份能用的
        // 候选判成 null。缺失时**留空**（界面显示"未记录"），不猜成当前底座。
        val baseModel = artifact.optString("base_model", "")
        val required = listOf("bert_enabled", "bert_model", "bert_vocab", "bert_labels", "bert_threshold", "bert_num_threads", "bert_max_len", "bert_label_map")
        if (required.any { !config.has(it) } || config.optJSONObject("bert_label_map") == null) return null
        val draft = previous.read(config)
        if (id.isEmpty() || revision.isEmpty() || sha.isEmpty() || draft.enabled != "true" || draft.model.isEmpty() || draft.vocab.isEmpty() || draft.labels.isEmpty()) return null
        return NluCandidate(id, revision, sha, draft, baseModel)
    }
}
