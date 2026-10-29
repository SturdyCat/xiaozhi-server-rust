package com.xiaozhi.admin

import com.tencent.kuikly.core.base.ViewRef
import com.tencent.kuikly.core.pager.Pager
import com.xiaozhi.admin.state.stateVar
import com.xiaozhi.admin.state.stateListOf
import com.tencent.kuikly.core.timer.setTimeout
import com.tencent.kuikly.core.views.DivView
import com.tencent.kuikly.core.views.SelectionType

/**
 * ASR / TTS 测试台状态（macOS 专用，macosArm64Main）。
 *
 * 录音流程（录音与识别解耦）：开始录音 → 停止录音（本地缓冲，可反复试听）→ 发送识别。
 * TTS 流程：合成并播放（实时下行边收边播）→ 完成后出现波形播放器可反复回听。
 *
 * ⚠️ XiaoZhiModule 由持有它的 admin Pager（AdminPage，mac actual 经 platformExternalModules 注入）注册；
 *   本类不是 Pager，故方法都接收 ctx: Pager 来 acquireModule。
 *   渲染见 TestBenchPage.kt（T12 起 Compose 版；旧 ViewContainer.renderBench 已删）。
 *   （T13：随 ui/ 删除，TabUiState/scope 参数退役——tab 选中态归 TestBenchPage 的快照缓存。）
 */
class TestBenchState {

    /** 由统一服务器基址推导的 WS 端点；「切换服务器」时随 API 基址一起更新（见 TestbenchSection）。 */
    var serverUrl by stateVar("ws://127.0.0.1:8000/api/ws")
    var token by stateVar("")
    var connected by stateVar(false)
    var connectionState by stateVar("idle")

    /** ASR 阶段：idle(未录) / starting(启动中) / recording(录音中) / recorded(已录待识别) / recognizing(识别中) */
    var asrPhase by stateVar("idle")
    var asrText by stateVar("")

    /** 录音计秒（秒，小数）：录音中由原生样本计数 200ms 轮询刷新 */
    var recSeconds by stateVar(0.0)

    /** 上次识别的耗时（ms）：sendAsr 发出 → 收到 stt 的服务端往返 */
    var asrElapsedMs by stateVar(0)

    // 录音试听播放器（原生 recordingPcm）
    var recWave by stateVar(emptyList<Float>())
    var recProgress by stateVar(0f)
    var recPlaying by stateVar(false)
    var recDuration by stateVar(0.0)

    // TTS 回放播放器（原生 ttsPcm）
    var ttsReady by stateVar(false)
    var ttsWave by stateVar(emptyList<Float>())
    var ttsProgress by stateVar(0f)
    var ttsPlaying by stateVar(false)
    var ttsDuration by stateVar(0.0)

    /** 合成中（发 tts_test → 收到 tts stop）：按钮菊花 + 禁止再次合成 */
    var ttsBusy by stateVar(false)
    var speaking by stateVar(false)

    var ttsText by stateVar("你好，小智")

    /** 最近一次合成立即失败的原因（服务端 tts_test 结果帧；空=无错误）。 */
    var ttsError by stateVar("")

    /** 最近一次合成使用的引擎名（服务端回报：sherpa / xfyun）。 */
    var ttsEngine by stateVar("")

    /** 合成语言：auto 自动（Kokoro 需明确 lang，auto 走服务器默认）/ zh / en */
    var ttsLang by stateVar("auto")
    var ttsSpeed by stateVar("1.0")

    /** 语音角色 sid（Kokoro voices.bin 的索引）。UI 用「性别 + 音色」两级选择派生本值。 */
    var ttsSpeaker by stateVar(3)

    /** 音色性别筛选：female(中文女声) / male(中文男声) / en(英文女声) */
    var voiceGender by stateVar("female")

    /** 已选音色（见 VoiceCatalog；与 ttsSpeaker 同步） */
    var voiceId by stateVar("zf_001")

    /** 当前展开的下拉（"" = 全部收起）；放状态类里避免 Pager body 重建时丢失展开态 */
    var openDropdown by stateVar("")

    // ===== 下拉选项（Compose 快照列表：音色列表随性别切换增删；GearUI Select 直读）=====
    val langOptions: MutableList<Pair<String, String>> = stateListOf()
    val genderOptions: MutableList<Pair<String, String>> = stateListOf()
    val voiceOptions: MutableList<Pair<String, String>> = stateListOf()

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

    var statusMsg by stateVar("")

    // ============================================================
    // 文本选中 / 复制（ASR 结果、LLM 回复；官方 SelectionContainer 能力）
    // ============================================================

