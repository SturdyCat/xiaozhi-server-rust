package com.xiaozhi.admin

import com.tencent.kuikly.core.base.ViewContainer
import com.tencent.kuikly.core.directives.velse
import com.tencent.kuikly.core.directives.vif
import com.tencent.kuikly.core.pager.Pager
import com.tencent.kuikly.core.reactive.collection.ObservableList
import com.tencent.kuikly.core.reactive.handler.observable
import com.tencent.kuikly.core.reactive.handler.observableList
import com.tencent.kuikly.core.timer.setTimeout
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
class TestBenchState {

    var serverUrl by observable("ws://127.0.0.1:8000/api/ws")
    var token by observable("")
    var connected by observable(false)
    var connectionState by observable("idle") // idle / connecting / connected / error

    /** ASR 阶段：idle(未录) / starting(启动中) / recording(录音中) / recorded(已录待识别) / recognizing(识别中) */
    var asrPhase by observable("idle")
    var asrText by observable("")

    // 录音试听播放器（原生 recordingPcm）
    var recWave by observable(emptyList<Float>())
    var recProgress by observable(0f)
    var recPlaying by observable(false)
    var recDuration by observable(0.0)

    // TTS 回放播放器（原生 ttsPcm）
    var ttsReady by observable(false)
    var ttsWave by observable(emptyList<Float>())
    var ttsProgress by observable(0f)
    var ttsPlaying by observable(false)
    var ttsDuration by observable(0.0)

    /** 合成中（发 tts_test → 收到 tts stop）：按钮菊花 + 禁止再次合成 */
    var ttsBusy by observable(false)
    var speaking by observable(false)

    var ttsText by observable("你好，小智")

    /** 合成语言：auto 自动（Kokoro 需明确 lang，auto 走服务器默认）/ zh / en */
    var ttsLang by observable("auto")
    var ttsSpeed by observable("1.0") // 参考 kokoro.js demo：语速

    /** 语音角色 sid（Kokoro voices.bin 的索引）。UI 用「性别 + 音色」两级选择派生本值。 */
    var ttsSpeaker by observable(3) // 默认 zf_001（中文女声，sid=3；见 VoiceCatalog 注释）

    /** 音色性别筛选：female(中文女声) / male(中文男声) / en(英文女声) */
    var voiceGender by observable("female")

    /** 已选音色（见 VoiceCatalog；与 ttsSpeaker 同步） */
    var voiceId by observable("zf_001")

    /** 当前展开的下拉（"" = 全部收起）；放状态类里避免 Pager body 重建时丢失展开态 */
    var openDropdown by observable("")

    // ===== 下拉选项（ObservableList：音色列表随性别切换增删，vfor 响应式渲染）=====
    val langOptions: ObservableList<Pair<String, String>> by observableList()
    val genderOptions: ObservableList<Pair<String, String>> by observableList()
    val voiceOptions: ObservableList<Pair<String, String>> by observableList()

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

    var statusMsg by observable("")

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

    /** 开始录音：asrPhase=starting 菊花期间挡住重复点击；原生同时累积本地缓冲。 */
    fun startAsr(ctx: Pager) {
        if (asrPhase == "starting" || asrPhase == "recording" || asrPhase == "recognizing") return
        asrPhase = "starting"
        asrText = ""
        recWave = emptyList()
        recProgress = 0f
        recPlaying = false
        statusMsg = "正在启动录音…"
        xz(ctx).startAsr { result ->
            // keepCallback：此回调在服务端回 stt（sendAsr 之后）时触发
            asrPhase = if (asrPhase == "recognizing") "recorded" else "idle"
            asrText = result?.optString("text", "") ?: ""
            statusMsg = "识别完成"
        }
        // 权限框弹出后无回调分支：0.8s 后认为进入录音态
        ctx.setTimeout(800) {
            if (asrPhase == "starting") {
                asrPhase = "recording"
                statusMsg = "录音中…说完点「停止录音」"
            }
        }
    }

    /** 停止录音（不发识别）：原生保留缓冲，进入试听态。 */
    fun stopRecording(ctx: Pager) {
        if (asrPhase != "recording") return
        asrPhase = "recorded"
        statusMsg = "已停止，可试听后点「发送识别」"
        xz(ctx).stopRecording { result ->
            recDuration = result?.optDouble("seconds") ?: 0.0
            refreshWave(ctx, "recording")
        }
    }

    /** 发送识别（asr_test stop）：服务端识别整段并经 stt 回调返回。 */
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

