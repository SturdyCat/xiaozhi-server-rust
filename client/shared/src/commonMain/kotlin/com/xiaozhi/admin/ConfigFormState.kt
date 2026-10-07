package com.xiaozhi.admin

import com.tencent.kuikly.core.base.PagerScope
import com.tencent.kuikly.core.base.ViewContainer
import com.tencent.kuikly.core.directives.vif
import com.tencent.kuikly.core.module.NetworkModule
import com.tencent.kuikly.core.nvi.serialization.json.JSONObject
import com.tencent.kuikly.core.pager.Pager
import com.tencent.kuikly.core.reactive.collection.ObservableList
import com.tencent.kuikly.core.reactive.handler.observable
import com.tencent.kuikly.core.reactive.handler.observableList
import com.tencent.kuikly.core.views.Text
import com.tencent.kuikly.core.views.View

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

    // ===== [tts] =====（区分「本地模型 / 远程服务」两大类，见 TTS_LOCAL/REMOTE 注册表）
    /// 当前合成方式："local"（本地模型，离线）| "remote"（远程服务，在线）。
    /// 与 [ttsBackend] 联动：切方式时 backend 跟随切到对应组的引擎/服务商。
    var ttsMode by scope.observable("local")
    /// 当前生效的引擎/服务商 id（本地组：sherpa…；远程组：xfyun…）。
    var ttsBackend by scope.observable("sherpa")
    /// 上次使用的远程服务商（切回「远程」时恢复，默认 xfyun）。
    var lastRemoteEngine by scope.observable("xfyun")
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

    // ===== [tts.xfyun] =====（backend=xfyun 时使用；讯飞开放平台「在线语音合成」控制台获取）
    var xfyunAppId by scope.observable("")
    var xfyunApiKey by scope.observable("")
    var xfyunApiSecret by scope.observable("")
    var xfyunVoice by scope.observable("xiaoyan")

    // AIUI 控制台会话（可选）：用于从平台接口拉取**已授权发音人目录**（浏览器登录抓包复制）。
    // 至少含 ssoSessionId / x-secure-token / JSESSIONID；会话过期后重新粘贴。
    var xfyunConsoleCookie by scope.observable("")
    var xfyunConsoleCsrf by scope.observable("")
    /** 拉取目录/刷新用的服务器地址（连接流程写入；空=当前页同源）。 */
    var serverBase by scope.observable("")
    /** 上次“刷新音色目录”的结果提示（动作按钮反馈）。 */
    var voicesRefreshMsg by scope.observable("")

    // 音色两级选择：性别分组（女/男/自定义）× 音色类型（全部/普通/极速拟人）。
    // 音色列表经 /api/tts/voices 从服务端动态获取（服务端为唯一数据源：实测目录+控制台元数据）。
    var xfyunVoiceGroup by scope.observable("female")
    val xfyunVoiceGroupOptions: ObservableList<Pair<String, String>> by scope.observableList()
    /// 音色类型筛选：all（全部）/ classic（普通发音人）/ x6（极速拟人）。
    var xfyunVoiceType by scope.observable("all")
    val xfyunVoiceTypeOptions: ObservableList<Pair<String, String>> by scope.observableList()
    /** 当前分组+类型下的音色选项（切分组/切类型/拉取目录时重建，官方 AlertDialog 要求 ObservableList）。 */
    val xfyunVoiceOptions: ObservableList<Pair<String, String>> by scope.observableList()
    // 四个源列表（loadVoices 填充）：性别 × 类型
    val voicesFemaleClassic: ObservableList<Pair<String, String>> by scope.observableList()
    val voicesFemaleX6: ObservableList<Pair<String, String>> by scope.observableList()
    val voicesMaleClassic: ObservableList<Pair<String, String>> by scope.observableList()
    val voicesMaleX6: ObservableList<Pair<String, String>> by scope.observableList()
    /** 目录是否已拉取（拉取前按 vcn 前缀启发式分组）。 */
    var voicesLoaded by scope.observable(false)

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

    /** 当前展开的下拉（TTS backend 选择等，"" = 全部收起）；放状态类避免 Pager body 重建丢失展开态。 */
    var openDropdown by scope.observable("")

    // ============================================================
    // TTS 引擎注册表（本地 / 远程两组；未来新增引擎/供应商只改这里 + 服务端）
    // ============================================================
    // 本地模型引擎（离线推理，凭据字段 = 模型路径等）。
    val ttsLocalEngines: ObservableList<Pair<String, String>> by scope.observableList()
    // 远程合成服务商（在线 API，凭据字段 = 各服务商密钥）。
    val ttsRemoteEngines: ObservableList<Pair<String, String>> by scope.observableList()
    // 合成方式两选项（local/remote）。
    val ttsModeOptions: ObservableList<Pair<String, String>> by scope.observableList()

    init {
        ttsModeOptions.addAll(
            listOf(
                "local" to "本地模型（离线合成）",
                "remote" to "远程服务（在线 API）",
            ),
        )
        ttsLocalEngines.addAll(listOf("sherpa" to "本地 Kokoro INT8（离线）"))
        xfyunVoiceGroupOptions.addAll(
            listOf(
                "female" to "女声",
                "male" to "男声",
                "custom" to "自定义（手填 vcn）",
            ),
        )
        xfyunVoiceTypeOptions.addAll(
            listOf(
                "all" to "全部发音人",
                "classic" to "普通发音人",
                "x6" to "极速拟人",
            ),
        )
        reloadXfyunVoiceOptions()
        ttsRemoteEngines.addAll(
            listOf(
                "xfyun" to "科大讯飞（在线）",
                // 未来供应商在此追加：如 "azure" to "Azure TTS（在线）"
            ),
        )
    }

    /** 引擎 id 是否属于远程服务商组。 */
    fun ttsEngineIsRemote(id: String): Boolean =
        ttsRemoteEngines.any { it.first == id }

    /** 按性别分组 + 音色类型筛选重建下拉选项（列表来自 /api/tts/voices 拉取结果）。 */
    fun reloadXfyunVoiceOptions() {
        xfyunVoiceOptions.clear()
        if (xfyunVoiceGroup == "custom") return
        val classic = if (xfyunVoiceGroup == "male") voicesMaleClassic else voicesFemaleClassic
        val x6 = if (xfyunVoiceGroup == "male") voicesMaleX6 else voicesFemaleX6
        when (xfyunVoiceType) {
            "classic" -> xfyunVoiceOptions.addAll(classic)
            "x6" -> xfyunVoiceOptions.addAll(x6)
            else -> {
                xfyunVoiceOptions.addAll(classic)
                xfyunVoiceOptions.addAll(x6)
            }
        }
    }

    /** 切换音色类型筛选：若当前 vcn 不在筛选结果里则回落第一个。 */
    fun selectXfyunVoiceType(t: String) {
        xfyunVoiceType = t
        reloadXfyunVoiceOptions()
        val ids = xfyunVoiceOptions.map { it.first }
        if (xfyunVoice !in ids) {
            xfyunVoice = ids.firstOrNull() ?: xfyunVoice
            dirty = true
        }
    }

    /** 切换音色分组：非自定义分组时若当前 vcn 不在该组，回落到该组第一个。 */
    fun selectXfyunVoiceGroup(g: String) {
        xfyunVoiceGroup = g
        reloadXfyunVoiceOptions()
        if (g != "custom") {
            val ids = xfyunVoiceOptions.map { it.first }
            if (xfyunVoice !in ids) {
                xfyunVoice = ids.firstOrNull() ?: xfyunVoice
                dirty = true
            }
        }
    }

    /** 依已拉取目录反推当前 vcn 的性别分组与音色类型；目录未拉取时按前缀启发。 */
    private fun deriveVoiceGroupAndType() {
        val v = xfyunVoice
        val inList: (ObservableList<Pair<String, String>>) -> Boolean = { l -> l.any { it.first == v } }
        when {
            inList(voicesFemaleClassic) -> { xfyunVoiceGroup = "female"; xfyunVoiceType = "classic" }
            inList(voicesFemaleX6) -> { xfyunVoiceGroup = "female"; xfyunVoiceType = "x6" }
            inList(voicesMaleClassic) -> { xfyunVoiceGroup = "male"; xfyunVoiceType = "classic" }
            inList(voicesMaleX6) -> { xfyunVoiceGroup = "male"; xfyunVoiceType = "x6" }
            !voicesLoaded -> {
                // 目录未就绪：前缀启发，避免把极速拟人音色误落「自定义」
                if (v.startsWith("x5_") || v.startsWith("x6_")) {
                    xfyunVoiceGroup = "female"; xfyunVoiceType = "x6"
                }
            }
            else -> xfyunVoiceGroup = "custom"
        }
    }

    /**
     * 触发服务端从 AIUI 平台**刷新**已授权发音人目录（POST /api/tts/voices/refresh），
     * 成功后重拉 `GET /api/tts/voices` 更新下拉。需先在下方填写控制台会话并保存配置。
     */
    fun refreshVoices(ctx: Pager) {
        voicesRefreshMsg = "刷新中…"
        val headers = JSONObject().apply { put("Content-Type", "application/json") }
        network(ctx).httpRequest("${serverBase}/api/tts/voices/refresh", true, JSONObject(), headers) { _, success, errorMsg, resp ->
            val code = resp.statusCode
            if (success && (code == null || code in 200..299)) {
                voicesRefreshMsg = "已刷新"
                loadVoices(ctx, serverBase)
            } else {
                voicesRefreshMsg = "刷新失败: ${if (errorMsg.isNotEmpty()) errorMsg else "HTTP $code"}（检查控制台会话是否过期）"
            }
        }
    }

    /** 拉取服务端音色目录（GET /api/tts/voices）并按性别×类型重建下拉。 */
    fun loadVoices(ctx: Pager, baseUrl: String = "") {
        serverBase = baseUrl // 供「刷新音色目录」按钮复用（macApp 连接流程不经 load()）
        network(ctx).requestGet("${baseUrl}/api/tts/voices", JSONObject()) { data, success, _, _ ->
            if (!success) return@requestGet
            val arr = data?.optJSONArray("voices") ?: return@requestGet
            voicesFemaleClassic.clear()
            voicesFemaleX6.clear()
            voicesMaleClassic.clear()
            voicesMaleX6.clear()
            for (i in 0 until arr.length()) {
                val v = arr.optJSONObject(i) ?: continue
                val vcn = v.optString("vcn")
                val name = v.optString("name")
                val tag = v.optString("tag")
                val label = if (tag.isEmpty()) name else "$name（$tag）"
                val type = v.optString("type")
                val pair = vcn to label
                when (v.optString("gender") to type) {
                    "female" to "classic" -> voicesFemaleClassic.add(pair)
                    "female" to "x6" -> voicesFemaleX6.add(pair)
                    "male" to "classic" -> voicesMaleClassic.add(pair)
                    else -> voicesMaleX6.add(pair)
                }
            }
            voicesLoaded = true
            // 目录到位后反推当前配置音色的分组与类型（fill 时目录可能尚未拉取）
            deriveVoiceGroupAndType()
            reloadXfyunVoiceOptions()
        }
    }

    // ============================================================
    // 网络：load / save
    // ============================================================

    private fun network(ctx: Pager): NetworkModule = ctx.acquireModule(NetworkModule.MODULE_NAME)

    fun load(ctx: Pager, baseUrl: String = "") {
        serverBase = baseUrl
        network(ctx).requestGet("${baseUrl}/api/config", JSONObject()) { data, success, errorMsg, _ ->
            if (success) {
                fill(data)
                loadVoices(ctx, baseUrl)
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
                put("xfyun", JSONObject().apply {
                    put("app_id", xfyunAppId)
                    put("api_key", xfyunApiKey)
                    put("api_secret", xfyunApiSecret)
                    put("voice", xfyunVoice)
                    put("console_cookie", xfyunConsoleCookie)
                    put("console_csrf", xfyunConsoleCsrf)
                })
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
            ttsBackend = t.optString("backend", ttsBackend)
            ttsMode = if (ttsEngineIsRemote(ttsBackend)) "remote" else "local"
            if (ttsMode == "remote") lastRemoteEngine = ttsBackend // 记住远程服务商，切回时恢复
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
            t.optJSONObject("xfyun")?.let { x ->
                xfyunAppId = x.optString("app_id", xfyunAppId)
                xfyunApiKey = x.optString("api_key", xfyunApiKey)
                xfyunApiSecret = x.optString("api_secret", xfyunApiSecret)
                xfyunVoice = x.optString("voice", xfyunVoice)
                xfyunConsoleCookie = x.optString("console_cookie", xfyunConsoleCookie)
                xfyunConsoleCsrf = x.optString("console_csrf", xfyunConsoleCsrf)
                // 按已存音色反推分组：不在预置目录 → 自定义（手填）
                deriveVoiceGroupAndType()
                reloadXfyunVoiceOptions()
            }
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
 * [tts].backend（sherpa/xfyun）用官方 AlertDialog 基座下拉（dropdownField），use_itn 用官方 switchRow，多行散文用 labeledTextArea（官方 TextArea）。
 *
 * ⚠️ 必须在目标容器闭包内**非限定**调用 `renderForm(form)`：
 * 扩展接收者即调用处容器，Tabs/PageList 才能正确挂进去。
 * 各配置卡（serverConfigCard 等）同时被测试台对应 tab 复用（AdminShell/testbench 右栏）。
 */
fun ViewContainer<*, *>.renderForm(
    form: ConfigFormState,
    ctx: Pager,
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
            TabPage("TTS") { ttsConfigCard(form, ctx) },
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

fun ViewContainer<*, *>.ttsConfigCard(form: ConfigFormState, ctx: Pager) {
    groupedCard("TTS") {
        // ===== 第一级：合成方式（本地模型=离线 / 远程服务=在线）=====
        // 官方 AlertDialog 基座下拉；切方式即联动切换引擎/服务商组与下方字段。
        dropdownField(
            label = "合成方式",
            currentLabel = {
                if (form.ttsMode == "remote") "远程服务（在线 API）" else "本地模型（离线合成）"
            },
            options = { form.ttsModeOptions },
            selectedId = { form.ttsMode },
            isOpen = { form.openDropdown == "tts_mode" },
            onToggle = { form.openDropdown = if (form.openDropdown == "tts_mode") "" else "tts_mode" },
            onSelect = { m ->
                if (m != form.ttsMode) {
                    form.ttsMode = m
                    // 切方式即切引擎组：本地→当前本地引擎；远程→上次使用的远程服务商
                    form.ttsBackend = if (m == "remote") form.lastRemoteEngine else "sherpa"
                    form.dirty = true
                    form.openDropdown = ""
                }
            },
        )
        // ===== 第二级：引擎 / 服务商（随方式联动）+ 各自字段 =====
        vif({ form.ttsMode == "local" }) {
            dropdownField(
                label = "本地引擎",
                currentLabel = {
                    form.ttsLocalEngines.firstOrNull { it.first == form.ttsBackend }?.second
                        ?: "本地 Kokoro INT8（离线）"
                },
                options = { form.ttsLocalEngines },
                selectedId = { form.ttsBackend },
                isOpen = { form.openDropdown == "tts_local_engine" },
                onToggle = { form.openDropdown = if (form.openDropdown == "tts_local_engine") "" else "tts_local_engine" },
                onSelect = {
                    form.ttsBackend = it
                    form.dirty = true
                    form.openDropdown = ""
                },
            )
            // 本地模型参数（sherpa/Kokoro）
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
        vif({ form.ttsMode == "remote" }) {
            dropdownField(
                label = "服务商",
                currentLabel = {
                    form.ttsRemoteEngines.firstOrNull { it.first == form.ttsBackend }?.second
                        ?: "科大讯飞（在线）"
                },
                options = { form.ttsRemoteEngines },
                selectedId = { form.ttsBackend },
                isOpen = { form.openDropdown == "tts_remote_engine" },
                onToggle = { form.openDropdown = if (form.openDropdown == "tts_remote_engine") "" else "tts_remote_engine" },
                onSelect = {
                    form.ttsBackend = it
                    form.lastRemoteEngine = it
                    form.dirty = true
                    form.openDropdown = ""
                },
            )
            Text {
                attr {
                    fontSize(AdminType.caption)
                    color(AdminColors.textSecondary)
                    text("远程合成需网络与对应服务商账号；保存后新会话/测试台立即用新引擎（无需重启）。")
                }
            }
            // 讯飞凭据与音色（backend=xfyun；未来其他供应商在此按 id 追加各自字段区）
            vif({ form.ttsBackend == "xfyun" }) {
                dividerH()
                Text {
                    attr {
                        fontSize(AdminType.caption)
                        color(AdminColors.textSecondary)
                        text("科大讯飞在线合成（在讯飞开放平台「在线语音合成」控制台获取密钥）")
                    }
                }
                labeledField("app_id", { form.xfyunAppId }, { form.xfyunAppId = it; form.dirty = true })
                labeledField("api_key", { form.xfyunApiKey }, { form.xfyunApiKey = it; form.dirty = true })
                labeledField("api_secret", { form.xfyunApiSecret }, { form.xfyunApiSecret = it; form.dirty = true })
                // AIUI 控制台会话（可选）：发音人目录动态拉取用；浏览器登录 aiui.xfyun.cn →
                // F12 Network → 任意请求 → 复制 Cookie 整串 / X-Csrf-Token 请求头
                labeledField("console_cookie（可选）", { form.xfyunConsoleCookie }, { form.xfyunConsoleCookie = it; form.dirty = true })
                labeledField("console_csrf（可选）", { form.xfyunConsoleCsrf }, { form.xfyunConsoleCsrf = it; form.dirty = true })
                actionRow {
                    secondaryButton("刷新音色目录") { form.refreshVoices(ctx) }
                    vif({ form.voicesRefreshMsg.isNotEmpty() }) {
                        View { attr { width(AdminSpace.sm) } }
                        Text {
                            attr {
                                fontSize(AdminType.micro)
                                color(AdminColors.textTertiary)
                                text(form.voicesRefreshMsg)
                            }
                        }
                    }
                }
                // 音色：分组下拉（女声/男声/自定义）→ 发音人下拉；完整清单以控制台为准
                dropdownField(
                    label = "音色分组",
                    currentLabel = {
                        when (form.xfyunVoiceGroup) {
                            "male" -> "男声"
                            "custom" -> "自定义（手填 vcn）"
                            else -> "女声"
                        }
                    },
                    options = { form.xfyunVoiceGroupOptions },
                    selectedId = { form.xfyunVoiceGroup },
                    isOpen = { form.openDropdown == "xfyun_voice_group" },
                    onToggle = { form.openDropdown = if (form.openDropdown == "xfyun_voice_group") "" else "xfyun_voice_group" },
                    onSelect = {
                        form.selectXfyunVoiceGroup(it)
                        form.openDropdown = ""
                    },
                )
                vif({ form.xfyunVoiceGroup != "custom" }) {
                    dropdownField(
                        label = "音色类型",
                        currentLabel = {
                            when (form.xfyunVoiceType) {
                                "classic" -> "普通发音人"
                                "x6" -> "极速拟人"
                                else -> "全部发音人"
                            }
                        },
                        options = { form.xfyunVoiceTypeOptions },
                        selectedId = { form.xfyunVoiceType },
                        isOpen = { form.openDropdown == "xfyun_voice_type" },
                        onToggle = { form.openDropdown = if (form.openDropdown == "xfyun_voice_type") "" else "xfyun_voice_type" },
                        onSelect = {
                            form.selectXfyunVoiceType(it)
                            form.openDropdown = ""
                        },
                    )
                    dropdownField(
                        label = "发音人（vcn）",
                        currentLabel = {
                            form.xfyunVoiceOptions.firstOrNull { it.first == form.xfyunVoice }?.second
                                ?: form.xfyunVoice
                        },
                        options = { form.xfyunVoiceOptions },
                        selectedId = { form.xfyunVoice },
                        isOpen = { form.openDropdown == "xfyun_voice" },
                        onToggle = { form.openDropdown = if (form.openDropdown == "xfyun_voice") "" else "xfyun_voice" },
                        onSelect = {
                            form.xfyunVoice = it
                            form.dirty = true
                            form.openDropdown = ""
                        },
                    )
                }
                vif({ form.xfyunVoiceGroup == "custom" }) {
                    labeledField("voice（手填 vcn）", { form.xfyunVoice }, { form.xfyunVoice = it; form.dirty = true })
                }
            }
        }
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
