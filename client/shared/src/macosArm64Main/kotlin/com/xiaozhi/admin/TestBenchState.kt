package com.xiaozhi.admin

import com.tencent.kuikly.core.base.ViewContainer
import com.tencent.kuikly.core.directives.velse
import com.tencent.kuikly.core.directives.vif
import com.tencent.kuikly.core.pager.Pager
import com.tencent.kuikly.core.reactive.handler.observable
import com.tencent.kuikly.core.timer.setTimeout
import com.tencent.kuikly.core.views.Text
import com.tencent.kuikly.core.views.View

/**
 * ASR / TTS 测试台状态（macOS 专用，macosArm64Main）。
 *
 * 持有连接 / 录音 / 合成 / 识别结果等状态，方法 connect/disconnect/startAsr/stopAsr/speak
 * 照 TestPage.kt 现有实现调用 XiaoZhiModule（web/Android/iOS 不注册该模块，故测试功能仅 Mac App）。
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
    var recording by observable(false)
    var speaking by observable(false)

    /** 录音启动中（权限申请/引擎启动，尚未真正录）：按钮显示菊花并禁止重复点击 */
    var asrStarting by observable(false)

    /** 等待识别结果中（已发 asr_test stop，服务端一次性识别）：按钮显示菊花并禁止重复点击 */
    var asrBusy by observable(false)

    /** 合成进行中（已发 tts_test，等待服务端 start→音频→stop 全流程）：按钮菊花。与 speaking 同义，
     *  单独字段以区分「等待服务端」与「正在播放」两个语义（播放中禁止再次合成）。 */
    var ttsBusy by observable(false)
    var asrText by observable("")
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

    /** 开始录音：asrStarting 菊花期间挡住重复点击；权限被拒时原生仅打日志，
     *  超时兜底进入录音态（用户能点「停止并识别」恢复，不阻塞 UI）。 */
    fun startAsr(ctx: Pager) {
        if (asrStarting || recording || asrBusy) return
        asrStarting = true
        asrText = ""
        statusMsg = "正在启动录音…"
        xz(ctx).startAsr { result ->
            // 回调在「识别结果回来」时才触发（keepCallbackAlive），此处只处理识别文本
            asrStarting = false
            asrBusy = false
            recording = false
            asrText = result?.optString("text", "") ?: ""
            statusMsg = "识别完成"
        }
        // 权限框弹出后无回调分支：0.8s 后认为进入录音态（按钮变「停止并识别」）
        ctx.setTimeout(800) {
            if (asrStarting) {
                asrStarting = false
                recording = true
                statusMsg = "录音中…（再点一次停止并识别）"
            }
        }
    }

    fun stopAsr(ctx: Pager) {
        if (!recording || asrBusy) return
        recording = false
        asrBusy = true
        statusMsg = "识别中…"
        xz(ctx).stopAsr()
        // 服务端识别（SenseVoice 整段）超时兜底：30s 未回 stt 则恢复按钮，避免永久卡死
        ctx.setTimeout(30_000) {
            if (asrBusy) {
                asrBusy = false
                statusMsg = "识别超时：请检查服务器日志或重试"
            }
        }
    }

    fun speak(ctx: Pager) {
        if (ttsBusy) return
        if (ttsText.isBlank()) {
            statusMsg = "请输入要合成的文字"
            return
        }
        ttsBusy = true
        speaking = true
        statusMsg = "合成中…"
        val speed = ttsSpeed.toDoubleOrNull() ?: 1.0
        val speaker = ttsSpeaker
        val lang = resolveLang(ttsLang)
        xz(ctx).speak(ttsText, speaker, lang, speed) { _ ->
            ttsBusy = false
            speaking = false
            statusMsg = "TTS 播放完成"
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

    /** 语音选择：切换性别时自动落到该性别第一个音色；同步 sid（老上游兼容字段）。 */
    fun selectVoice(gender: String, id: String) {
        voiceGender = gender
        voiceId = id
        VoiceCatalog.sidOf(id)?.let { ttsSpeaker = it }
    }

    // ============================================================
    // 渲染：见文件底部 ViewContainer.renderBench(bench, ctx) 扩展
    // ============================================================

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
 * ⚠️ 必须在目标容器（Scroller）闭包内**非限定**调用 `renderBench(bench, ctx, wide)`：
 * Kuikly 的 View{} DSL 静态绑定到词法作用域最近的 ViewContainer 接收者，
 * 以 `ctx.groupedCard(...)` 方式调用会把卡片挂到 Pager 根容器，导致布局逃逸。
 * AdminCard 的 content（ViewBuilder，带接收者）在 groupedCard 卡片容器内执行，节点挂进卡片。
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
                    primaryButton(if (bench.connected) "断开" else "连接") {
                        if (bench.connected) bench.disconnect(ctx) else bench.connect(ctx)
                    }
                    View { attr { width(AdminSpace.md) } }
                    statusBadge(bench.connectionState, connectionLabel(bench.connectionState))
                }
            },
            AdminCard("ASR 识别测试") {
                Text {
                    attr {
                        fontSize(AdminType.caption)
                        color(AdminColors.textSecondary)
                        marginTop(AdminSpace.xs)
                        text("语言由 SenseVoice 自动检测（zh/en/ja/ko/yue）；说话结束后点「停止并识别」")
                    }
                }
                actionRow {
                    // 四态按钮：启动中(菊花) / 识别中(菊花) / 录音中(红色，点击停止并识别) / 空闲。
                    // 合成中禁用录音；录音/启动/识别中禁用 TTS 合成（见下方 TTS 组按钮）。
                    when {
                        bench.asrStarting -> primaryButton("正在启动…", enabled = false, loading = true) { }
                        bench.asrBusy -> primaryButton("识别中…", enabled = false, loading = true) { }
                        bench.recording -> primaryButton("停止并识别", danger = true, enabled = !bench.speaking) {
                            bench.stopAsr(ctx)
                        }
                        else -> primaryButton("开始录音", enabled = !bench.speaking) { bench.startAsr(ctx) }
                    }
                    View { attr { width(AdminSpace.md) } }
                    vif({ bench.recording }) {
                        statusBadge("recording", "录音中")
                    }
                    velse {
                        vif({ bench.asrBusy }) {
                            statusBadge("busy", "识别中")
                        }
                        velse {
                            statusBadge("idle", "空闲")
                        }
                    }
                }
                View { attr { height(AdminSpace.md) } }
                groupedCard("识别结果", withDivider = false) {
                    if (bench.asrText.isEmpty()) {
                        Text {
                            attr {
                                fontSize(AdminType.body)
                                color(AdminColors.textTertiary)
                                text("识别结果将显示在此")
                            }
                        }
                    } else {
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
                    currentLabel = { LANG_OPTIONS.firstOrNull { it.first == bench.ttsLang }?.second ?: bench.ttsLang },
                    options = LANG_OPTIONS,
                    selectedId = { bench.ttsLang },
                    isOpen = { bench.openDropdown == "lang" },
                    onToggle = { bench.openDropdown = if (bench.openDropdown == "lang") "" else "lang" },
                    onSelect = {
                        bench.ttsLang = it
                        bench.openDropdown = ""
                    },
                )
                // 音色性别：下拉（切换时自动落到该组第一个音色，并同步语言 zh/en）
                dropdownField(
                    label = "音色性别",
                    currentLabel = { GenderOptions.firstOrNull { it.first == bench.voiceGender }?.second ?: bench.voiceGender },
                    options = GenderOptions,
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
                // 音色：下拉（Kokoro voices.bin 索引，55/45/3 项，面板内可滚动）
                dropdownField(
                    label = "音色（共 ${VoiceCatalog.options(bench.voiceGender).size} 项）",
                    currentLabel = { "${bench.voiceId}（sid ${bench.ttsSpeaker}）" },
                    options = VoiceCatalog.options(bench.voiceGender).map { it.id to "${it.id}（sid ${it.sid}）" },
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
                    // 录音/启动/识别中禁用合成；合成中显示菊花并堵重复点击
                    if (bench.ttsBusy) {
                        primaryButton("合成中…", enabled = false, loading = true) { }
                    } else {
                        primaryButton("合成并播放", enabled = !bench.recording && !bench.asrStarting && !bench.asrBusy) {
                            bench.speak(ctx)
                        }
                    }
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

/** 音色性别下拉选项（与 VoiceCatalog.GENDERS 同源）。 */
private val GenderOptions: List<Pair<String, String>> = VoiceCatalog.GENDERS
