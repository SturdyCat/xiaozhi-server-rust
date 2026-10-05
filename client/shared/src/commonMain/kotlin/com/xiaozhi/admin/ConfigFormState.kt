package com.xiaozhi.admin

import com.tencent.kuikly.core.base.PagerScope
import com.tencent.kuikly.core.base.ViewContainer
import com.tencent.kuikly.core.module.NetworkModule
import com.tencent.kuikly.core.nvi.serialization.json.JSONObject
import com.tencent.kuikly.core.pager.Pager
import com.tencent.kuikly.core.reactive.handler.observable

/**
 * 配置表单状态（跨端，commonMain，无平台依赖）。
 *
 * 持有 server / audio / asr / vad / tts / llm 全部字段的 observable（与 ConfigPage 原字段一致），
 * 外加 dirty / saving / statusMsg / lastSavedAt。
 * - load(ctx) / save(ctx)：经 NetworkModule 拉取 / 写回 server 的 GET|POST /api/config
 *   （acquireModule 是 Pager 方法，故 load/save 接收 ctx: Pager）。
 * - baseUrl：请求基址前缀。web（h5App）由 server 同域托管，保持默认 "" → 相对路径 /api/config；
 *   macOS 测试台跨机访问时由 ConnectState 传入已连接服务器的绝对基址（如 http://192.168.1.10:8000）。
 * - 任意字段变更时 dirty=true（在 onChange 里设）。
 * - 渲染见文件底部的 ViewContainer.renderForm(form) 扩展。
 *
 * ⚠️ Kuikly DSL 父子绑定机制：View{} 等组件是 ViewContainer 的扩展函数，静态绑定到
 * 「词法作用域最近的接收者」。因此渲染函数必须是 ViewContainer 扩展、并在目标容器
 * （Scroller）闭包内**非限定**调用；若经 `ctx.groupedCard(...)` 调用，接收者是 Pager 根容器，
 * 节点会逃逸出 Scroller，导致布局错乱。
 */
class ConfigFormState(private val scope: PagerScope) {

    // ===== [server] =====
    var port by scope.observable("8000")
    var expectedToken by scope.observable("")
    var workerThreads by scope.observable("2")
    var adminDir by scope.observable("")

    // ===== [audio] =====
    var downlinkSampleRate by scope.observable("24000")
    var downlinkFrameMs by scope.observable("60")
    var channels by scope.observable("1")
    var binaryProtocolVersion by scope.observable("1")

    // ===== [asr] =====（无 mock：ASR 恒为 SenseVoice，无 backend 字段）
    var asrModel by scope.observable("")
    var asrTokens by scope.observable("")
    var asrLanguage by scope.observable("auto")
    var asrUseItn by scope.observable("true")
    var asrNumThreads by scope.observable("2")
    var asrProvider by scope.observable("cpu")

    // ===== [vad] =====
    var vadModel by scope.observable("")
    var vadThreshold by scope.observable("0.5")
    var vadMinSilence by scope.observable("0.25")
    var vadMinSpeech by scope.observable("0.25")

    // ===== [tts] =====（无 mock：TTS 恒为 Kokoro，无 backend 字段）
    var ttsModel by scope.observable("")
    var ttsVoices by scope.observable("")
    var ttsTokens by scope.observable("")
    var ttsDataDir by scope.observable("")
    var ttsDictDir by scope.observable("")
    var ttsLexicon by scope.observable("")
    var ttsLang by scope.observable("zh")
    var ttsSpeaker by scope.observable("0")
    var ttsSpeed by scope.observable("1.0")
    var ttsNumThreads by scope.observable("1")

    // ===== [llm] =====（无 mock：LLM 恒为 OpenAI 兼容 HTTP，无 backend 字段）
    var llmApiBase by scope.observable("")
    var llmApiKey by scope.observable("")
    var llmModel by scope.observable("")
    var llmSystemPrompt by scope.observable("")
    var llmMaxHistory by scope.observable("10")
    var llmTemperature by scope.observable("0.7")
    var llmStream by scope.observable("true")

