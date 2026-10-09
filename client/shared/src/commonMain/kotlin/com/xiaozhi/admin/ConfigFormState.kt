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
import com.tencent.kuikly.core.timer.setTimeout
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
    /// 下行抖动缓冲提前量（毫秒）：下发时间表比实时提前该时长，使 ESP 解码队列常备音频存货。
    /// ⚠️ 上限建议 ≤400ms（固件解码队列 20 包 ≈1.2s；越大打断响应越迟钝）。
    var downlinkLeadMs by scope.observable("240")

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
    /// 当前生效的引擎/服务商 id（= 服务端 `[tts].engine`；本地组：kokoro…；远程组：xfyun…）。
    /// ⚠️ P4 前本地组叫 "sherpa"（旧 config.toml 的 `backend` 值），服务端仍认，这里统一为 "kokoro"。
    var ttsBackend by scope.observable("kokoro")
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
    var ttsNumThreads by scope.observable("4")
    /// TTS 结果缓存条目数（按句缓存已编码下行 Opus 帧，跨会话共享；命中零合成延迟）。0 = 关闭。
    var ttsCacheEntries by scope.observable("256")

    // ===== [tts.xfyun] =====（engine=xfyun 时使用；讯飞开放平台「在线语音合成」控制台获取）
    var xfyunAppId by scope.observable("")
    var xfyunApiKey by scope.observable("")
    var xfyunApiSecret by scope.observable("")
    /**
     * 密钥 **presence**：服务端 GET /api/config 已打码，只回 `has_api_key` / `has_api_secret`。
     * 明文永不进表单，输入框留空 = 「不修改盘上已存值」；为 true 时提示「已配置」。
     */
    var xfyunApiKeyConfigured by scope.observable(false)
    var xfyunApiSecretConfigured by scope.observable(false)
    var xfyunVoice by scope.observable("xiaoyan")

    /** 拉取目录/刷新用的服务器地址（连接流程写入；空=当前页同源）。 */
    var serverBase by scope.observable("")
    /** 上次“刷新音色目录”的结果提示（动作按钮反馈）。 */
    var voicesRefreshMsg by scope.observable("")
    /** 上次“测试凭据”的结果提示（动作按钮反馈）。 */
    var credsTestMsg by scope.observable("")
    var credsTestOk by scope.observable(false)

    /** 配置来源提示（配置文件路径 + 持久化状态；Server 配置卡展示）。 */
    var configMetaMsg by scope.observable("")

    /** 讯飞三要素是否就绪——决定依赖凭据的音色选择区是否显示。
     *  密钥可能只以 presence 形式存在（打码后表单里是空的），此时同样算就绪。 */
    fun xfyunCredsReady(): Boolean =
        xfyunAppId.isNotBlank() &&
            (xfyunApiKey.isNotBlank() || xfyunApiKeyConfigured) &&
            (xfyunApiSecret.isNotBlank() || xfyunApiSecretConfigured)

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
    /** 密钥 presence（服务端打码，只回 `has_api_key`）；见 xfyun 同名说明。 */
    var llmApiKeyConfigured by scope.observable(false)
    var llmModel by scope.observable("")
    var llmSystemPrompt by scope.observable("")
    var llmMaxHistory by scope.observable("10")
    var llmTemperature by scope.observable("0.7")
    var llmStream by scope.observable("true")

    // ===== [aiui] =====（全链路极速超拟人：VAD 切段 → 讯飞 AIUI 云端 ASR+LLM+TTS 闭环）
    // ⚠️ 启用后设备流水线走云端闭环，**本地 ASR/LLM/TTS 全部闲置**（`[soul]`/`[memory]` 亦不生效）。
    var aiuiEnabled by scope.observable("false")
    var aiuiAppid by scope.observable("")
    var aiuiApiKey by scope.observable("")
    var aiuiApiSecret by scope.observable("")
    /** 密钥 presence（服务端打码，只回 `has_api_key` / `has_api_secret`）。 */
    var aiuiApiKeyConfigured by scope.observable(false)
    var aiuiApiSecretConfigured by scope.observable(false)
    var aiuiScene by scope.observable("main_box")
    var aiuiSnPrefix by scope.observable("xiaozhi")
    var aiuiVoice by scope.observable("x6_dongmanshaonv_pro")
    var aiuiSpeed by scope.observable("50")
    var aiuiVolume by scope.observable("50")
    var aiuiPitch by scope.observable("50")
    var aiuiPrompt by scope.observable("")
    var aiuiPaceMs by scope.observable("10")

    // ===== [soul] / [memory]：上下文生产者（拆到 ConfigContextState，见 AGENTS.md §5.10）=====
    /// 人格档案与图记忆的表单状态 + 记忆动作（状态查询/测试召回/维护/清空）。
    val sm = ContextSectionState(scope)

    /// 字段元数据（`GET /api/config/schema`）：目前用于「这个字段多久生效」提示（见 FormSchemaState）。
    val schema = FormSchemaState(scope)

    // ===== 状态 =====
    /// 服务端配置内容指纹（GET /api/config 的 `revision`）：保存时回传，用于拒绝**过期写入**。
    /// 两个标签页/两个端同时打开表单时，后保存者会拿到 409 而不是静默覆盖前者的改动。
    var revision by scope.observable("")
    var dirty by scope.observable(false)
    var saving by scope.observable(false)
    var statusMsg by scope.observable("")
    var statusLevel by scope.observable("info")
    var lastSavedAt by scope.observable("")
    /// 上次保存因**过期写入（409）**被拒：顶栏显示「重新加载」入口（草稿保留）。
    var conflict by scope.observable(false)

    /** 配置页标签页 UI 状态（Server/Audio/ASR/VAD/TTS/LLM/AIUI/灵魂/记忆 九个 tab，见 renderForm）。 */
    val tabUi = TabUiState(scope)

    /** 当前展开的下拉（TTS engine 选择等，"" = 全部收起）；放状态类避免 Pager body 重建丢失展开态。 */
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
        ttsLocalEngines.addAll(listOf("kokoro" to "本地 Kokoro INT8（离线）"))
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
    internal fun deriveVoiceGroupAndType() {
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
     * 测试当前表单里的讯飞三要素（POST /api/tts/voices/test-credentials，**只测不存**）：
     * 服务端用传入凭据真实发起一次短合成，验证密钥/服务/发音人是否可用。
     * 结果以提示显示在按钮旁（成功含耗时；失败含可行动原因，如 11200 发音人未授权）。
     */
    fun testXfyunCreds(ctx: Pager) {
        if (!xfyunCredsReady()) {
            credsTestOk = false
            credsTestMsg = "请先填写 app_id / api_key / api_secret"
            return
        }
        credsTestMsg = "测试中…"
        credsTestOk = false
        val body = JSONObject().apply {
            put("app_id", xfyunAppId.trim())
            put("api_key", xfyunApiKey.trim())
            put("api_secret", xfyunApiSecret.trim())
            put("voice", xfyunVoice.trim())
        }
        val headers = JSONObject().apply { put("Content-Type", "application/json") }
        network(ctx).httpRequest("${serverBase}/api/tts/voices/test-credentials", true, body, headers) { data, success, errorMsg, resp ->
            val code = resp.statusCode
            if (success && (code == null || code in 200..299)) {
                val ok = data?.optBoolean("ok", false) ?: false
                credsTestOk = ok
                credsTestMsg = if (ok) {
                    "凭据可用（合成耗时 ${data?.optLong("elapsed_ms") ?: 0}ms）"
                } else {
                    "凭据不可用: ${data?.optString("error", "未知错误")}"
                }
            } else {
                credsTestOk = false
                credsTestMsg = "测试失败: ${if (errorMsg.isNotEmpty()) errorMsg else "HTTP $code"}"
            }
        }
    }

    /**
     * 触发服务端**真实探测**发音人目录（POST /api/tts/voices/refresh，异步后台任务：
     * 对候选池逐项调用合成 API 探测可用性），随后轮询 `GET /api/tts/voices` 直到
     * `probing=false` 再刷新下拉。需先保存三要素（音色列表由真实 API 探测生成）。
     */
    fun refreshVoices(ctx: Pager) {
        voicesRefreshMsg = "探测中…（对候选音色逐个真实合成验证，约需 1 分钟）"
        val headers = JSONObject().apply { put("Content-Type", "application/json") }
        network(ctx).httpRequest("${serverBase}/api/tts/voices/refresh", true, JSONObject(), headers) { _, success, errorMsg, resp ->
            val code = resp.statusCode
            if (success && (code == null || code in 200..299)) {
                pollVoices(ctx, attempts = 0)
            } else {
                voicesRefreshMsg = "探测启动失败: ${if (errorMsg.isNotEmpty()) errorMsg else "HTTP $code"}"
            }
        }
    }

    /** 轮询探测进度（后台任务完成后自动刷新下拉；最多约 3 分钟）。 */
    private fun pollVoices(ctx: Pager, attempts: Int) {
        if (attempts > 90) {
            voicesRefreshMsg = "探测超时（请查看服务端日志）"
            return
        }
        network(ctx).requestGet("${serverBase}/api/tts/voices", JSONObject()) { data, success, _, _ ->
            if (success && data?.optBoolean("probing", false) == false) {
                loadVoices(ctx, serverBase)
            } else {
                ctx.setTimeout(2000) { pollVoices(ctx, attempts + 1) }
            }
        }
    }

    /** 拉取配置来源元信息（GET /api/config/meta）→ 展示"配置文件路径 + 是否持久化"。 */
    fun loadConfigMeta(ctx: Pager, baseUrl: String = "") {
        network(ctx).requestGet("${baseUrl}/api/config/meta", JSONObject()) { data, success, _, _ ->
            if (!success) return@requestGet
            val path = data?.optString("config_path", "") ?: ""
            val persistent = data?.optBoolean("persistent", true) ?: true
            val hint = data?.optString("hint", "") ?: ""
            configMetaMsg = when {
                path.isEmpty() -> "配置来源：内置默认（未指定配置文件，保存不会持久化）"
                !persistent -> "配置来源：$path ⚠️ 不在挂载卷上，重建容器会丢失！$hint"
                else -> "配置来源：$path（持久化 ✓）"
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
            // 空目录（尚未探测过）：给出可行动指引（非错误，避免误以为服务端坏了）
            if (arr.length() == 0) {
                voicesRefreshMsg = "音色列表为空：保存三要素后点「探测音色目录」（服务端会对候选音色逐个真实合成验证）"
            }
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
                loadConfigMeta(ctx, baseUrl)
                // 字段热生效元数据（用于"这个字段多久生效"提示；失败静默）
                schema.load(ctx, baseUrl)
                conflict = false
                statusMsg = "已加载配置"
                statusLevel = "ok"
            } else {
                statusMsg = "加载失败: $errorMsg"
                statusLevel = "error"
            }
        }
    }

    /// 「重新加载」：409 冲突后按服务端现值重填表单（放弃本地草稿）。
    fun reload(ctx: Pager, baseUrl: String = "") {
        dirty = false
        conflict = false
        load(ctx, baseUrl)
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
                put("downlink_lead_ms", downlinkLeadMs.toIntOrNull() ?: 240)
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
                    // 密钥：留空 = 「不修改盘上已存值」（服务端对空串按未修改处理；这里直接不发）
                    putSecret("api_key", xfyunApiKey)
                    putSecret("api_secret", xfyunApiSecret)
                    put("voice", xfyunVoice)
                })
            })
            put("llm", JSONObject().apply {
                put("api_base", llmApiBase)
                // 打码后表单里没有明文：留空 = 不修改（不发送该字段）
                putSecret("api_key", llmApiKey)
                put("model", llmModel)
                put("system_prompt", llmSystemPrompt)
                put("max_history", llmMaxHistory.toIntOrNull() ?: 10)
                put("temperature", llmTemperature.toDoubleOrNull() ?: 0.7)
                // ⚠️ stream 必须回传：serde 端有 default，漏传会被重置为 true，
                // 手工在 TOML 里设的 stream=false 经 UI 保存一次就会丢。
                put("stream", llmStream.toBooleanStrictOrNull() ?: true)
            })
            // [soul] / [memory]：上下文生产者（P6）。与其它段一样**必须整段回传**：
            // 后端虽是部分更新语义，但前端不回传就等于"这个段在 UI 里不可配置"。
            sm.putJson(this)
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
        // ⚠️ 必须显式带 Content-Type: application/json：
        // Kuikly 的 requestPost 默认 headers=null，原生 KRHttpRequestTool 在非 JSON
        // Content-Type 下会把 body 编码成 form-urlencoded —— 服务端 Json<Config> 提取器
        // 直接 415（实测报 "Expected request with `Content-Type: application/json`"，
        // macApp 表现为「保存配置」失败）。同时按 HTTP 状态码判定成功：
        // 传输层 success 且 statusCode 非 2xx（如 415/400）时给出明确错误。
        val headers = JSONObject().apply { put("Content-Type", "application/json") }
        network(ctx).httpRequest("${baseUrl}/api/config", true, body, headers) { data, success, errorMsg, resp ->
            saving = false
            val code = resp.statusCode
            // 服务端返回 JSON {ok, message, config_path}：优先透传 message（含**实际写入路径**
            // 与持久化告警）——"配置怎么没生效/丢哪了"直接可见。
            val serverMsg = data?.optString("message", "") ?: ""
            if (success && (code == null || code in 200..299)) {
                dirty = false
                conflict = false
                lastSavedAt = "已保存"
                // 服务端回传新指纹：无需再 GET 即可继续保存
                revision = data?.optString("revision", revision) ?: revision
                // 三档生效文案由**服务端**判定（字段级 hot 取最严格，见 docs §5.4-6）：
                // 客户端只做翻译，避免"说立即生效、其实要重启"这类误导。
                val hotText = when (data?.optString("hot", "") ?: "") {
                    "live" -> "已保存并立即生效"
                    "next_session" -> "已保存，新会话生效"
                    "restart_only" -> "已保存，下次启动生效"
                    else -> ""
                }
                statusMsg = if (hotText.isEmpty()) {
                    serverMsg.ifEmpty { "保存成功（新会话生效）" }
                } else {
                    "$hotText。$serverMsg"
                }
                statusLevel = "ok"
                // 三要素已填齐时：保存即自动触发音色探测（目录为空或想更新时最省事；失败不影响保存）
                if (xfyunCredsReady()) {
                    refreshVoices(ctx)
                }
            } else if (code == 409) {
                // 过期写入：**保留草稿**（dirty 不动），顶栏出现「重新加载」入口
                dirty = true
                conflict = true
                statusMsg = serverMsg.ifEmpty { "配置已被其他窗口修改，请重新加载后再保存" }
                statusLevel = "error"
            } else {
                statusMsg = "保存失败: ${serverMsg.ifEmpty { if (errorMsg.isNotEmpty()) errorMsg else "HTTP $code" }}"
                statusLevel = "error"
            }
        }
    }

    // ============================================================
    // 渲染：见文件底部 ViewContainer.renderForm(form) 扩展
    // ============================================================
}
