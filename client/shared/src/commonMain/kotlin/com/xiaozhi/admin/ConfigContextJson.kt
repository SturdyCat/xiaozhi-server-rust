package com.xiaozhi.admin


import com.tencent.kuikly.core.nvi.serialization.json.JSONArray
import com.tencent.kuikly.core.nvi.serialization.json.JSONObject

/**
 * 「灵魂」（`[soul]`）与「记忆」（`[memory]`）的 **JSON 往返**：表单 → JSON、JSON → 表单。
 *
 * 从 `ConfigContextState.kt` 按职责拆出（原文件 606 行，超过 `AGENTS.md` §5.10 的
 * ≤600 行上限）。拆分口径是"读写的两个方向放一起、动作放另一处"：
 *
 * | 本文件 | 职责 | 谁在用 |
 * | --- | --- | --- |
 * | [ContextSectionState.soulJson] | `[soul]` 段 JSON（保存/预览/载入预设三处共用） | 保存、预览、设备 overlay |
 * | [ContextSectionState.putJson] | 表单 → `[soul]` + `[memory]` 请求体 | `ConfigFormDraft` |
 * | [ContextSectionState.fill] | 服务端回包 → 表单 | `ConfigFormFill`、载入预设 |
 *
 * 字段声明与网络动作在 `ConfigContextState.kt`；渲染见 `ConfigSoulMemoryCards.kt`。
 * 三者共享同一个 `ContextSectionState` 实例（`form.sm`），拆的只是**文件**，不是状态。
 *
 * ⚠️ 数组字段（traits/values/boundaries…）在表单里是**多行文本**（每行一条），
 * 读写时统一转换：写入按行拆分成 JSON 数组，读取按 \n 拼回。
 */

internal fun jsonHeaders(): JSONObject = JSONObject().apply { put("Content-Type", "application/json") }

/** 多行文本 → JSON 数组（空行丢弃）。 */
private fun lines(s: String): JSONArray {
    val arr = JSONArray()
    s.split("\n").map { it.trim() }.filter { it.isNotEmpty() }.forEach { arr.put(it) }
    return arr
}

/** JSON 数组 → 多行文本。 */
private fun joinLines(arr: JSONArray?): String {
    if (arr == null) return ""
    val out = mutableListOf<String>()
    for (i in 0 until arr.length()) out.add(arr.optString(i) ?: "")
    return out.joinToString("\n")
}

/**
 * `[soul]` 段的 JSON（保存、预览、载入预设三处共用，避免字段漂移）。
 *
 * ⚠️ `preset` 必须回传：它是"默认灵魂"的基线选择；不回传就会被服务端的部分更新语义
 * 保留成旧值（用户在下拉里换了预设却等于没换）。
 *
 * internal：设备个性化（P1-5）构建 overlay 时整段复用（`[soul]` 在白名单内全段可覆盖）。
 */
internal fun ContextSectionState.soulJson(): JSONObject = JSONObject().apply {
    put("enabled", soulEnabled.toBooleanStrictOrNull() ?: true)
    put("preset", soulPreset)
    put("name", soulName)
    put("self_intro", soulSelfIntro)
    put("form", soulForm)
    put("age_feel", soulAgeFeel)
    put("worldview", soulWorldview)
    put("backstory", soulBackstory)
    put("relationship_origin", soulRelationshipOrigin)
    put("traits", lines(soulTraits))
    put("values", lines(soulValues))
    put("boundaries", lines(soulBoundaries))
    put("tone", soulTone)
    put("colloquial", soulColloquial.toBooleanStrictOrNull() ?: true)
    put("address_user", soulAddressUser)
    put("catchphrases", lines(soulCatchphrases))
    put("emoji", soulEmoji.toBooleanStrictOrNull() ?: false)
    put("max_sentences", soulMaxSentences.toIntOrNull() ?: 2)
    put("max_chars", soulMaxChars.toIntOrNull() ?: 40)
    put("scenario", soulScenario)
    put("device_hint", soulDeviceHint)
    put("examples", lines(soulExamples))
}