    /** 可选中的结果容器（渲染时经 ref 回填；长按 createSelection / 复制取选区都用它）。 */
    var asrResultRef: ViewRef<DivView>? = null
    var llmReplyRef: ViewRef<DivView>? = null

    /** 「复制」按钮的卡片内反馈（各卡片独立，避免串台）。 */
    var asrCopyHint by stateVar("")
    var llmCopyHint by stateVar("")

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

    /** onDone：连接结果回调（成功与否），供统一壳接续 UI 切换。 */
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
        // 合成+下发超时兜底：60s 未收到 tts stop 则恢复按钮（服务端 tts_test 有 30s
        // 合成链预算，正常错误会以结果帧先回；走到这里说明服务器整链未回包/连接异常）。
        // 只解锁按钮与提示，不锁死——允许直接再次发起合成。
        ctx.setTimeout(60_000) {
            if (ttsBusy) {
                ttsBusy = false
                speaking = false
                ttsError = "未收到服务器结果，请查看服务器日志"
                statusMsg = "合成超时：未收到服务器结果，请查看服务器日志（可重试）"
            }
        }
    }

    // ============================================================
    // LLM 对话测试：发 llm_test → 服务端按最新配置直调 LLM → llm_test 回调
    // ============================================================

    var llmPrompt by stateVar("你好，请用一句话介绍你自己")

    /** 回复正文（state=error 时是服务端返回的可读错误信息） */
    var llmReply by stateVar("")
    var llmBusy by stateVar(false)
    var llmOk by stateVar(false)

    /** 服务端直调 LLM 的总耗时（含到 LLM API 的网络往返），由 llm_test 回包携带 */
    var llmElapsedMs by stateVar(0)

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
    // NLU 意图测试：发 nlu_test → 服务端按最新配置构建 TinyBERT 直推 → nlu_test 回调
    // ============================================================

    var nluInput by stateVar("把客厅的灯打开")

    /** 识别到的意图名（低于阈值/出错时为空，原因看 nluText）。 */
    var nluIntent by stateVar("")

    /** 最高置信度 0~1；<0 表示本轮回包没带（出错帧）。 */
    var nluConfidence by stateVar(-1f)
    var nluBusy by stateVar(false)
    var nluOk by stateVar(false)

    /** 说明文本：命中时是映射后的技能短语，未命中/出错时是服务端给出的可读原因。 */
    var nluText by stateVar("")

    /** 分词 / 推理耗时（ms），由 nlu_test 回包携带。 */
    var nluTokenizeMs by stateVar(0)
    var nluInferMs by stateVar(0)

    var nluReplyRef: ViewRef<DivView>? = null
    var nluCopyHint by stateVar("")

    /** 复制 NLU 测试详情（有选区复制选区，无选区复制整段）。 */
    fun copyNlu(ctx: Pager) = copySelectionOr(ctx, nluReplyRef, nluResultText()) { nluCopyHint = it }

    /** 详情面板整段文本（复制回退用）。 */
    fun nluResultText(): String {
        if (!nluOk) return nluText
        val head = buildString {
            append("意图：").append(nluIntent.ifBlank { "（未命中）" })
            if (nluConfidence >= 0f) append("　置信度：").append((nluConfidence * 1000).toInt() / 1000.0)
            append("\n耗时：分词 ").append(nluTokenizeMs).append("ms / 推理 ").append(nluInferMs).append("ms")
        }
        return if (nluText.isBlank()) head else "$head\n$nluText"
    }

    fun testNlu(ctx: Pager) {
        if (nluBusy) return
        if (!connected) {
            statusMsg = "请先在「连接」卡片建立连接"
            return
        }
        if (nluInput.isBlank()) {
            statusMsg = "请输入测试文本"
            return
        }
        nluBusy = true
        nluIntent = ""
        nluConfidence = -1f
        nluText = ""
        nluTokenizeMs = 0
        nluInferMs = 0
        statusMsg = "NLU 测试中…"
        xz(ctx).nluTest(nluInput) { result ->
            nluBusy = false
            nluOk = result?.optString("state", "error") == "ok"
            if (nluOk) {
                nluIntent = result?.optString("intent", "") ?: ""
                nluConfidence = if (result?.has("confidence") == true) {
                    (result?.optDouble("confidence") ?: Double.NaN).toFloat()
                } else -1f
                nluTokenizeMs = result?.optInt("tokenizeMs", 0) ?: 0
                nluInferMs = result?.optInt("inferMs", 0) ?: 0
                nluText = result?.optString("text", "") ?: ""
                statusMsg = if (nluIntent.isBlank()) {
                    "未命中意图：$nluText"
                } else {
                    "命中意图 $nluIntent（分词 ${nluTokenizeMs}ms / 推理 ${nluInferMs}ms）"
                }
            } else {
                nluText = result?.optString("text", "")?.ifBlank { "（服务端未返回原因）" } ?: ""
                statusMsg = "NLU 测试失败：$nluText"
            }
        }
        // 超时兜底：本地 TinyBERT 推理通常毫秒级，10s 未回基本是连接/服务端异常
        ctx.setTimeout(10_000) {
            if (nluBusy) {
                nluBusy = false
                statusMsg = "NLU 测试超时：请检查连接或服务器日志"
            }
        }
    }

    // ============================================================
    // 播放器控制（录音 / TTS 试听；T12 起播放控制下沉 XiaoZhiModule）
    //
    // 旧实现挂两个 1x1 原生 AudioView（src + playControl 命令 + playState/playTime
    // 事件回推）；DSL 平台视图挂不进 Compose 树，且回归模块层本就有的视图无关播放
    // API——AudioView 正是"替代原先 100ms 轮询原生播放器"引入的（见下），这里改回：
    // UI 只发命令（playRecording/playTts/stopPlayback）+ 100ms 轮询 getAudioState
    // （playing/position/duration）驱动进度与三态按钮。
    // ============================================================

    /** 开始试听：原生从头播放对应缓冲（录音 → playRecording、TTS → playTts），并启动进度轮询。 */
    fun togglePlayback(ctx: Pager, source: String) {
        val playing = if (source == "tts") ttsPlaying else recPlaying
        if (playing) {
            xz(ctx).stopPlayback()
            setPlayback(source, playing = false, progress = 0f)
            return
        }
        // 模块层单播放器语义，起始一路前停另一路（旧双 AudioView 的"独立停止"在模块层不存在）：
        // rec/tts 任一路开始时另一路立即复位，避免残留的 playing 态与真实播放器错乱。
        stopOtherPlayback(ctx, source)
        setPlayback(source, playing = true, progress = 0f)
        if (source == "tts") {
            xz(ctx).playTts { st ->
                val d = st?.optDouble("duration") ?: 0.0
                if (d > 0) ttsDuration = d
            }
        } else {
            xz(ctx).playRecording { st ->
                val d = st?.optDouble("duration") ?: 0.0
                if (d > 0) recDuration = d
            }
        }
        pollPlayback(ctx, source)
    }

    /** 播放进度轮询（100ms）：playing=true 推进进度并续轮询；false 视为播完/已停，复位并停止。 */
    private fun pollPlayback(ctx: Pager, source: String) {
        xz(ctx).getAudioState(source) { st ->
            val stillPlaying = st?.optBoolean("playing", false) ?: false
            val duration = st?.optDouble("duration", 0.0) ?: 0.0
            val position = st?.optDouble("position", 0.0) ?: 0.0
            if (duration > 0) {
                if (source == "tts") ttsDuration = duration else recDuration = duration
            }
            if (stillPlaying) {
                val p = if (duration > 0) (position / duration).toFloat().coerceIn(0f, 1f) else 0f
                setPlayback(source, playing = true, progress = p)
                ctx.setTimeout(PLAY_POLL_MS) { pollPlayback(ctx, source) }
            } else {
                // duration<=0 说明原生侧根本没有可播的缓冲（AudioView 时代的空导出兜底同因）
                if (duration <= 0) statusMsg = "试听失败：音频缓冲为空或原生播放器不可用"
                setPlayback(source, playing = false, progress = 0f)
            }
        }
    }

    /** 按来源写回播放态（rec/tts 两套同构字段，与旧 AudioView 事件回写同口径）。 */
    private fun setPlayback(source: String, playing: Boolean, progress: Float) {
        if (source == "tts") {
            ttsPlaying = playing
            ttsProgress = progress
        } else {
            recPlaying = playing
            recProgress = progress
        }
    }

    /** 模块层单播放器：起始 [source] 前停掉另一路（仅在它确实在播时才发原生 STOP）。 */
    private fun stopOtherPlayback(ctx: Pager, source: String) {
        val otherPlaying = if (source == "tts") recPlaying else ttsPlaying
        if (otherPlaying) {
            xz(ctx).stopPlayback()
            setPlayback(if (source == "tts") "recording" else "tts", playing = false, progress = 0f)
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
        /** 播放进度轮询间隔（ms）：原生 getAudioState 本就按该节奏设计（playing/position/duration）。 */
        private const val PLAY_POLL_MS = 100

        /** auto → 交给服务器默认；zh/en 显式指定 Kokoro lang（Kokoro 的 lang 在建模时固定，
         *  测试台自动模式下由服务端回退配置默认值）。 */
        fun resolveLang(lang: String): String = if (lang == "auto") "" else lang
    }
}
