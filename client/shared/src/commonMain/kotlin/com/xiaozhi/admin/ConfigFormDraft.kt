package com.xiaozhi.admin

import com.xiaozhi.admin.ui.*

import com.tencent.kuikly.core.nvi.serialization.json.JSONObject

// 只从当前草稿构建 JSON，不持有副本、不填回表单、不自行持久化。
// 缺失字段仍由服务端部分更新合并；密钥沿用 putSecret 的空白省略规则。
internal fun ConfigFormState.buildConfigDraft(): JSONObject {
    val body = JSONObject().apply {
        put("server", JSONObject().apply {
            put("port", port.toIntOrNull() ?: 8000)
            put("expected_token", expectedToken)
            put("worker_threads", workerThreads.toIntOrNull() ?: 2)
            put("admin_dir", adminDir)
        })
        put("audio", JSONObject().apply {
            put("downlink_sample_rate", downlinkSampleRate.toIntOrNull() ?: 24000)
            put("downlink_frame_duration_ms", downlinkFrameMs.toIntOrNull() ?: 60)
            put("channels", channels.toIntOrNull() ?: 1)
            put("binary_protocol_version", binaryProtocolVersion.toIntOrNull() ?: 1)
            put("downlink_lead_ms", downlinkLeadMs.toIntOrNull() ?: 1000)
            put("downlink_queue_packets", downlinkQueuePackets.toIntOrNull() ?: 20)
            put("downlink_buffer_ms", downlinkBufferMs.toIntOrNull() ?: 30000)
            put("downlink_buffer_start_ms", downlinkBufferStartMs.toIntOrNull() ?: 1500)
            put("downlink_buffer_start_max_ms", downlinkBufferStartMaxMs.toIntOrNull() ?: 4000)
        })
        put("playback", JSONObject().apply {
            put("mode", playbackMode)
            put("announcement_start_ms", announcementStartMs.toIntOrNull() ?: 5000)
            put("announcement_start_max_ms", announcementStartMaxMs.toIntOrNull() ?: 10000)
        })
        put("asr", JSONObject().apply {
            put("model", asrModel)
            put("tokens", asrTokens)
            put("language", asrLanguage)
            put("use_itn", asrUseItn.toBooleanStrictOrNull() ?: true)
            put("num_threads", asrNumThreads.toIntOrNull() ?: 2)
            put("provider", asrProvider)
        })
        put("vad", JSONObject().apply {
            put("model", vadModel)
            put("threshold", vadThreshold.toDoubleOrNull() ?: 0.5)
            put("min_silence_duration", vadMinSilence.toDoubleOrNull() ?: 0.25)
            put("min_speech_duration", vadMinSpeech.toDoubleOrNull() ?: 0.25)
        })
        put("tts", JSONObject().apply {
            // P4 配置约定：`engine` 选实现；实现私有参数进 `[tts.<id>]` 子段；
            // 跨实现通用项（音色槽/语速/缓存）留在本体。
            put("engine", ttsBackend)
            put("speaker", ttsSpeaker.toIntOrNull() ?: 0)
            put("speed", ttsSpeed.toDoubleOrNull() ?: 1.0)
            put("cache_entries", ttsCacheEntries.toIntOrNull() ?: 256)
            put("kokoro", JSONObject().apply {
                put("model", ttsModel)
                put("voices", ttsVoices)
                put("tokens", ttsTokens)
                put("data_dir", ttsDataDir)
                put("dict_dir", ttsDictDir)
                put("lexicon", ttsLexicon)
                put("lang", ttsLang)
                put("num_threads", ttsNumThreads.toIntOrNull() ?: 4)
            })
            put("xfyun", JSONObject().apply {
                put("app_id", xfyunAppId)
                // 密钥：留空 = 不修改 SQLite 已保存值（这里直接不发）
                putSecret("api_key", xfyunApiKey)
                putSecret("api_secret", xfyunApiSecret)
                put("voice", xfyunVoice)
            })
        })
        put("llm", JSONObject().apply {
            put("enabled", llmEnabled.toBooleanStrictOrNull() ?: true)
            put("api_base", llmApiBase)
            // 打码后表单里没有明文：留空 = 不修改（不发送该字段）
            putSecret("api_key", llmApiKey)
            put("model", llmModel)
            put("system_prompt", llmSystemPrompt)
            put("max_history", llmMaxHistory.toIntOrNull() ?: 10)
            put("temperature", llmTemperature.toDoubleOrNull() ?: 0.7)
            // ⚠️ stream 必须回传：serde 端有 default，漏传会被重置为 true，
            // SQLite 已保存的 stream=false 不能经 UI 保存一次就丢失。
            put("stream", llmStream.toBooleanStrictOrNull() ?: true)
        })
        // [soul] / [memory]：上下文生产者（P6）。与其它段一样**必须整段回传**：
        // 后端虽是部分更新语义，但前端不回传就等于"这个段在 UI 里不可配置"。
        sm.putJson(this)
        // [command]：指令闸门（ASR → LLM 之间）——同样整段回传，否则保存一次就把它关掉。
        cmd.putJson(this)
        // [wakeup]：唤醒问候语（唤醒 → LLM 之间）——同样整段回传。
        wake.putJson(this)
        // [nlu]：技能级联（自定义意图/槽位是嵌套数组）——同样整段回传，否则保存一次就清空。
        nlu.putJson(this)
        // [echomuse]：设备入口（enabled/token）——同样整段回传，否则保存一次就把它关掉。
        em.putJson(this)
        // [aiui]：此前管理页**完全没有这个段**，保存一次就把它静默关掉（后端已改为部分更新
        // 兜底；这里补齐前端，使全链路模式可在 UI 里配置与保留）。
        put("aiui", JSONObject().apply {
            put("enabled", aiuiEnabled.toBooleanStrictOrNull() ?: false)
            put("appid", aiuiAppid)
            putSecret("api_key", aiuiApiKey)
            putSecret("api_secret", aiuiApiSecret)
            put("scene", aiuiScene)
            put("sn_prefix", aiuiSnPrefix)
            put("voice", aiuiVoice)
            put("speed", aiuiSpeed.toIntOrNull() ?: 50)
            put("volume", aiuiVolume.toIntOrNull() ?: 50)
            put("pitch", aiuiPitch.toIntOrNull() ?: 50)
            put("prompt", aiuiPrompt)
            put("pace_ms", aiuiPaceMs.toIntOrNull() ?: 10)
        })
    }
    // 乐观并发控制：回传上次 GET 的指纹；服务端发现已变化则 409（不覆盖别人的改动）
    val sendRevision = revision
    if (sendRevision.isNotEmpty()) {
        body.put("expected_revision", sendRevision)
    }
    return body
}

