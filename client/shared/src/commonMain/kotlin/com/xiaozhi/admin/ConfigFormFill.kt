package com.xiaozhi.admin

import com.xiaozhi.admin.ui.*

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
 * 用服务端 /api/config 回包填充表单（公开：macOS 端经 AdminPage.refreshAll 以绝对基址调用，
 * 以便同时提取 expected_token 供 WS 鉴权；load() 内部也走这里）。
 */
internal fun ConfigFormState.fill(obj: JSONObject) {
    revision = obj.optString("revision", revision)
    obj.optJSONObject("server")?.let { s ->
        port = s.intText("port", port, 8000)
        expectedToken = s.optString("expected_token", expectedToken)
        workerThreads = s.intText("worker_threads", workerThreads, 2)
        adminDir = s.optString("admin_dir", adminDir)
    }
    obj.optJSONObject("audio")?.let { a ->
        downlinkSampleRate = a.intText("downlink_sample_rate", downlinkSampleRate, 24000)
        downlinkFrameMs = a.intText("downlink_frame_duration_ms", downlinkFrameMs, 60)
        channels = a.intText("channels", channels, 1)
        binaryProtocolVersion = a.intText("binary_protocol_version", binaryProtocolVersion, 1)
        downlinkLeadMs = a.intText("downlink_lead_ms", downlinkLeadMs, 1000)
        downlinkQueuePackets = a.intText("downlink_queue_packets", downlinkQueuePackets, 20)
        downlinkBufferMs = a.intText("downlink_buffer_ms", downlinkBufferMs, 30000)
        downlinkBufferStartMs = a.intText("downlink_buffer_start_ms", downlinkBufferStartMs, 1500)
        downlinkBufferStartMaxMs = a.intText("downlink_buffer_start_max_ms", downlinkBufferStartMaxMs, 4000)
    }
    obj.optJSONObject("playback")?.let { p ->
        playbackMode = p.optString("mode", playbackMode)
        announcementStartMs = p.intText("announcement_start_ms", announcementStartMs, 5000)
        announcementStartMaxMs = p.intText("announcement_start_max_ms", announcementStartMaxMs, 10000)
    }
    obj.optJSONObject("asr")?.let { a ->
        asrModel = a.optString("model", asrModel)
        asrTokens = a.optString("tokens", asrTokens)
        asrLanguage = a.optString("language", asrLanguage)
        asrUseItn = a.boolText("use_itn", asrUseItn, true)
        asrNumThreads = a.intText("num_threads", asrNumThreads, 2)
        asrProvider = a.optString("provider", asrProvider)
    }
    obj.optJSONObject("vad")?.let { v ->
        vadModel = v.optString("model", vadModel)
        vadThreshold = v.doubleText("threshold", vadThreshold, 0.5)
        vadMinSilence = v.doubleText("min_silence_duration", vadMinSilence, 0.25)
        vadMinSpeech = v.doubleText("min_speech_duration", vadMinSpeech, 0.25)
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
        ttsSpeaker = t.intText("speaker", ttsSpeaker, 0)
        ttsSpeed = t.doubleText("speed", ttsSpeed, 1.0)
        ttsCacheEntries = t.intText("cache_entries", ttsCacheEntries, 256)
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
        llmEnabled = l.boolText("enabled", llmEnabled, true)
        llmApiBase = l.optString("api_base", llmApiBase)
        llmApiKey = l.optString("api_key", llmApiKey)
        llmApiKeyConfigured = l.optBoolean("has_api_key", llmApiKey.isNotBlank())
        llmModel = l.optString("model", llmModel)
        llmSystemPrompt = l.optString("system_prompt", llmSystemPrompt)
        llmMaxHistory = l.intText("max_history", llmMaxHistory, 10)
        llmTemperature = l.doubleText("temperature", llmTemperature, 0.7)
        llmStream = l.boolText("stream", llmStream, true)
    }
    obj.optJSONObject("aiui")?.let { a ->
        aiuiEnabled = a.boolText("enabled", aiuiEnabled, false)
        aiuiAppid = a.optString("appid", aiuiAppid)
        aiuiApiKey = a.optString("api_key", aiuiApiKey)
        aiuiApiSecret = a.optString("api_secret", aiuiApiSecret)
        aiuiApiKeyConfigured = a.optBoolean("has_api_key", aiuiApiKey.isNotBlank())
        aiuiApiSecretConfigured =
            a.optBoolean("has_api_secret", aiuiApiSecret.isNotBlank())
        aiuiScene = a.optString("scene", aiuiScene)
        aiuiSnPrefix = a.optString("sn_prefix", aiuiSnPrefix)
        aiuiVoice = a.optString("voice", aiuiVoice)
        aiuiSpeed = a.intText("speed", aiuiSpeed, 50)
        aiuiVolume = a.intText("volume", aiuiVolume, 50)
        aiuiPitch = a.intText("pitch", aiuiPitch, 50)
        aiuiPrompt = a.optString("prompt", aiuiPrompt)
        aiuiPaceMs = a.intText("pace_ms", aiuiPaceMs, 10)
    }
    // [soul] / [memory]（上下文生产者）：字段较多，映射集中在 ConfigContextState
    sm.fill(obj)
    // [command]（指令闸门）：映射同样集中在 ConfigCommandState
    cmd.fill(obj)
    // [wakeup]（唤醒问候语）：映射集中在 ConfigWakeupState
    wake.fill(obj)
    // [nlu]（技能级联）：映射集中在 ConfigNluState（含 intents/slots 嵌套数组）
    nlu.fill(obj)
    // [echomuse]（设备入口）：映射集中在 EchoMuseSectionState（token presence 打码）
    em.fill(obj)
    // 全部字段已就位：刷新内容栏重建键，强制表单整段重渲染——web 渲染器不会在异步
    // fill 后重 diff Input props，不重建则输入框停留在 build 时的初始值（误读成「保存的值丢了」）
    notifyConfigReloaded()
}