/** 服务端错误正文（`{ok:false,message}`）→ 可展示文案；缺正文时退回传输错误/HTTP 码。 */
internal fun serverMessage(data: JSONObject?, errorMsg: String, code: Int?): String {
    val msg = data?.optString("message", "") ?: ""
    if (msg.isNotEmpty()) return msg
    return if (errorMsg.isNotEmpty()) errorMsg else "HTTP $code"
}

/** JSON 字符串数组 → "a、b、c"（预览里列出字段名）。 */
internal fun joinNames(arr: JSONArray?): String {
    if (arr == null) return ""
    val out = mutableListOf<String>()
    for (i in 0 until arr.length()) out.add(arr.optString(i) ?: "")
    return out.filter { it.isNotEmpty() }.joinToString("、")
}

// ============================================================
// 写入：表单 → JSON（供 ConfigFormState.save 合并进请求体）
// ============================================================

internal fun ContextSectionState.putJson(body: JSONObject) {
    body.put("soul", soulJson())
    body.put(
        "memory",
        JSONObject().apply {
            put("enabled", memoryEnabled.toBooleanStrictOrNull() ?: true)
            put("engine", memoryBackend)
            put("db_path", memoryDbPath)
            put("db_busy_timeout_ms", memoryDbBusyTimeoutMs.toIntOrNull() ?: 5000)
            put("recall_enabled", memoryRecallEnabled.toBooleanStrictOrNull() ?: true)
            put("extraction_enabled", memoryExtractionEnabled.toBooleanStrictOrNull() ?: true)
            put("fresh_turn_count", memoryFreshTurnCount.toIntOrNull() ?: 5)
            put("recall_max_nodes", memoryRecallMaxNodes.toIntOrNull() ?: 6)
            put("recall_max_tokens", memoryRecallMaxTokens.toIntOrNull() ?: 400)
            put("recall_budget_ms", memoryRecallBudgetMs.toIntOrNull() ?: 300)
            put("maintenance_interval", memoryMaintenanceInterval.toIntOrNull() ?: 6)
            put("inject_position", memoryInjectPosition)
            put("quarantine_max_attempts", memoryQuarantineMaxAttempts.toIntOrNull() ?: 3)
            put("scope_prefix", memoryScopePrefix)
            put(
                "retention",
                JSONObject().apply {
                    put("keep", memoryRetentionKeep)
                    put("recent_turns", memoryRetentionRecentTurns.toIntOrNull() ?: 0)
                    put("retention_days", memoryRetentionDays.toIntOrNull() ?: 0)
                    put("dry_run", memoryRetentionDryRun.toBooleanStrictOrNull() ?: false)
                },
            )
            put(
                "extractor",
                JSONObject().apply {
                    put("api_base", memoryExtractorApiBase)
                    // 打码后表单里没有明文：留空 = 不修改（不发送该字段）
                    putSecret("api_key", memoryExtractorApiKey)
                    put("model", memoryExtractorModel)
                    put("temperature", memoryExtractorTemperature.toDoubleOrNull() ?: 0.1)
                },
            )
            put(
                "rag",
                JSONObject().apply {
                    put("docs_dir", memoryRagDocsDir)
                    put("chunk_size", memoryRagChunkSize.toIntOrNull() ?: 512)
                    put("chunk_overlap", memoryRagChunkOverlap.toIntOrNull() ?: 64)
                    put("top_k", memoryRagTopK.toIntOrNull() ?: 4)
                },
            )
        },
    )
}

// ============================================================
// 读取：JSON → 表单
// ============================================================

