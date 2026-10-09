package com.xiaozhi.admin

import com.tencent.kuikly.core.nvi.serialization.json.JSONObject

// ============================================================
// 服务端 JSON ⇄ 表单 observable 的映射（从 ConfigFormState.kt 拆出）
// ============================================================
// 拆出原因：`AGENTS.md` §5.10「逻辑文件 ≤ 600 行」。
// 这里只用到 `ConfigFormState` 的**公开** observable 字段 + 两个成员/内部方法
// （`reloadXfyunVoiceOptions`、`deriveVoiceGroupAndType`），因此可作为扩展函数独立成文件。
// ⚠️ `putSecret` 是 `internal`（同模块可见）：`buildConfigJson()` 仍在 ConfigFormState.kt 里调用它。
// ============================================================

/**
 * 用服务端 /api/config 回包填充表单（公开：macOS ConnectState 连接流程自行 GET 后调用，
 * 以便同时提取 expected_token 供 WS 鉴权；load() 内部也走这里）。
 */
internal fun ConfigFormState.fill(obj: JSONObject) {
    revision = obj.optString("revision", revision)
    obj.optJSONObject("server")?.let { s ->
        port = s.optInt("port", port.toIntOrNull() ?: 8000).toString()
        expectedToken = s.optString("expected_token", expectedToken)
        workerThreads = s.optInt("worker_threads", workerThreads.toIntOrNull() ?: 2).toString()
        adminDir = s.optString("admin_dir", adminDir)
    }
    obj.optJSONObject("audio")?.let { a ->
        downlinkSampleRate = a.optInt("downlink_sample_rate", downlinkSampleRate.toIntOrNull() ?: 24000).toString()
        downlinkFrameMs = a.optInt("downlink_frame_duration_ms", downlinkFrameMs.toIntOrNull() ?: 60).toString()
        channels = a.optInt("channels", channels.toIntOrNull() ?: 1).toString()
        binaryProtocolVersion = a.optInt("binary_protocol_version", binaryProtocolVersion.toIntOrNull() ?: 1).toString()
        downlinkLeadMs = a.optInt("downlink_lead_ms", downlinkLeadMs.toIntOrNull() ?: 240).toString()
    }
    obj.optJSONObject("asr")?.let { a ->
        asrModel = a.optString("model", asrModel)
        asrTokens = a.optString("tokens", asrTokens)
        asrLanguage = a.optString("language", asrLanguage)
        asrUseItn = a.optBoolean("use_itn", asrUseItn.toBooleanStrictOrNull() ?: true).toString()
        asrNumThreads = a.optInt("num_threads", asrNumThreads.toIntOrNull() ?: 2).toString()
        asrProvider = a.optString("provider", asrProvider)
    }
    obj.optJSONObject("vad")?.let { v ->
        vadModel = v.optString("model", vadModel)
        vadThreshold = v.optDouble("threshold", vadThreshold.toDoubleOrNull() ?: 0.5).toString()
        vadMinSilence = v.optDouble("min_silence_duration", vadMinSilence.toDoubleOrNull() ?: 0.25).toString()
        vadMinSpeech = v.optDouble("min_speech_duration", vadMinSpeech.toDoubleOrNull() ?: 0.25).toString()
    }
    obj.optJSONObject("tts")?.let { t ->
        // P4 配置约定：规范键是 `engine`；旧服务端/旧文件可能仍是 `backend`，本地引擎的
        // 历史值 "sherpa" 也统一折成 "kokoro"（否则下拉框选不中任何一项）。
        ttsBackend = canonicalTtsEngine(t.optString("engine", t.optString("backend", ttsBackend)))
        ttsMode = if (ttsEngineIsRemote(ttsBackend)) "remote" else "local"
        if (ttsMode == "remote") lastRemoteEngine = ttsBackend // 记住远程服务商，切回时恢复
        // 实现私有段 `[tts.kokoro]`；旧写法是把这些字段直接放在 `[tts]` 下（双读兼容）
        val ko = t.optJSONObject("kokoro")
        fun privateStr(key: String, current: String): String =
            ko?.optString(key, "")?.takeIf { it.isNotEmpty() }
                ?: t.optString(key, "").takeIf { it.isNotEmpty() }
                ?: current
        fun privateInt(key: String, current: String, fallback: Int): String =
            (ko?.optInt(key, 0)?.takeIf { it > 0 }
                ?: t.optInt(key, 0).takeIf { it > 0 }
                ?: current.toIntOrNull()
                ?: fallback).toString()
        ttsModel = privateStr("model", ttsModel)
        ttsVoices = privateStr("voices", ttsVoices)
        ttsTokens = privateStr("tokens", ttsTokens)
        ttsDataDir = privateStr("data_dir", ttsDataDir)
        ttsDictDir = privateStr("dict_dir", ttsDictDir)
        ttsLexicon = privateStr("lexicon", ttsLexicon)
        ttsLang = privateStr("lang", ttsLang)
        ttsNumThreads = privateInt("num_threads", ttsNumThreads, 4)
        // 跨实现通用项：位置未变（旧、新写法都在 `[tts]` 本体）
        ttsSpeaker = t.optInt("speaker", ttsSpeaker.toIntOrNull() ?: 0).toString()
        ttsSpeed = t.optDouble("speed", ttsSpeed.toDoubleOrNull() ?: 1.0).toString()
        ttsCacheEntries = t.optInt("cache_entries", ttsCacheEntries.toIntOrNull() ?: 256).toString()
        t.optJSONObject("xfyun")?.let { x ->
            xfyunAppId = x.optString("app_id", xfyunAppId)
            // 密钥明文不再下发（服务端打码）：无该键 → 保留用户已输入的草稿；另取 presence 标记
            xfyunApiKey = x.optString("api_key", xfyunApiKey)
            xfyunApiSecret = x.optString("api_secret", xfyunApiSecret)
            xfyunApiKeyConfigured =
                x.optBoolean("has_api_key", xfyunApiKey.isNotBlank())
            xfyunApiSecretConfigured =
                x.optBoolean("has_api_secret", xfyunApiSecret.isNotBlank())
            xfyunVoice = x.optString("voice", xfyunVoice)
            // 按已存音色反推分组：不在预置目录 → 自定义（手填）
            deriveVoiceGroupAndType()
            reloadXfyunVoiceOptions()
        }
    }
    obj.optJSONObject("llm")?.let { l ->
        llmApiBase = l.optString("api_base", llmApiBase)
        llmApiKey = l.optString("api_key", llmApiKey)
        llmApiKeyConfigured = l.optBoolean("has_api_key", llmApiKey.isNotBlank())
        llmModel = l.optString("model", llmModel)
        llmSystemPrompt = l.optString("system_prompt", llmSystemPrompt)
        llmMaxHistory = l.optInt("max_history", llmMaxHistory.toIntOrNull() ?: 10).toString()
        llmTemperature = l.optDouble("temperature", llmTemperature.toDoubleOrNull() ?: 0.7).toString()
        llmStream = l.optBoolean("stream", llmStream.toBooleanStrictOrNull() ?: true).toString()
    }
    obj.optJSONObject("aiui")?.let { a ->
        aiuiEnabled = a.optBoolean("enabled", aiuiEnabled.toBooleanStrictOrNull() ?: false).toString()
        aiuiAppid = a.optString("appid", aiuiAppid)
        aiuiApiKey = a.optString("api_key", aiuiApiKey)
        aiuiApiSecret = a.optString("api_secret", aiuiApiSecret)
        aiuiApiKeyConfigured = a.optBoolean("has_api_key", aiuiApiKey.isNotBlank())
        aiuiApiSecretConfigured =
            a.optBoolean("has_api_secret", aiuiApiSecret.isNotBlank())
        aiuiScene = a.optString("scene", aiuiScene)
        aiuiSnPrefix = a.optString("sn_prefix", aiuiSnPrefix)
        aiuiVoice = a.optString("voice", aiuiVoice)
        aiuiSpeed = a.optInt("speed", aiuiSpeed.toIntOrNull() ?: 50).toString()
        aiuiVolume = a.optInt("volume", aiuiVolume.toIntOrNull() ?: 50).toString()
        aiuiPitch = a.optInt("pitch", aiuiPitch.toIntOrNull() ?: 50).toString()
        aiuiPrompt = a.optString("prompt", aiuiPrompt)
        aiuiPaceMs = a.optInt("pace_ms", aiuiPaceMs.toIntOrNull() ?: 10).toString()
    }
    // [soul] / [memory]（上下文生产者）：字段较多，映射集中在 ConfigContextState
    sm.fill(obj)
}

/**
 * 密钥字段写入：**留空 = 不发送**。
 *
 * 服务端已对密钥打码（GET 只回 `has_*` presence 标记），表单里永远没有明文；
 * 用户不重填就不该发出任何密钥字段——服务端把"缺失/空串"一并解释为「保留盘上原值」
 *（见 `server/src/app/ws/config.rs` 的 `SECRET_PATHS` / `sanitize_patch`）。
 */
internal fun JSONObject.putSecret(key: String, value: String) {
    val v = value.trim()
    if (v.isNotEmpty()) put(key, v)
}

/**
 * TTS 引擎 id 规范化：P4 之前本地引擎叫 `sherpa`（`[tts].backend` 的取值），
 * 现在规范值是 `kokoro`（= 服务端实现 id `tts.kokoro` 的后半段）。服务端两种都认，
 * 客户端统一按规范值显示与提交。
 */
internal fun canonicalTtsEngine(raw: String): String =
    if (raw.equals("sherpa", ignoreCase = true)) "kokoro" else raw
