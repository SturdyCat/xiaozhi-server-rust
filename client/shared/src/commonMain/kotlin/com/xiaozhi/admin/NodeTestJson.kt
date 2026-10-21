package com.xiaozhi.admin

import com.tencent.kuikly.core.nvi.serialization.json.JSONObject

internal fun parseNodeTestCapabilities(data: JSONObject): Map<String, NodeTestCapability> {
    val rows = data.optJSONArray("actions") ?: return emptyMap()
    val allowed = PipelineFlow.all.flatMap { it.tests }.map { it.id }.toSet() + setOf("pipeline.text", "pipeline.audio")
    val result = mutableMapOf<String, NodeTestCapability>()
    for (i in 0 until rows.length().coerceAtMost(64)) {
        val row = rows.optJSONObject(i) ?: continue
        val id = row.optString("action_id", "")
        if (id !in allowed) continue
        result[id] = NodeTestCapability(
            id, row.optBoolean("available", false), row.optString("reason", "").take(512),
            row.optString("readiness", "unavailable"), TestInputKind.fromWire(row.optString("input_kind", "")),
        )
    }
    return result
}

internal fun parseNodeTestSamples(data: JSONObject): List<NodeTestSample> {
    val rows = data.optJSONArray("samples") ?: return emptyList()
    val result = mutableListOf<NodeTestSample>()
    for (i in 0 until rows.length().coerceAtMost(16)) {
        val row = rows.optJSONObject(i) ?: continue
        val id = row.optString("id", "")
        if (id.isBlank() || id.length > 80 || result.any { it.id == id }) continue
        result.add(NodeTestSample(id, row.optString("label", id).take(128), row.optString("transcript", "").take(512), row.optString("license", "").take(256)))
    }
    return result
}

private fun JSONObject.optionalMs(key: String): Long? = optLong(key, -1).takeIf { it >= 0 }

internal fun parseNodeTestStage(data: JSONObject): NodeTestStage = NodeTestStage(
    stage = data.optString("stage", "test"), state = data.optString("state", "error"),
    elapsedMs = data.optionalMs("elapsed_ms"), firstPcmMs = data.optionalMs("first_pcm_ms"),
    firstTokenMs = data.optionalMs("first_token_ms"), engine = data.optString("engine", "").take(160),
    text = data.optString("text", "").take(4096), error = data.optString("error", "").take(1024),
    errorCode = data.optString("error_code", "").take(160), measurement = data.optString("measurement", "").take(80),
    detailsText = data.optJSONObject("details")?.toString()?.take(3000).orEmpty(),
)

internal fun parseNodeTestResult(data: JSONObject, actionId: String, revision: String, transportError: String?): NodeTestResult {
    val stage = parseNodeTestStage(data)
    val stages = mutableListOf<NodeTestStage>()
    val array = data.optJSONArray("stages")
    if (array != null) for (i in 0 until array.length().coerceAtMost(24)) {
        array.optJSONObject(i)?.let { stages.add(parseNodeTestStage(it)) }
    }
    val result = NodeTestResult(
        requestId = data.optString("request_id", ""), actionId = data.optString("action_id", ""),
        source = data.optString("source", ""), revision = data.optString("revision", ""),
        state = data.optString("state", "error"), stage = stage, stages = stages,
        contextMode = data.optString("context_mode", "isolated"),
    )
    if (transportError != null) return result.copy(
        actionId = actionId, state = "error",
        stage = stage.copy(state = "error", error = stage.error.ifBlank { transportError.take(1024) }),
    )
    if (result.actionId != actionId || result.source != "saved" || result.revision != revision || result.requestId.isBlank()) {
        return result.copy(actionId = actionId, state = "error", stage = stage.copy(
            state = "error", error = "测试响应来源、版本或标识不匹配，请刷新测试能力后重试", errorCode = "response_mismatch",
        ))
    }
    return result
}