    // ===== 状态 =====
    var dirty by scope.observable(false)
    var saving by scope.observable(false)
    var statusMsg by scope.observable("")
    var statusLevel by scope.observable("info")
    var lastSavedAt by scope.observable("")

    /** 配置页标签页 UI 状态（Server/Audio/ASR/VAD/TTS/LLM 六个 tab，见 renderForm）。 */
    val tabUi = TabUiState(scope)

    // ============================================================
    // 网络：load / save
    // ============================================================

    private fun network(ctx: Pager): NetworkModule = ctx.acquireModule(NetworkModule.MODULE_NAME)

    fun load(ctx: Pager, baseUrl: String = "") {
        network(ctx).requestGet("${baseUrl}/api/config", JSONObject()) { data, success, errorMsg, _ ->
            if (success) {
                fill(data)
                statusMsg = "已加载配置"
                statusLevel = "ok"
            } else {
                statusMsg = "加载失败: $errorMsg"
                statusLevel = "error"
            }
        }
    }

    fun save(ctx: Pager, baseUrl: String = "") {
        saving = true
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
                put("model", ttsModel)
                put("voices", ttsVoices)
                put("tokens", ttsTokens)
                put("data_dir", ttsDataDir)
                put("dict_dir", ttsDictDir)
                put("lexicon", ttsLexicon)
                put("lang", ttsLang)
                put("speaker", ttsSpeaker.toIntOrNull() ?: 0)
                put("speed", ttsSpeed.toDoubleOrNull() ?: 1.0)
                put("num_threads", ttsNumThreads.toIntOrNull() ?: 1)
            })
            put("llm", JSONObject().apply {
                put("api_base", llmApiBase)
                put("api_key", llmApiKey)
                put("model", llmModel)
                put("system_prompt", llmSystemPrompt)
                put("max_history", llmMaxHistory.toIntOrNull() ?: 10)
                put("temperature", llmTemperature.toDoubleOrNull() ?: 0.7)
                // ⚠️ stream 必须回传：serde 端有 default，漏传会被重置为 true，
                // 手工在 TOML 里设的 stream=false 经 UI 保存一次就会丢。
                put("stream", llmStream.toBooleanStrictOrNull() ?: true)
            })
        }
        // ⚠️ 必须显式带 Content-Type: application/json：
        // Kuikly 的 requestPost 默认 headers=null，原生 KRHttpRequestTool 在非 JSON
        // Content-Type 下会把 body 编码成 form-urlencoded —— 服务端 Json<Config> 提取器
        // 直接 415（实测报 "Expected request with `Content-Type: application/json`"，
        // macApp 表现为「保存配置」失败）。同时按 HTTP 状态码判定成功：
        // 传输层 success 且 statusCode 非 2xx（如 415/400）时给出明确错误。
        val headers = JSONObject().apply { put("Content-Type", "application/json") }
        network(ctx).httpRequest("${baseUrl}/api/config", true, body, headers) { _, success, errorMsg, resp ->
            saving = false
            val code = resp.statusCode
            if (success && (code == null || code in 200..299)) {
                dirty = false
                lastSavedAt = "已保存"
                statusMsg = "保存成功（引擎参数需重启生效）"
                statusLevel = "ok"
            } else {
                statusMsg = "保存失败: ${if (errorMsg.isNotEmpty()) errorMsg else "HTTP $code"}"
                statusLevel = "error"
            }
        }
    }

    /**
     * 用服务端 /api/config 回包填充表单（公开：macOS ConnectState 连接流程自行 GET 后调用，
     * 以便同时提取 expected_token 供 WS 鉴权；load() 内部也走这里）。
     */
    fun fill(obj: JSONObject) {
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
            ttsModel = t.optString("model", ttsModel)
            ttsVoices = t.optString("voices", ttsVoices)
            ttsTokens = t.optString("tokens", ttsTokens)
            ttsDataDir = t.optString("data_dir", ttsDataDir)
            ttsDictDir = t.optString("dict_dir", ttsDictDir)
            ttsLexicon = t.optString("lexicon", ttsLexicon)
            ttsLang = t.optString("lang", ttsLang)
            ttsSpeaker = t.optInt("speaker", ttsSpeaker.toIntOrNull() ?: 0).toString()
            ttsSpeed = t.optDouble("speed", ttsSpeed.toDoubleOrNull() ?: 1.0).toString()
            ttsNumThreads = t.optInt("num_threads", ttsNumThreads.toIntOrNull() ?: 1).toString()
        }
        obj.optJSONObject("llm")?.let { l ->
            llmApiBase = l.optString("api_base", llmApiBase)
            llmApiKey = l.optString("api_key", llmApiKey)
            llmModel = l.optString("model", llmModel)
            llmSystemPrompt = l.optString("system_prompt", llmSystemPrompt)
            llmMaxHistory = l.optInt("max_history", llmMaxHistory.toIntOrNull() ?: 10).toString()
            llmTemperature = l.optDouble("temperature", llmTemperature.toDoubleOrNull() ?: 0.7).toString()
            llmStream = l.optBoolean("stream", llmStream.toBooleanStrictOrNull() ?: true).toString()
        }
    }

    // ============================================================
    // 渲染：见文件底部 ViewContainer.renderForm(form) 扩展
    // ============================================================
}