/** overlay 的顶层 section（与白名单一一对应；同序构建 → toString 可比较）。 */
private val overlaySectionKeys = listOf("tts", "llm", "soul", "wakeup", "command", "memory")

/**
 * 从当前表单构建**完整白名单 overlay**（所有可覆盖字段的当前生效值）。
 * 仅用于：设备加载时抓全局基线、保存时与基线求差。不直接整体上传——
 * 整体上传会把没改过的白名单字段也钉死到设备上，全局后续调整会被旧值遮蔽。
 */
internal fun ConfigFormState.buildOverlayDraft(): JSONObject = JSONObject().apply {
    put("tts", JSONObject().apply {
        put("speaker", ttsSpeaker.toIntOrNull() ?: 0)
        put("speed", ttsSpeed.toDoubleOrNull() ?: 1.0)
        put("xfyun", JSONObject().apply { put("voice", xfyunVoice) })
    })
    put("llm", JSONObject().apply {
        put("system_prompt", llmSystemPrompt)
        put("max_history", llmMaxHistory.toIntOrNull() ?: 10)
    })
    // [soul] 全段可覆盖：直接复用 sm.soulJson()（与保存/预览同一构建，避免字段漂移）
    put("soul", sm.soulJson())
    // [wakeup] / [command] 全段可覆盖：复用各自 putJson（写入本对象）
    wake.putJson(this)
    cmd.putJson(this)
    // [memory] 仅召回参数可覆盖（引擎/构建/抽取字段不在白名单）
    put("memory", JSONObject().apply {
        put("recall_enabled", sm.memoryRecallEnabled.toBooleanStrictOrNull() ?: true)
        put("fresh_turn_count", sm.memoryFreshTurnCount.toIntOrNull() ?: 5)
        put("recall_max_nodes", sm.memoryRecallMaxNodes.toIntOrNull() ?: 6)
        put("recall_max_tokens", sm.memoryRecallMaxTokens.toIntOrNull() ?: 400)
        put("recall_budget_ms", sm.memoryRecallBudgetMs.toIntOrNull() ?: 300)
        put("inject_position", sm.memoryInjectPosition)
    })
}

/**
 * 求差：当前表单 vs 全局基线，返回「相对全局有改动的 section」。
 * 同一构建器同序 put → section 的 toString 相等即内容相等（无需逐字段比较）。
 */
internal fun ConfigFormState.computeOverlayDraftDiff(deviceBaseline: JSONObject?): JSONObject {
    val diff = JSONObject()
    val current = buildOverlayDraft()
    val baseline = deviceBaseline ?: JSONObject()
    for (key in overlaySectionKeys) {
        val cur = current.optJSONObject(key)
        val base = baseline.optJSONObject(key)
        if (cur != null && cur.toString() != (base?.toString() ?: "")) {
            diff.put(key, cur)
        }
    }
    return diff
}
