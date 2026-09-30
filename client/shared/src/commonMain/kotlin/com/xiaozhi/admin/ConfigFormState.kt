package com.xiaozhi.admin

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
class ConfigFormState {

    // ===== [server] =====
    var listen by observable("")
    var expectedToken by observable("")
    var workerThreads by observable("2")
    var adminDir by observable("")

    // ===== [audio] =====
    var downlinkSampleRate by observable("24000")
    var downlinkFrameMs by observable("60")
    var channels by observable("1")
    var binaryProtocolVersion by observable("1")

    // ===== [asr] =====
    var asrBackend by observable("mock")
    var asrModel by observable("")
    var asrTokens by observable("")
    var asrLanguage by observable("auto")
    var asrUseItn by observable("true")
    var asrNumThreads by observable("2")
    var asrProvider by observable("cpu")

    // ===== [vad] =====
    var vadModel by observable("")
    var vadThreshold by observable("0.5")
    var vadMinSilence by observable("0.25")
    var vadMinSpeech by observable("0.25")

    // ===== [tts] =====
    var ttsBackend by observable("mock")
    var ttsModel by observable("")
    var ttsVoices by observable("")
    var ttsTokens by observable("")
    var ttsDataDir by observable("")
    var ttsDictDir by observable("")
    var ttsLexicon by observable("")
    var ttsLang by observable("zh")
    var ttsSpeaker by observable("0")
    var ttsSpeed by observable("1.0")
    var ttsNumThreads by observable("1")

    // ===== [llm] =====
    var llmBackend by observable("mock")
    var llmApiBase by observable("")
    var llmApiKey by observable("")
    var llmModel by observable("")
    var llmSystemPrompt by observable("")
    var llmMaxHistory by observable("10")
    var llmTemperature by observable("0.7")

    // ===== 状态 =====
    var dirty by observable(false)
    var saving by observable(false)
    var statusMsg by observable("")
    var lastSavedAt by observable("")

    // ============================================================
    // 网络：load / save
    // ============================================================

    private fun network(ctx: Pager): NetworkModule = ctx.acquireModule(NetworkModule.MODULE_NAME)

    fun load(ctx: Pager, baseUrl: String = "") {
        network(ctx).requestGet("${baseUrl}/api/config", JSONObject()) { data, success, errorMsg, _ ->
            if (success) {
                fill(data)
                statusMsg = "已加载配置"
            } else {
                statusMsg = "加载失败: $errorMsg"
            }
        }
    }

    fun save(ctx: Pager, baseUrl: String = "") {
        saving = true
        val body = JSONObject().apply {
            put("server", JSONObject().apply {
                put("listen", listen)
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
                put("backend", asrBackend)
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
                put("backend", ttsBackend)
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
                put("backend", llmBackend)
                put("api_base", llmApiBase)
                put("api_key", llmApiKey)
                put("model", llmModel)
                put("system_prompt", llmSystemPrompt)
                put("max_history", llmMaxHistory.toIntOrNull() ?: 10)
                put("temperature", llmTemperature.toDoubleOrNull() ?: 0.7)
            })
        }
        network(ctx).requestPost("${baseUrl}/api/config", body) { _, success, errorMsg, _ ->
            saving = false
            if (success) {
                dirty = false
                lastSavedAt = "已保存"
                statusMsg = "保存成功（引擎参数需重启生效）"
            } else {
                statusMsg = "保存失败: $errorMsg"
            }
        }
    }

    /**
     * 用服务端 /api/config 回包填充表单（公开：macOS ConnectState 连接流程自行 GET 后调用，
     * 以便同时提取 expected_token 供 WS 鉴权；load() 内部也走这里）。
     */
    fun fill(obj: JSONObject) {
        obj.optJSONObject("server")?.let { s ->
            listen = s.optString("listen", listen)
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
            asrBackend = a.optString("backend", asrBackend)
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
            ttsBackend = t.optString("backend", ttsBackend)
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
            llmBackend = l.optString("backend", llmBackend)
            llmApiBase = l.optString("api_base", llmApiBase)
            llmApiKey = l.optString("api_key", llmApiKey)
            llmModel = l.optString("model", llmModel)
            llmSystemPrompt = l.optString("system_prompt", llmSystemPrompt)
            llmMaxHistory = l.optInt("max_history", llmMaxHistory.toIntOrNull() ?: 10).toString()
            llmTemperature = l.optDouble("temperature", llmTemperature.toDoubleOrNull() ?: 0.7).toString()
        }
    }

    // ============================================================
    // 渲染：见文件底部 ViewContainer.renderForm(form) 扩展
    // ============================================================
}

/**
 * 渲染 6 张分组卡片（Server/Audio/ASR/VAD/TTS/LLM），经 cardGrid 响应式排版
 * （wide() 为 true 双列，false 单列）。
 * backend 用 segmentedControl，use_itn 用 switchRow，多行字段用 labeledField(height=100)。
 *
 * ⚠️ 必须在目标容器（Scroller）闭包内**非限定**调用 `renderForm(form, wide)`：
 * 扩展接收者即 Scroller，卡片才能正确挂进 Scroller。
 * ⚠️ wide 须为 lambda（如 `{ pagerData.pageViewWidth >= 900f }`），在 cardGrid 的
 * vif 闭包内读取才能随窗口 resize 响应式重排。
 */
