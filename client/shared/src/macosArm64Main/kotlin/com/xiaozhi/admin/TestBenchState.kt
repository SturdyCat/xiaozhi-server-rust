package com.xiaozhi.admin

import com.tencent.kuikly.core.base.PagerScope
import com.tencent.kuikly.core.base.ViewContainer
import com.tencent.kuikly.core.base.ViewRef
import com.tencent.kuikly.core.directives.velse
import com.tencent.kuikly.core.directives.vif
import com.tencent.kuikly.core.pager.Pager
import com.tencent.kuikly.core.reactive.collection.ObservableList
import com.tencent.kuikly.core.reactive.handler.observable
import com.tencent.kuikly.core.reactive.handler.observableList
import com.tencent.kuikly.core.timer.setTimeout
import com.tencent.kuikly.core.views.DivView
import com.tencent.kuikly.core.views.SelectableOption
import com.tencent.kuikly.core.views.SelectionType
import com.tencent.kuikly.core.views.Text
import com.tencent.kuikly.core.views.View

/**
 * ASR / TTS 测试台状态（macOS 专用，macosArm64Main）。
 *
 * 录音流程（录音与识别解耦）：开始录音 → 停止录音（本地缓冲，可反复试听）→ 发送识别。
 * TTS 流程：合成并播放（实时下行边收边播）→ 完成后出现波形播放器可反复回听。
 *
 * ⚠️ XiaoZhiModule 由持有它的 Pager（AdminShell）经 createExternalModules() 注册；
 *   本类不是 Pager，故方法都接收 ctx: Pager 来 acquireModule。
 *   渲染见文件底部的 ViewContainer.renderBench(bench, ctx) 扩展。
 */
class TestBenchState(private val scope: PagerScope) {

    var serverUrl by scope.observable("ws://127.0.0.1:8000/api/ws")
    var token by scope.observable("")
    var connected by scope.observable(false)
    var connectionState by scope.observable("idle") // idle / connecting / connected / error

    /** ASR 阶段：idle(未录) / starting(启动中) / recording(录音中) / recorded(已录待识别) / recognizing(识别中) */
    var asrPhase by scope.observable("idle")
    var asrText by scope.observable("")

    /** 录音计秒（秒，小数）：录音中由原生样本计数 200ms 轮询刷新 */
    var recSeconds by scope.observable(0.0)

    /** 上次识别的耗时（ms）：sendAsr 发出 → 收到 stt 的服务端往返 */
    var asrElapsedMs by scope.observable(0)

    // 录音试听播放器（原生 recordingPcm）
    var recWave by scope.observable(emptyList<Float>())
    var recProgress by scope.observable(0f)
    var recPlaying by scope.observable(false)
    var recDuration by scope.observable(0.0)

    // TTS 回放播放器（原生 ttsPcm）
    var ttsReady by scope.observable(false)
    var ttsWave by scope.observable(emptyList<Float>())
    var ttsProgress by scope.observable(0f)
    var ttsPlaying by scope.observable(false)
    var ttsDuration by scope.observable(0.0)

    /** 合成中（发 tts_test → 收到 tts stop）：按钮菊花 + 禁止再次合成 */
    var ttsBusy by scope.observable(false)
    var speaking by scope.observable(false)

    var ttsText by scope.observable("你好，小智")

    /** 最近一次合成立即失败的原因（服务端 tts_test 结果帧；空=无错误）。 */
    var ttsError by scope.observable("")

    /** 最近一次合成使用的引擎名（服务端回报：sherpa / xfyun）。 */
    var ttsEngine by scope.observable("")

    /** 合成语言：auto 自动（Kokoro 需明确 lang，auto 走服务器默认）/ zh / en */
    var ttsLang by scope.observable("auto")
    var ttsSpeed by scope.observable("1.0") // 参考 kokoro.js demo：语速

    /** 语音角色 sid（Kokoro voices.bin 的索引）。UI 用「性别 + 音色」两级选择派生本值。 */
    var ttsSpeaker by scope.observable(3) // 默认 zf_001（中文女声，sid=3；见 VoiceCatalog 注释）

    /** 音色性别筛选：female(中文女声) / male(中文男声) / en(英文女声) */
    var voiceGender by scope.observable("female")

    /** 已选音色（见 VoiceCatalog；与 ttsSpeaker 同步） */
    var voiceId by scope.observable("zf_001")

    /** 当前展开的下拉（"" = 全部收起）；放状态类里避免 Pager body 重建时丢失展开态 */
    var openDropdown by scope.observable("")

    // ===== 下拉选项（ObservableList：音色列表随性别切换增删，vfor 响应式渲染）=====
    val langOptions: ObservableList<Pair<String, String>> by scope.observableList()
    val genderOptions: ObservableList<Pair<String, String>> by scope.observableList()
    val voiceOptions: ObservableList<Pair<String, String>> by scope.observableList()

