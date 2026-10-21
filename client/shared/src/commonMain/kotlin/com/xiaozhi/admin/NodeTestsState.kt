package com.xiaozhi.admin

import com.tencent.kuikly.core.base.PagerScope
import com.tencent.kuikly.core.nvi.serialization.json.JSONObject
import com.tencent.kuikly.core.pager.Pager
import com.tencent.kuikly.core.reactive.collection.ObservableList
import com.tencent.kuikly.core.reactive.handler.observable
import com.tencent.kuikly.core.reactive.handler.observableList

/** Shared Web/macOS HTTP controller. No native bridge, production engine swap, or device action. */
class NodeTestsState(scope: PagerScope) {
    private val ledger = NodeTestLedger()
    private val capabilityGate = NodeRequestGate()
    private var changes by scope.observable(0L)
    private var base = ""
    private var adminToken = "" // Request credential only; never form.expectedToken or a rendered diagnostic.
    private var target = ""
    private var bound = false

    var capabilityState: ActionState by scope.observable(ActionState.Idle)
        private set
    var capabilityMessage by scope.observable("")
        private set
    var snapshotRevision by scope.observable("")
        private set
    private var capabilities by scope.observable(emptyMap<String, NodeTestCapability>())
    var samples by scope.observable(emptyList<NodeTestSample>())
        private set
    // Official dropdown adapter only. Selection and source metadata are not copied into it.
    val sampleOptions: ObservableList<Pair<String, String>> by scope.observableList()
    var selectedSampleId by scope.observable("")
    var inputText by scope.observable("你好，请简短介绍一下你自己。")
    var openMenu by scope.observable("")

    /** A→B→A still advances request generations; invalidation is not remote cancellation. */
    fun invalidateScope() {
        invalidateRuns()
        capabilityGate.invalidate()
        capabilities = emptyMap()
        samples = emptyList()
        sampleOptions.clear()
        snapshotRevision = ""
        capabilityState = ActionState.Idle
        capabilityMessage = "配置或连接已变化，请刷新测试能力"
        openMenu = ""
    }

    fun invalidateRuns() {
        ledger.invalidate()
        changes++
    }

    private fun bind(baseUrl: String, token: String, deviceTarget: String) {
        val normalized = baseUrl.trimEnd('/')
        if (!bound || base != normalized || adminToken != token || target != deviceTarget) {
            invalidateScope()
            base = normalized
            adminToken = token
            target = deviceTarget
            bound = true
        }
    }

    private fun headers(): JSONObject = adminRequestHeaders(adminToken)

    fun refresh(ctx: Pager, baseUrl: String, token: String, deviceTarget: String) {
        bind(baseUrl, token, deviceTarget)
        if (target.isNotEmpty()) {
            capabilityMessage = "共享测试暂只支持已保存的全局快照；设备 overlay / 未保存草稿不会被偷偷当成测试配置"
            capabilityState = ActionState.Finished(ActionOutcome.Failure(capabilityMessage))
            return
        }
        val ticket = capabilityGate.begin("capabilities") ?: return
        invalidateRuns()
        capabilityState = ActionState.Running(ticket.requestId)
        capabilityMessage = "正在读取测试能力（不执行模型推理）"
        val accessRevision = ctx.adminAccessRevision()
        val requestToken = adminToken
        network(ctx).httpRequest("$base/api/test/capabilities", false, JSONObject(), headers(), timeout = 10) { data, success, error, response ->
            if (!ctx.isAdminAccessCurrent(accessRevision, requestToken)) {
                if (capabilityGate.isActive(ticket)) invalidateScope()
                return@httpRequest
            }
            if (!capabilityGate.finish(ticket)) return@httpRequest
            val code = response.statusCode
            if (!success || (code != null && code !in 200..299)) {
                capabilities = emptyMap()
                snapshotRevision = ""
                capabilityMessage = if (code == 404) "此服务端尚不支持共享节点测试" else "读取测试能力失败：${error.ifBlank { "HTTP $code" }}"
                capabilityState = ActionState.Finished(ActionOutcome.Failure(capabilityMessage))
                return@httpRequest
            }
            val revision = data.optString("revision", "")
            if (data.optString("source", "") != "saved" || revision.isBlank()) {
                capabilities = emptyMap()
                capabilityMessage = "服务端测试能力缺少受支持的已保存快照版本"
                capabilityState = ActionState.Finished(ActionOutcome.Failure(capabilityMessage))
                return@httpRequest
            }
            capabilities = parseNodeTestCapabilities(data)
            samples = parseNodeTestSamples(data)
            sampleOptions.clear()
            sampleOptions.addAll(samples.map { it.id to it.label })
            if (samples.none { it.id == selectedSampleId }) selectedSampleId = samples.firstOrNull()?.id.orEmpty()
            snapshotRevision = revision
            capabilityMessage = "使用已保存的全局快照；可调用不等于推理已验证。未保存草稿不参与测试。"
            capabilityState = ActionState.Finished(ActionOutcome.Success())
        }
    }