/**
 * 渲染配置标签页（tabbedPanel：官方 Tabs + PageList）：Server / Audio / ASR / VAD / TTS / LLM 六个 tab，
 * 保持全量配置汇总——每个 tab 一张配置卡，宽窄屏均整卡铺满（tab 内自带纵向滚动）。
 * backend 用官方 AlertDialog 基座下拉（dropdownField），use_itn 用官方 switchRow，多行散文用 labeledTextArea（官方 TextArea）。
 *
 * ⚠️ 必须在目标容器闭包内**非限定**调用 `renderForm(form)`：
 * 扩展接收者即调用处容器，Tabs/PageList 才能正确挂进去。
 * 各配置卡（serverConfigCard 等）同时被测试台对应 tab 复用（AdminShell/testbench 右栏）。
 */
fun ViewContainer<*, *>.renderForm(
    form: ConfigFormState,
    pageWidth: () -> Float,
    pageHeight: () -> Float,
) {
    tabbedPanel(
        ui = form.tabUi,
        pageWidth = pageWidth,
        pageHeight = pageHeight,
        pages = listOf(
            TabPage("Server") { serverConfigCard(form) },
            TabPage("Audio") { audioConfigCard(form) },
            TabPage("ASR") { asrConfigCard(form) },
            TabPage("VAD") { vadConfigCard(form) },
            TabPage("TTS") { ttsConfigCard(form) },
            TabPage("LLM") { llmConfigCard(form) },
        ),
    )
}

// ============================================================
// 各配置分组卡（renderForm 与测试台 tab 右栏共用；字段变更统一 markDirty）
// ============================================================

fun ViewContainer<*, *>.serverConfigCard(form: ConfigFormState) {
    groupedCard("Server") {
        labeledField("port", { form.port }, { form.port = it; form.dirty = true }, "8000")
        labeledField("expected_token", { form.expectedToken }, { form.expectedToken = it; form.dirty = true })
        labeledField("worker_threads", { form.workerThreads }, { form.workerThreads = it; form.dirty = true })
        labeledField("admin_dir", { form.adminDir }, { form.adminDir = it; form.dirty = true })
    }
}

fun ViewContainer<*, *>.audioConfigCard(form: ConfigFormState) {
    groupedCard("Audio") {
        labeledField("downlink_sample_rate", { form.downlinkSampleRate }, { form.downlinkSampleRate = it; form.dirty = true })
        labeledField("downlink_frame_duration_ms", { form.downlinkFrameMs }, { form.downlinkFrameMs = it; form.dirty = true })
        labeledField("channels", { form.channels }, { form.channels = it; form.dirty = true })
        labeledField("binary_protocol_version", { form.binaryProtocolVersion }, { form.binaryProtocolVersion = it; form.dirty = true })
    }
}