    init {
        langOptions.addAll(LANG_OPTIONS)
        genderOptions.addAll(VoiceCatalog.GENDERS)
        reloadVoiceOptions()
    }

    /** 按当前性别重建音色选项列表（sid 一并写入显示文案）。 */
    private fun reloadVoiceOptions() {
        voiceOptions.clear()
        voiceOptions.addAll(VoiceCatalog.options(voiceGender).map { it.id to "${it.id}（sid ${it.sid}）" })
    }

    var statusMsg by scope.observable("")

    /** 测试台标签页 UI 状态（连接 / ASR 识别 / TTS 合成 / LLM 对话 四个 tab，见 renderBench）。 */
    val tabUi = TabUiState(scope)

    // ============================================================
    // 文本选中 / 复制（ASR 结果、LLM 回复；官方 SelectionContainer 能力）
    // ============================================================

    /** 可选中的结果容器（渲染时经 ref 回填；长按 createSelection / 复制取选区都用它）。 */
    var asrResultRef: ViewRef<DivView>? = null
    var llmReplyRef: ViewRef<DivView>? = null

    /** 「复制」按钮的卡片内反馈（各卡片独立，避免串台）。 */
    var asrCopyHint by scope.observable("")
    var llmCopyHint by scope.observable("")

    /** 复制 ASR 识别结果（有选区复制选区，无选区复制整段）。 */
    fun copyAsr(ctx: Pager) = copySelectionOr(ctx, asrResultRef, asrText) { asrCopyHint = it }

    /** 复制 LLM 回复（有选区复制选区，无选区复制整段）。 */
    fun copyLlm(ctx: Pager) = copySelectionOr(ctx, llmReplyRef, llmReply) { llmCopyHint = it }

    /** 长按进入选中态（官方约定：渲染层只画选区，创建选区由业务手势触发）。 */
    fun startSelection(ref: ViewRef<DivView>?, x: Float, y: Float) {
        ref?.view?.createSelection(x, y, SelectionType.WORD)
    }

    /** 取选区文本（异步回调）→ 剪贴板；选区为空回退整段 fallback。 */
    private fun copySelectionOr(
        ctx: Pager,
        ref: ViewRef<DivView>?,
        fallback: String,
        setHint: (String) -> Unit,
    ) {
        val view = ref?.view
        if (view == null) {
            finishCopy(ctx, fallback, setHint)
            return
        }
        view.getSelection { result ->
            // 选区为若干文本段（按阅读顺序），直接拼接；未长按选择时为空 → 整段复制
            val sel = result.joinToString("")
            finishCopy(ctx, sel.ifBlank { fallback }, setHint)
        }
    }

    private fun finishCopy(ctx: Pager, text: String, setHint: (String) -> Unit) {
        val t = text.trim()
        if (t.isEmpty()) {
            setHint("没有可复制的内容")
            return
        }
        xz(ctx).copyText(t)
        setHint("已复制 ${t.length} 字")
    }

    // ============================================================
    // 桥接：经 Pager 取 XiaoZhiModule
    // ============================================================

    private fun xz(ctx: Pager): XiaoZhiModule = ctx.acquireModule(XiaoZhiModule.MODULE_NAME)

    /** onDone：连接结果回调（成功与否），供启动自动连接流程（ConnectState）接续 UI 切换。 */
    fun connect(ctx: Pager, onDone: ((Boolean) -> Unit)? = null) {
        connectionState = "connecting"
        statusMsg = "连接中…"
        xz(ctx).connect(serverUrl, token) { result ->
            val ok = result?.optBoolean("success", false) ?: false
            connected = ok
            connectionState = if (ok) "connected" else "error"
            statusMsg = if (ok) "已连接" else "连接失败: ${result?.optString("error", "") ?: ""}"
            onDone?.invoke(ok)
        }
    }

    fun disconnect(ctx: Pager) {
        xz(ctx).disconnect()
        connected = false
        connectionState = "idle"
        statusMsg = "已断开"
    }

    // ============================================================
    // ASR：录音 → 试听 → 发送识别
    // ============================================================

