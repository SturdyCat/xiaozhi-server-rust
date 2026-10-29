package com.xiaozhi.admin

import com.xiaozhi.admin.contracts.*

/** Shared read-only test contracts; no JSON or platform/Kuikly dependency. */
enum class TestInputKind {
    None, Text, Audio, Unsupported;
    companion object {
        fun fromWire(value: String): TestInputKind = when (value) {
            "none" -> None
            "text" -> Text
            "audio", "sample", "pcm", "sample_or_pcm" -> Audio
            else -> Unsupported
        }
    }
}

data class NodeTestCapability(
    val actionId: String,
    val available: Boolean,
    val reason: String,
    val readiness: String,
    val inputKind: TestInputKind,
) {
    fun serverCapability(): ServerTestCapability = ServerTestCapability(
        actionId,
        available && readiness in setOf("configured", "provider_verified") && inputKind != TestInputKind.Unsupported,
        reason.ifBlank { if (inputKind == TestInputKind.Unsupported) "客户端不支持此输入类型" else "服务端未就绪" },
    )
}

data class NodeTestSample(val id: String, val label: String, val transcript: String, val license: String)

data class NodeTestStage(
    val stage: String,
    val state: String,
    val elapsedMs: Long? = null,
    val firstPcmMs: Long? = null,
    val firstTokenMs: Long? = null,
    val engine: String = "",
    val text: String = "",
    val error: String = "",
    val errorCode: String = "",
    val measurement: String = "",
    val detailsText: String = "",
)

data class NodeTestResult(
    val requestId: String,
    val actionId: String,
    val source: String,
    val revision: String,
    val state: String,
    val stage: NodeTestStage,
    val stages: List<NodeTestStage> = emptyList(),
    val contextMode: String = "isolated",
) {
    val succeeded: Boolean get() = state == "ok"
    fun outcome(): ActionOutcome = if (succeeded) ActionOutcome.Success("测试完成")
        else ActionOutcome.Failure(stage.error.ifBlank { "测试未完成：${stage.errorCode.ifBlank { state }}" })
}

data class NodeTestRunState(
    val operation: ActionState = ActionState.Idle,
    val result: NodeTestResult? = null,
    val inputSummary: String = "",
)

/** The context-thread controller observes a version of this single pure state owner. */
class NodeTestLedger {
    private val gate = NodeRequestGate()
    private val runs = mutableMapOf<String, NodeTestRunState>()

    fun stateFor(actionId: String): NodeTestRunState = runs[actionId] ?: NodeTestRunState()
    fun isActive(ticket: NodeRequestTicket): Boolean = gate.isActive(ticket)

    fun begin(actionId: String, inputSummary: String): NodeRequestTicket? {
        val ticket = gate.begin(actionId) ?: return null
        runs[actionId] = NodeTestRunState(ActionState.Running(ticket.requestId), inputSummary = inputSummary.take(160))
        return ticket
    }

    fun complete(ticket: NodeRequestTicket, result: NodeTestResult): Boolean {
        if (!gate.finish(ticket)) return false
        val accepted = if (result.actionId == ticket.actionId) result else NodeTestResult(
            requestId = result.requestId, actionId = ticket.actionId, source = result.source,
            revision = result.revision, state = "error",
            stage = NodeTestStage("response", "error", error = "测试响应标识不匹配", errorCode = "response_mismatch"),
        )
        runs[ticket.actionId] = stateFor(ticket.actionId).copy(operation = ActionState.Finished(accepted.outcome()), result = accepted)
        return true
    }

    fun invalidate() {
        gate.invalidate()
        runs.clear()
    }
}

fun validateNodeTestInput(kind: TestInputKind, text: String, sampleId: String): String? = when {
    kind == TestInputKind.Unsupported -> "客户端不支持此输入类型"
    kind == TestInputKind.Text && text.isBlank() -> "请输入测试文本"
    kind == TestInputKind.Text && text.encodeToByteArray().size > 4096 -> "测试文本最多 4096 UTF-8 字节"
    kind == TestInputKind.Audio && sampleId.isBlank() -> "请选择服务端授权的语音样本"
    else -> null
}