// ============================================================
// 字段读取助手（JSON 类型 → 表单文本）
// ============================================================
// 为什么需要：表单里每个字段都是 `observable(String)`（Input 只吃文本），而服务端回包是
// 强类型 JSON。于是每个数字/布尔字段的读取都要写一遍
// `optInt("k", 字段.toIntOrNull() ?: 默认值).toString()`——全仓 59 处，已全部改走这里。
// 那套写法的真正问题不是长：默认值在「字段初始值」和「读取兜底」各写一次，
// 改一处忘另一处，下次轮询就会被静默回滚成旧默认值。
// 这里只做**一件事的收敛**：把「缺键 → 保留当前值、类型不符 → 兜底默认值」这条既有语义
// 原样表达一次。⚠️ 语义与调用点手写完全等价（仍走同一个 `optInt/optDouble/optBoolean`），
// 不引入宽松解析，也不改成"缺失即清空"——部分更新语义见 AGENTS.md「表单与网络契约」。

/** 整数字段：缺键 → 保留 [current]；[current] 非数字 → [fallback]；结果转回表单文本。 */
internal fun JSONObject.intText(key: String, current: String, fallback: Int): String =
    optInt(key, current.toIntOrNull() ?: fallback).toString()

/** 浮点字段：同 [intText]，用于 `labeledDecimalField` 绑定的表单文本。 */
internal fun JSONObject.doubleText(key: String, current: String, fallback: Double): String =
    optDouble(key, current.toDoubleOrNull() ?: fallback).toString()

/** 布尔字段：表单里存 "true"/"false" 文本（`toString()` 的原样结果）。 */
internal fun JSONObject.boolText(key: String, current: String, fallback: Boolean): String =
    optBoolean(key, current.toBooleanStrictOrNull() ?: fallback).toString()

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