fun ViewContainer<*, *>.renderForm(form: ConfigFormState, wide: () -> Boolean = { false }) {
    fun markDirty() {
        form.dirty = true
    }

    cardGrid(
        wide = wide,
        cards = listOf(
            AdminCard("Server") {
                labeledField("listen", { form.listen }, { form.listen = it; markDirty() }, "0.0.0.0:8000")
                labeledField("expected_token", { form.expectedToken }, { form.expectedToken = it; markDirty() })
                labeledField("worker_threads", { form.workerThreads }, { form.workerThreads = it; markDirty() })
                labeledField("admin_dir", { form.adminDir }, { form.adminDir = it; markDirty() })
            },
            AdminCard("Audio") {
                labeledField("downlink_sample_rate", { form.downlinkSampleRate }, { form.downlinkSampleRate = it; markDirty() })
                labeledField("downlink_frame_duration_ms", { form.downlinkFrameMs }, { form.downlinkFrameMs = it; markDirty() })
                labeledField("channels", { form.channels }, { form.channels = it; markDirty() })
                labeledField("binary_protocol_version", { form.binaryProtocolVersion }, { form.binaryProtocolVersion = it; markDirty() })
            },
            AdminCard("ASR") {
                fieldLabel("backend")
                val asrOpts = listOf("mock", "sherpa")
                segmentedControl(asrOpts, if (form.asrBackend == "sherpa") 1 else 0) { i ->
                    form.asrBackend = asrOpts[i]; markDirty()
                }
                labeledField("model", { form.asrModel }, { form.asrModel = it; markDirty() })
                labeledField("tokens", { form.asrTokens }, { form.asrTokens = it; markDirty() })
                labeledField("language", { form.asrLanguage }, { form.asrLanguage = it; markDirty() }, "auto/zh/en/ja/ko/yue")
                switchRow("use_itn", form.asrUseItn == "true") {
                    form.asrUseItn = if (form.asrUseItn == "true") "false" else "true"; markDirty()
                }
                labeledField("num_threads", { form.asrNumThreads }, { form.asrNumThreads = it; markDirty() })
                labeledField("provider", { form.asrProvider }, { form.asrProvider = it; markDirty() })
            },
            AdminCard("VAD") {
                labeledField("model", { form.vadModel }, { form.vadModel = it; markDirty() })
                labeledField("threshold", { form.vadThreshold }, { form.vadThreshold = it; markDirty() })
                labeledField("min_silence_duration", { form.vadMinSilence }, { form.vadMinSilence = it; markDirty() })
                labeledField("min_speech_duration", { form.vadMinSpeech }, { form.vadMinSpeech = it; markDirty() })
            },
            AdminCard("TTS") {
                fieldLabel("backend")
                val ttsOpts = listOf("mock", "sherpa")
                segmentedControl(ttsOpts, if (form.ttsBackend == "sherpa") 1 else 0) { i ->
                    form.ttsBackend = ttsOpts[i]; markDirty()
                }
                labeledField("model", { form.ttsModel }, { form.ttsModel = it; markDirty() })
                labeledField("voices", { form.ttsVoices }, { form.ttsVoices = it; markDirty() }, height = 100f)
                labeledField("tokens", { form.ttsTokens }, { form.ttsTokens = it; markDirty() })
                labeledField("data_dir", { form.ttsDataDir }, { form.ttsDataDir = it; markDirty() })
                labeledField("dict_dir", { form.ttsDictDir }, { form.ttsDictDir = it; markDirty() })
                labeledField("lexicon", { form.ttsLexicon }, { form.ttsLexicon = it; markDirty() }, height = 100f)
                labeledField("lang", { form.ttsLang }, { form.ttsLang = it; markDirty() })
                labeledField("speaker", { form.ttsSpeaker }, { form.ttsSpeaker = it; markDirty() })
                labeledField("speed", { form.ttsSpeed }, { form.ttsSpeed = it; markDirty() })
                labeledField("num_threads", { form.ttsNumThreads }, { form.ttsNumThreads = it; markDirty() })
            },
            AdminCard("LLM") {
                fieldLabel("backend")
                val llmOpts = listOf("mock", "http")
                segmentedControl(llmOpts, if (form.llmBackend == "http") 1 else 0) { i ->
                    form.llmBackend = llmOpts[i]; markDirty()
                }
                labeledField("api_base", { form.llmApiBase }, { form.llmApiBase = it; markDirty() })
                labeledField("api_key", { form.llmApiKey }, { form.llmApiKey = it; markDirty() })
                labeledField("model", { form.llmModel }, { form.llmModel = it; markDirty() })
                labeledField("system_prompt", { form.llmSystemPrompt }, { form.llmSystemPrompt = it; markDirty() }, height = 100f)
                labeledField("max_history", { form.llmMaxHistory }, { form.llmMaxHistory = it; markDirty() })
                labeledField("temperature", { form.llmTemperature }, { form.llmTemperature = it; markDirty() })
            },
        ),
    )
}