    fun capability(actionId: String): NodeTestCapability? = capabilities[actionId]

    fun availability(descriptor: NodeTestDescriptor): Availability {
        if (target.isNotEmpty()) return Availability.Disabled("设备 overlay 测试暂未提供快照隔离适配")
        if (capabilityState is ActionState.Running) return Availability.Disabled("正在读取测试能力")
        val server = capabilities[descriptor.id]?.serverCapability()
        if (server == null) return Availability.Disabled(capabilityMessage.ifBlank { "请先刷新测试能力" })
        // Native hardware descriptors stay hidden until an actual client adapter supplies capabilities.
        return descriptor.availability(server, emptySet())
    }

    fun runState(actionId: String): NodeTestRunState {
        changes // Observable version of the pure ledger; no duplicate mutable operation state.
        return ledger.stateFor(actionId)
    }

    fun run(ctx: Pager, descriptor: NodeTestDescriptor, baseUrl: String, token: String, deviceTarget: String) {
        bind(baseUrl, token, deviceTarget)
        if (!canInvokeAction(availability(descriptor), runState(descriptor.id).operation)) return
        val kind = capabilities[descriptor.id]?.inputKind ?: return
        val validation = validateNodeTestInput(kind, inputText, selectedSampleId)
        val summary = when (kind) {
            TestInputKind.Text -> inputText
            TestInputKind.Audio -> samples.firstOrNull { it.id == selectedSampleId }?.label.orEmpty()
            else -> "只读配置预览"
        }
        val ticket = ledger.begin(descriptor.id, summary) ?: return
        changes++
        if (validation != null) {
            ledger.complete(ticket, NodeTestResult("client", descriptor.id, "saved", snapshotRevision, "error",
                NodeTestStage("input", "error", error = validation, errorCode = "invalid_input")))
            changes++
            return
        }
        val revision = snapshotRevision
        val accessRevision = ctx.adminAccessRevision()
        val requestToken = adminToken
        val body = JSONObject().apply {
            put("action_id", descriptor.id)
            put("expected_revision", revision)
            if (kind == TestInputKind.Text) put("text", inputText)
            if (kind == TestInputKind.Audio) put("sample_id", selectedSampleId)
        }
        network(ctx).httpRequest("$base/api/test/run", true, body, headers(), timeout = 60) { data, success, error, response ->
            if (!ctx.isAdminAccessCurrent(accessRevision, requestToken)) {
                if (ledger.isActive(ticket)) invalidateScope()
                return@httpRequest
            }
            val code = response.statusCode
            val failed = !success || (code != null && code !in 200..299)
            val transportError = if (failed) {
                if (code == 409) "配置快照已变化，请刷新测试能力（不会覆盖当前草稿）"
                else error.ifBlank { "测试请求失败：HTTP $code" }
            } else null
            if (ledger.complete(ticket, parseNodeTestResult(data, descriptor.id, revision, transportError))) changes++
        }
    }
}