    /** 开始录音：asrPhase=starting 菊花期间挡住重复点击；原生同时累积本地缓冲。
     *  录音实际开始的信号来自原生 started 回调（非猜测延时）——拿到即进入 recording 并开始计秒。 */
    fun startAsr(ctx: Pager) {
        if (asrPhase == "starting" || asrPhase == "recording" || asrPhase == "recognizing") return
        asrPhase = "starting"
        asrText = ""
        asrElapsedMs = 0
        recSeconds = 0.0
        recWave = emptyList()
        recProgress = 0f
        recPlaying = false
        statusMsg = "正在启动录音…"
        xz(ctx).startAsr { result ->
            if (result == null) return@startAsr
            if (result.optBoolean("started", false)) {
                // 原生确认：tap 已装、引擎已起 —— 真正开始录音
                asrPhase = "recording"
                statusMsg = "录音中…说完点「停止录音」"
                startRecordTick(ctx)
                return@startAsr
            }
            val err = result.optString("error", "")
            if (err.isNotEmpty()) {
                // 启动失败（权限被拒/设备异常）
                asrPhase = "idle"
                statusMsg = "录音启动失败：$err"
                return@startAsr
            }
            // 识别结果（sendAsr 之后服务端回 stt）
            asrElapsedMs = result.optInt("elapsedMs", 0)
            asrPhase = if (asrPhase == "recognizing") "recorded" else "idle"
            asrText = result.optString("text", "")
            statusMsg = if (asrElapsedMs > 0) "识别完成（耗时 ${asrElapsedMs}ms）" else "识别完成"
        }
    }

    /** 录音计秒轮询（200ms）：读原生已采集样本数刷新 recSeconds；停止/退出时自然结束。 */
    private fun startRecordTick(ctx: Pager) {
        if (asrPhase != "recording") return
        xz(ctx).getAudioState("recording") { st ->
            if (st != null && st.optBoolean("recording", false)) {
                recSeconds = st.optDouble("recordingSeconds", recSeconds)
                ctx.setTimeout(200) { startRecordTick(ctx) }
            }
        }
    }

    /** 停止录音（不发识别）：原生保留缓冲，进入试听态。 */
    fun stopRecording(ctx: Pager) {
        if (asrPhase != "recording") return
        asrPhase = "recorded"
        statusMsg = "已停止（录了 ${formatSeconds(recSeconds)}），可试听后点「发送识别」"
        xz(ctx).stopRecording { result ->
            recDuration = result?.optDouble("seconds") ?: recSeconds
            refreshWave(ctx, "recording")
        }
    }

    /** 发送识别（asr_test stop）：服务端识别整段并经 stt 回调返回；stt 回调携带往返耗时。 */
    fun sendAsr(ctx: Pager) {
        if (asrPhase != "recorded") return
        asrPhase = "recognizing"
        statusMsg = "识别中…"
        xz(ctx).sendAsr()
        // 超时兜底：SenseVoice 整段识别一般数秒；30s 未回则恢复（按钮不卡死）
        ctx.setTimeout(30_000) {
            if (asrPhase == "recognizing") {
                asrPhase = "recorded"
                statusMsg = "识别超时：请检查服务器日志或重试"
            }
        }
    }

    // ============================================================
    // TTS：合成（实时播放）→ 完成后波形回放
    // ============================================================

    fun speak(ctx: Pager, vcn: String) {
        if (ttsBusy) return
        if (ttsText.isBlank()) {
            statusMsg = "请输入要合成的文字"
            return
        }
        ttsBusy = true
        speaking = true
        ttsReady = false
        ttsPlaying = false
        ttsProgress = 0f
        ttsError = ""
        statusMsg = "合成中…"
        val speed = ttsSpeed.toDoubleOrNull() ?: 1.0
        val lang = resolveLang(ttsLang)
        // vcn（远程音色）由调用点从配置取（TTS 卡/测试卡同一 form）：
        // 服务端本次合成即用它，无需先保存配置；本地（Kokoro）模式传空串。
        xz(ctx).speak(ttsText, ttsSpeaker, lang, speed, vcn) { result ->
            ttsEngine = result?.optString("engine", "") ?: ""
            if (result?.optString("state", "stop") == "error") {
                // 服务端合成失败（引擎切换失败/凭据错误/网络等）：错误原因显示在卡片
                ttsBusy = false
                speaking = false
                ttsError = result.optString("text", "合成失败")
                statusMsg = "TTS 失败：$ttsError"
                return@speak
            }
            // tts stop：合成结束（第一遍已实时播完/在播尾帧）
            ttsBusy = false
            speaking = false
            ttsReady = true
            statusMsg = "TTS 播放完成，可回听（${ttsEngine.ifEmpty { "未知引擎" }}）"
            refreshWave(ctx, "tts")
        }
        // 合成+下发超时兜底：60s 未收到 tts stop 则恢复按钮（模型首次加载可能较慢）
        ctx.setTimeout(60_000) {
            if (ttsBusy) {
                ttsBusy = false
                speaking = false
                statusMsg = "合成超时：请检查服务器日志或重试"
            }
        }
    }

    // ============================================================
    // LLM 对话测试：发 llm_test → 服务端按最新配置直调 LLM → llm_test 回调
    // ============================================================