internal fun ContextSectionState.fill(obj: JSONObject) {
    obj.optJSONObject("soul")?.let { s ->
        soulEnabled = s.boolText("enabled", soulEnabled, true)
        // P6.x：内置预设键（旧服务端/旧文件没有它 → 保留当前值）
        soulPreset = s.optString("preset", soulPreset)
        soulName = s.optString("name", soulName)
        soulSelfIntro = s.optString("self_intro", soulSelfIntro)
        soulForm = s.optString("form", soulForm)
        soulAgeFeel = s.optString("age_feel", soulAgeFeel)
        soulWorldview = s.optString("worldview", soulWorldview)
        soulBackstory = s.optString("backstory", soulBackstory)
        soulRelationshipOrigin = s.optString("relationship_origin", soulRelationshipOrigin)
        soulTraits = joinLines(s.optJSONArray("traits"))
        soulValues = joinLines(s.optJSONArray("values"))
        soulBoundaries = joinLines(s.optJSONArray("boundaries"))
        soulTone = s.optString("tone", soulTone)
        soulColloquial = s.boolText("colloquial", soulColloquial, true)
        soulAddressUser = s.optString("address_user", soulAddressUser)
        soulCatchphrases = joinLines(s.optJSONArray("catchphrases"))
        soulEmoji = s.boolText("emoji", soulEmoji, false)
        soulMaxSentences = s.intText("max_sentences", soulMaxSentences, 2)
        soulMaxChars = s.intText("max_chars", soulMaxChars, 40)
        soulScenario = s.optString("scenario", soulScenario)
        soulDeviceHint = s.optString("device_hint", soulDeviceHint)
        soulExamples = joinLines(s.optJSONArray("examples"))
    }
    obj.optJSONObject("memory")?.let { m ->
        memoryEnabled = m.boolText("enabled", memoryEnabled, true)
        // `engine` 是 P4 起的规范键；旧服务端回 `backend`，一并兼容读
        memoryBackend = m.optString("engine", m.optString("backend", memoryBackend))
        memoryDbPath = m.optString("db_path", memoryDbPath)
        memoryDbBusyTimeoutMs = m.intText("db_busy_timeout_ms", memoryDbBusyTimeoutMs, 5000)
        memoryRecallEnabled = m.boolText("recall_enabled", memoryRecallEnabled, true)
        memoryExtractionEnabled = m.boolText("extraction_enabled", memoryExtractionEnabled, true)
        memoryFreshTurnCount = m.intText("fresh_turn_count", memoryFreshTurnCount, 5)
        memoryRecallMaxNodes = m.intText("recall_max_nodes", memoryRecallMaxNodes, 6)
        memoryRecallMaxTokens = m.intText("recall_max_tokens", memoryRecallMaxTokens, 400)
        memoryRecallBudgetMs = m.intText("recall_budget_ms", memoryRecallBudgetMs, 300)
        memoryMaintenanceInterval = m.intText("maintenance_interval", memoryMaintenanceInterval, 6)
        memoryInjectPosition = m.optString("inject_position", memoryInjectPosition)
        memoryQuarantineMaxAttempts = m.intText("quarantine_max_attempts", memoryQuarantineMaxAttempts, 3)
        memoryScopePrefix = m.optString("scope_prefix", memoryScopePrefix)
        m.optJSONObject("retention")?.let { r ->
            memoryRetentionKeep = r.optString("keep", memoryRetentionKeep)
            memoryRetentionRecentTurns = r.intText("recent_turns", memoryRetentionRecentTurns, 0)
            memoryRetentionDays = r.intText("retention_days", memoryRetentionDays, 0)
            memoryRetentionDryRun = r.boolText("dry_run", memoryRetentionDryRun, false)
        }
        m.optJSONObject("extractor")?.let { e ->
            memoryExtractorApiBase = e.optString("api_base", memoryExtractorApiBase)
            // 密钥明文不再下发（服务端打码）：无该键 → 保留用户已输入的草稿；另取 presence 标记
            memoryExtractorApiKey = e.optString("api_key", memoryExtractorApiKey)
            memoryExtractorApiKeyConfigured =
                e.optBoolean("has_api_key", memoryExtractorApiKey.isNotBlank())
            memoryExtractorModel = e.optString("model", memoryExtractorModel)
            memoryExtractorTemperature = e.doubleText("temperature", memoryExtractorTemperature, 0.1)
        }
        // rag 参数组：旧服务端无该键 → 保留当前值（P1-9）
        m.optJSONObject("rag")?.let { r ->
            memoryRagDocsDir = r.optString("docs_dir", memoryRagDocsDir)
            memoryRagChunkSize = r.intText("chunk_size", memoryRagChunkSize, 512)
            memoryRagChunkOverlap = r.intText("chunk_overlap", memoryRagChunkOverlap, 64)
            memoryRagTopK = r.intText("top_k", memoryRagTopK, 4)
        }
    }
}