    fun speak(ctx: Pager) {
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
        statusMsg = "合成中…"
        val speed = ttsSpeed.toDoubleOrNull() ?: 1.0
        val lang = resolveLang(ttsLang)
        xz(ctx).speak(ttsText, ttsSpeaker, lang, speed) { _ ->
            // tts stop：合成结束（第一遍已实时播完/在播尾帧）
            ttsBusy = false
            speaking = false
            ttsReady = true
            statusMsg = "TTS 播放完成，可回听"
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

/**
 * Kokoro voices.bin 的 sid ↔ 音色名目录（基于 sherpa-onnx kokoro-int8-multi-lang-v1_1
 * 的 voices.bin 生成脚本 `scripts/kokoro/v1.1-zh/generate_voices_bin.py` 的顺序：
 *   0-2: af_maple/af_sol/bf_vale（英文女声）
 *   3-57: zf_xxx（中文女声，55 个；列表来自 sherpa 官方 sid 表）
 *   58-102: zm_xxx（中文男声，45 个）
 * ⚠️ 顺序由 voices.bin 决定，不能自行排序——sid 必须与 voices.bin 索引一致，否则选错音色。
 */
object VoiceCatalog {
    /** 英文女声（sid 0-2） */
    val EN_FEMALE: List<VoiceOption> = listOf(
        VoiceOption("af_maple", 0),
        VoiceOption("af_sol", 1),
        VoiceOption("bf_vale", 2),
    )

    val ZF_NAMES: List<String> = listOf(
        "zf_001", "zf_002", "zf_003", "zf_004", "zf_005", "zf_006",
        "zf_007", "zf_008", "zf_017", "zf_018", "zf_019", "zf_021",
        "zf_022", "zf_023", "zf_024", "zf_026", "zf_027", "zf_028",
        "zf_032", "zf_036", "zf_038", "zf_039", "zf_040", "zf_042",
        "zf_043", "zf_044", "zf_046", "zf_047", "zf_048", "zf_049",
        "zf_051", "zf_059", "zf_060", "zf_067", "zf_070", "zf_071",
        "zf_072", "zf_073", "zf_074", "zf_075", "zf_076", "zf_077",
        "zf_078", "zf_079", "zf_083", "zf_084", "zf_085", "zf_086",
        "zf_087", "zf_088", "zf_090", "zf_092", "zf_093", "zf_094",
        "zf_099",
    )

    val ZM_NAMES: List<String> = listOf(
        "zm_009", "zm_010", "zm_011", "zm_012", "zm_013", "zm_014",
        "zm_015", "zm_016", "zm_020", "zm_025", "zm_029", "zm_030",
        "zm_031", "zm_033", "zm_034", "zm_035", "zm_037", "zm_041",
        "zm_045", "zm_050", "zm_052", "zm_053", "zm_054", "zm_055",
        "zm_056", "zm_057", "zm_058", "zm_061", "zm_062", "zm_063",
        "zm_064", "zm_065", "zm_066", "zm_068", "zm_069", "zm_080",
        "zm_081", "zm_082", "zm_089", "zm_091", "zm_095", "zm_096",
        "zm_097", "zm_098", "zm_100",
    )

    /** 中文女声（sid 3 起连续） */
    val ZH_FEMALE: List<VoiceOption> = ZF_NAMES.mapIndexed { i, n -> VoiceOption(n, 3 + i) }

    /** 中文男声（sid 58 起连续） */
    val ZH_MALE: List<VoiceOption> = ZM_NAMES.mapIndexed { i, n -> VoiceOption(n, 58 + i) }

    fun options(gender: String): List<VoiceOption> = when (gender) {
        "male" -> ZH_MALE
        "en" -> EN_FEMALE
        else -> ZH_FEMALE
    }

    fun sidOf(id: String): Int? =
        (EN_FEMALE + ZH_FEMALE + ZH_MALE).firstOrNull { it.id == id }?.sid

    /** 性别筛选项（下拉）：中文女声 / 中文男声 / 英文女声 */
    val GENDERS: List<Pair<String, String>> = listOf(
        "female" to "中文女声",
        "male" to "中文男声",
        "en" to "英文女声",
    )
}

data class VoiceOption(val id: String, val sid: Int)

/**
 * 渲染测试台三个分组卡片（连接 / ASR 识别测试 / TTS 合成测试）。
 * 宽屏（wide()=true）：连接 + ASR 一行两列，TTS（奇数张）独占整行；窄屏：纵向单列。
 *
 * ⚠️ 响应式铁律（本项目反复踩坑）：凡依赖 observable 的文案/启用态/分支，必须写在
 * vif/velse 条件闭包或 attr 闭包内——构建期裸读（if/when 直接读 observable）只求值一次，
 * 状态变化后 UI 不会更新（实测「按钮三态不出现」的根因）。本文件所有分支均为 vif/velse 链。
 * ⚠️ 必须在目标容器（Scroller）闭包内**非限定**调用 `renderBench(bench, ctx, wide)`：
 * Kuikly 的 View{} DSL 静态绑定到词法作用域最近的 ViewContainer 接收者，
 * 以 `ctx.groupedCard(...)` 方式调用会把卡片挂到 Pager 根容器，导致布局逃逸。
 * ⚠️ wide 须为 lambda（如 `{ pagerData.pageViewWidth >= 900f }`），在 cardGrid 的
 * vif 闭包内读取才能随窗口 resize 响应式重排。
 */
fun ViewContainer<*, *>.renderBench(bench: TestBenchState, ctx: Pager, wide: () -> Boolean) {
    cardGrid(
        wide = wide,
        cards = listOf(
            AdminCard("连接") {
                labeledField("server url", { bench.serverUrl }, { bench.serverUrl = it }, "ws://127.0.0.1:8000/api/ws")
                labeledField("token（可选）", { bench.token }, { bench.token = it })
                actionRow {
                    vif({ bench.connected }) {
                        primaryButton("断开") { bench.disconnect(ctx) }
                    }
                    velse {
                        primaryButton("连接") { bench.connect(ctx) }
                    }
                    View { attr { width(AdminSpace.md) } }
                    statusBadge({ bench.connectionState }, { connectionLabel(bench.connectionState) })
                }
            },
            AdminCard("ASR 识别测试") {
                Text {
                    attr {
                        fontSize(AdminType.caption)
                        color(AdminColors.textSecondary)
                        marginTop(AdminSpace.xs)
                        text("流程：录音 → 停止后可反复试听 → 发送识别（SenseVoice 自动检测语言）")
                    }
                }
                actionRow {
                    // 五态按钮：每态一个 vif 分支（裸读 when 不随状态更新——见函数头注释）
                    vif({ bench.asrPhase == "starting" }) {
                        primaryButton("正在启动…", enabled = false, loading = true) { }
                    }
                    velse {
                        vif({ bench.asrPhase == "recording" }) {
                            primaryButton("停止录音", danger = true) { bench.stopRecording(ctx) }
                        }
                        velse {
                            vif({ bench.asrPhase == "recognizing" }) {
                                primaryButton("识别中…", enabled = false, loading = true) { }
                            }
                            velse {
                                vif({ bench.asrPhase == "recorded" }) {
                                    primaryButton("重新录音") { bench.startAsr(ctx) }
                                }
                                velse {
                                    primaryButton("开始录音") { bench.startAsr(ctx) }
                                }
                            }
                        }
                    }
                    View { attr { width(AdminSpace.md) } }
                    // 发送识别：仅在已录（且未在录/未在识别）时出现
                    vif({ bench.asrPhase == "recorded" }) {
                        primaryButton("发送识别") { bench.sendAsr(ctx) }
                        View { attr { width(AdminSpace.md) } }
                    }
                    vif({ bench.asrPhase == "recording" }) {
                        statusBadge({ "recording" }, { "录音中" })
                    }
                    velse {
                        vif({ bench.asrPhase == "recognizing" }) {
                            statusBadge({ "busy" }, { "识别中" })
                        }
                        velse {
                            statusBadge({ "idle" }, { "空闲" })
                        }
                    }
                }
                // 录音试听播放器（已录且有波形时显示）
                vif({ bench.recWave.isNotEmpty() }) {
                    waveformPlayer(
                        wave = { bench.recWave },
                        progress = { bench.recProgress },
                        playing = { bench.recPlaying },
                        durationText = { formatDuration(bench.recDuration) },
                        enabled = { bench.asrPhase != "recording" && !bench.ttsBusy },
                    ) { bench.togglePlayback(ctx, "recording") }
                }
                View { attr { height(AdminSpace.md) } }
                groupedCard("识别结果", withDivider = false) {
                    vif({ bench.asrText.isEmpty() }) {
                        Text {
                            attr {
                                fontSize(AdminType.body)
                                color(AdminColors.textTertiary)
                                text("识别结果将显示在此")
                            }
                        }
                    }
                    velse {
                        Text {
                            attr {
                                fontSize(AdminType.body)
                                color(AdminColors.textPrimary)
                                text(bench.asrText)
                            }
                        }
                        View { attr { height(AdminSpace.sm) } }
                        secondaryButton("复制") { /* 剪贴板需平台模块，从略，见交付备注 */ }
                    }
                }
            },
            AdminCard("TTS 合成测试") {
                labeledField("合成文字", { bench.ttsText }, { bench.ttsText = it }, "输入要合成的文字", height = 100f)
                // 语言：下拉（auto 走服务器默认；zh/en 显式指定）
                dropdownField(
                    label = "语言",
                    currentLabel = { bench.langOptions.firstOrNull { it.first == bench.ttsLang }?.second ?: bench.ttsLang },
                    options = { bench.langOptions },
                    selectedId = { bench.ttsLang },
                    isOpen = { bench.openDropdown == "lang" },
                    onToggle = { bench.openDropdown = if (bench.openDropdown == "lang") "" else "lang" },
                    onSelect = {
                        bench.ttsLang = it
                        bench.openDropdown = ""
                    },
                )
                // 音色性别：下拉（切换时自动落到该组第一个音色，并联动语言 zh/en）
                dropdownField(
                    label = "音色性别",
                    currentLabel = { bench.genderOptions.firstOrNull { it.first == bench.voiceGender }?.second ?: bench.voiceGender },
                    options = { bench.genderOptions },
                    selectedId = { bench.voiceGender },
                    isOpen = { bench.openDropdown == "gender" },
                    onToggle = { bench.openDropdown = if (bench.openDropdown == "gender") "" else "gender" },
                    onSelect = { g ->
                        bench.selectVoice(g, VoiceCatalog.options(g).firstOrNull()?.id ?: bench.voiceId)
                        // 英文音色配中文 lang 会产生错配音；测试台按性别联动语言，减少无效组合
                        bench.ttsLang = if (g == "en") "en" else "zh"
                        bench.openDropdown = ""
                    },
                )
                // 音色：下拉（Kokoro voices.bin 索引，55/45/3 项；选项列表随性别切换增删）
                dropdownField(
                    label = "音色",
                    currentLabel = { "${bench.voiceId}（sid ${bench.ttsSpeaker} · 共 ${bench.voiceOptions.size} 项）" },
                    options = { bench.voiceOptions },
                    selectedId = { bench.voiceId },
                    isOpen = { bench.openDropdown == "voice" },
                    onToggle = { bench.openDropdown = if (bench.openDropdown == "voice") "" else "voice" },
                    onSelect = { id ->
                        bench.selectVoice(bench.voiceGender, id)
                        bench.openDropdown = ""
                    },
                )
                labeledField("语速 (0.5~2.0)", { bench.ttsSpeed }, { bench.ttsSpeed = it })
                actionRow {
                    vif({ bench.ttsBusy }) {
                        primaryButton("合成中…", enabled = false, loading = true) { }
                    }
                    velse {
                        primaryButton(
                            "合成并播放",
                            enabled = bench.asrPhase != "recording" && bench.asrPhase != "starting" && bench.asrPhase != "recognizing",
                        ) {
                            bench.speak(ctx)
                        }
                    }
                }
                // TTS 波形回放（合成完成后显示，可反复回听；播放进度流过点亮）
                vif({ bench.ttsReady && bench.ttsWave.isNotEmpty() }) {
                    waveformPlayer(
                        wave = { bench.ttsWave },
                        progress = { bench.ttsProgress },
                        playing = { bench.ttsPlaying },
                        durationText = { formatDuration(bench.ttsDuration) },
                        enabled = { !bench.ttsBusy },
                    ) { bench.togglePlayback(ctx, "tts") }
                }
            },
        ),
    )
}

/** 语言下拉选项：auto（服务器默认）/ 中文 / 英文（Kokoro 模型仅支持中文与英文）。 */
private val LANG_OPTIONS: List<Pair<String, String>> = listOf(
    "auto" to "自动（服务器默认）",
    "zh" to "中文（zh）",
    "en" to "英文（en）",
)

/** 秒 → "m:ss" 播放器时间文本。 */
internal fun formatDuration(seconds: Double): String {
    val s = seconds.toInt().coerceAtLeast(0)
    return "${s / 60}:${(s % 60).toString().padStart(2, '0')}"
}