fun ViewContainer<*, *>.asrConfigCard(form: ConfigFormState) {
    groupedCard("ASR") {
        // 无 backend 选择：ASR 恒为 SenseVoice（sherpa），mock 已移除
        labeledField("model", { form.asrModel }, { form.asrModel = it; form.dirty = true })
        labeledField("tokens", { form.asrTokens }, { form.asrTokens = it; form.dirty = true })
        labeledField("language", { form.asrLanguage }, { form.asrLanguage = it; form.dirty = true }, "auto/zh/en/ja/ko/yue")
        switchRow("use_itn", form.asrUseItn == "true") {
            form.asrUseItn = if (form.asrUseItn == "true") "false" else "true"; form.dirty = true
        }
        labeledField("num_threads", { form.asrNumThreads }, { form.asrNumThreads = it; form.dirty = true })
        labeledField("provider", { form.asrProvider }, { form.asrProvider = it; form.dirty = true })
    }
}

fun ViewContainer<*, *>.vadConfigCard(form: ConfigFormState) {
    groupedCard("VAD") {
        labeledField("model", { form.vadModel }, { form.vadModel = it; form.dirty = true })
        labeledField("threshold", { form.vadThreshold }, { form.vadThreshold = it; form.dirty = true })
        labeledField("min_silence_duration", { form.vadMinSilence }, { form.vadMinSilence = it; form.dirty = true })
        labeledField("min_speech_duration", { form.vadMinSpeech }, { form.vadMinSpeech = it; form.dirty = true })
    }
}

fun ViewContainer<*, *>.ttsConfigCard(form: ConfigFormState) {
    groupedCard("TTS") {
        // 无 backend 选择：TTS 恒为 Kokoro（sherpa），mock 已移除
        labeledField("model", { form.ttsModel }, { form.ttsModel = it; form.dirty = true })
        labeledField("voices", { form.ttsVoices }, { form.ttsVoices = it; form.dirty = true })
        labeledField("tokens", { form.ttsTokens }, { form.ttsTokens = it; form.dirty = true })
        labeledField("data_dir", { form.ttsDataDir }, { form.ttsDataDir = it; form.dirty = true })
        labeledField("dict_dir", { form.ttsDictDir }, { form.ttsDictDir = it; form.dirty = true })
        labeledField("lexicon", { form.ttsLexicon }, { form.ttsLexicon = it; form.dirty = true })
        labeledField("lang", { form.ttsLang }, { form.ttsLang = it; form.dirty = true })
        labeledField("speaker", { form.ttsSpeaker }, { form.ttsSpeaker = it; form.dirty = true })
        labeledField("speed", { form.ttsSpeed }, { form.ttsSpeed = it; form.dirty = true })
        labeledField("num_threads", { form.ttsNumThreads }, { form.ttsNumThreads = it; form.dirty = true })
    }
}

fun ViewContainer<*, *>.llmConfigCard(form: ConfigFormState) {
    groupedCard("LLM") {
        // 无 backend 选择：LLM 恒为 OpenAI 兼容 HTTP（Responses API），mock 已移除
        labeledField("api_base", { form.llmApiBase }, { form.llmApiBase = it; form.dirty = true })
        labeledField("api_key", { form.llmApiKey }, { form.llmApiKey = it; form.dirty = true })
        labeledField("model", { form.llmModel }, { form.llmModel = it; form.dirty = true })
        // 系统提示词是散文输入：官方 TextArea 多行（此前用单行 Input 输不了换行）
        labeledTextArea("system_prompt", { form.llmSystemPrompt }, { form.llmSystemPrompt = it; form.dirty = true }, height = 100f)
        labeledField("max_history", { form.llmMaxHistory }, { form.llmMaxHistory = it; form.dirty = true })
        labeledField("temperature", { form.llmTemperature }, { form.llmTemperature = it; form.dirty = true })
        switchRow("stream（SSE 流式）", form.llmStream == "true") {
            form.llmStream = if (form.llmStream == "true") "false" else "true"; form.dirty = true
        }
    }
}