    var llmPrompt by scope.observable("你好，请用一句话介绍你自己")

    /** 回复正文（state=error 时是服务端返回的可读错误信息） */
    var llmReply by scope.observable("")
    var llmBusy by scope.observable(false)
    var llmOk by scope.observable(false)

    /** 服务端直调 LLM 的总耗时（含到 LLM API 的网络往返），由 llm_test 回包携带 */
    var llmElapsedMs by scope.observable(0)

    fun testLlm(ctx: Pager) {
        if (llmBusy) return
        if (!connected) {
            statusMsg = "请先在「连接」卡片建立连接"
            return
        }
        if (llmPrompt.isBlank()) {
            statusMsg = "请输入测试提示词"
            return
        }
        llmBusy = true
        llmReply = ""
        llmOk = false
        llmElapsedMs = 0
        statusMsg = "LLM 测试中…"
        xz(ctx).llmTest(llmPrompt) { result ->
            llmBusy = false
            llmOk = result?.optString("state", "error") == "ok"
            llmElapsedMs = result?.optInt("elapsedMs", 0) ?: 0
            llmReply = result?.optString("text", "")?.ifBlank {
                // 空回复单独提示（state=ok 但正文为空，如模型只回了工具调用）
                if (llmOk) "（LLM 返回了空回复）" else ""
            } ?: ""
            statusMsg = if (llmOk) "LLM 正常（服务端耗时 ${llmElapsedMs}ms）" else "LLM 测试失败：$llmReply"
        }
        // 超时兜底：真实 API 网络+推理可能较慢；60s 未回恢复按钮
        ctx.setTimeout(60_000) {
            if (llmBusy) {
                llmBusy = false
                statusMsg = "LLM 测试超时：请检查 api_base/api_key 或服务器日志"
            }
        }
    }

    // ============================================================
    // 播放器控制（录音 / TTS 共用；100ms 轮询驱动进度）
    // ============================================================

    fun togglePlayback(ctx: Pager, source: String) {
        val playing = if (source == "tts") ttsPlaying else recPlaying
        if (playing) {
            xz(ctx).stopPlayback()
            if (source == "tts") {
                ttsPlaying = false
                ttsProgress = 0f
            } else {
                recPlaying = false
                recProgress = 0f
            }
            return
        }
        // 从头播
        if (source == "tts") {
            ttsPlaying = true
            xz(ctx).playTts(null)
        } else {
            recPlaying = true
            xz(ctx).playRecording(null)
        }
        startPolling(ctx, source)
    }

    /** 轮询原生播放状态（100ms）刷新进度；播完自动停轮询并复位。 */
    private fun startPolling(ctx: Pager, source: String) {
        xz(ctx).getAudioState(source) { st ->
            val playing = st?.optBoolean("playing", false) ?: false
            val position = st?.optDouble("position") ?: 0.0
            val duration = st?.optDouble("duration") ?: 0.0
            val p = if (duration > 0) (position / duration).toFloat().coerceIn(0f, 1f) else 0f
            if (source == "tts") {
                ttsPlaying = playing
                ttsProgress = p
            } else {
                recPlaying = playing
                recProgress = p
            }
            if (playing) {
                ctx.setTimeout(100) { startPolling(ctx, source) }
            } else {
                // 播完/停止：进度复位由 stopPlayback/下一次播放处理
                if (source == "tts") ttsProgress = 0f else recProgress = 0f
            }
        }
    }

    /** 拉一次波形（录音/合成完成后各拉一次；波形静态，播放只推进度）。 */
    private fun refreshWave(ctx: Pager, source: String) {
        xz(ctx).getAudioState(source) { st ->
            val wave = mutableListOf<Float>()
            val arr = st?.optJSONArray("wave")
            if (arr != null) {
                for (i in 0 until arr.length()) wave.add(arr.optDouble(i).toFloat())
            }
            if (source == "tts") {
                ttsWave = wave
                ttsDuration = st?.optDouble("duration") ?: 0.0
            } else {
                recWave = wave
                recDuration = st?.optDouble("duration") ?: recDuration
            }
        }
    }

    /** 语音选择：切换性别时自动落到该性别第一个音色；同步 sid 与下拉选项列表。 */
    fun selectVoice(gender: String, id: String) {
        voiceGender = gender
        voiceId = id
        VoiceCatalog.sidOf(id)?.let { ttsSpeaker = it }
        reloadVoiceOptions()
    }

    companion object {
        /** auto → 交给服务器默认；zh/en 显式指定 Kokoro lang（Kokoro 的 lang 在建模时固定，
         *  测试台自动模式下由服务端回退配置默认值）。 */
        fun resolveLang(lang: String): String = if (lang == "auto") "" else lang
    }
}
