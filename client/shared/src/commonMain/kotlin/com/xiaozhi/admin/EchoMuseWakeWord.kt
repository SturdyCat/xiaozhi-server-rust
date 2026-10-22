package com.xiaozhi.admin

/**
 * EchoMuse 唤醒词展示/选择纯逻辑（上游锚点：wilbowes/EchoMuse @858987c）。
 *
 * 为什么需要它：设备端有真实的默认唤醒词，而管理面存储的 `owwModel` 可以为空
 * （从未单独设置），面板不能把"空存储值"显示成"没有唤醒词"。
 *
 * - **设备默认模型**：上游 `device/internal/config/config.go` 的 `loadDefaults`
 *   里 `OWW_MODEL` 环境变量缺省 `hey_jarvis_v0.1`。本服务器的 config 推送基线
 *   不含 `owwModel`，设备未被单独设置时实际监听的就是它（人读唤醒词
 *   "hey jarvis"）。
 * - **官方内置唤醒词**：上游 `controller/em_oww_assets.py` 的 `STOCK_MODELS`，
 *   四款在设备开通资产同步时全部预装；openwakeword 另带的 `timer`/`weather`
 *   是意图模型，上游明确不作为唤醒词提供。设备本地评分（`owwOnDevice=on`）
 *   只能使用设备已预装的分类器——切到设备没有的模型会让它听不到任何唤醒词
 *   （上游 #191 的教训），而本服务器不做上游 shell 面的模型分发，因此面板提供
 *   官方四款，外加服务端 `oww-models/` 已上传的自定义模型与已存的自定义值原样保留。
 *   注意：把 .onnx 上传到服务端 `oww-models/`（「文件」页）只是让管理台可选，
 *   **上传 ≠ 安装**——设备仍只认本地已装模型，未安装的模型选了也听不到唤醒词。
 * - **人读名字规则**：上游 `controller/em_oww_models.py` 的 `display_name`——
 *   先取 prediction key（`.onnx` 路径取文件名主干，普通名取自身），再去掉
 *   `_v<数字>` 版本后缀、下划线转空格（`hey_jarvis_v0.1` → "hey jarvis"）。
 *   版本后缀是打包约定，不属于唤醒词本身。
 * - **修改机制**：管理面 settings `owwModel` → 服务端随 `config` 下发，设备端
 *   `Apply` 对非空值生效（空值被忽略，所以"恢复默认"必须显式发
 *   `hey_jarvis_v0.1`，发空串不会重置）。
 */
object EchoMuseWakeWord {
    /** 设备固件缺省唤醒词模型（上游 config.go：`OWW_MODEL` 默认 hey_jarvis_v0.1）。 */
    const val DEFAULT_MODEL = "hey_jarvis_v0.1"

    /** 官方内置唤醒词模型（上游 em_oww_assets.py `STOCK_MODELS`，设备开通时全部预装）。 */
    val STOCK_MODELS = listOf(
        "hey_jarvis_v0.1",
        "alexa_v0.1",
        "hey_mycroft_v0.1",
        "hey_rhasspy_v0.1",
    )

    /** 管理面存储值 → 设备实际生效模型：未设置（空/空白）即设备默认。 */
    fun effectiveModel(stored: String): String = stored.trim().ifEmpty { DEFAULT_MODEL }

    /** 模型名 → 人读唤醒词：去 `_v<数字>` 版本后缀、下划线转空格（上游 display_name）。 */
    fun displayName(model: String): String =
        effectiveModel(model)
            .replace(Regex("_v\\d+(?:\\.\\d+)*$"), "")
            .replace("_", " ")

    /** 是否官方内置模型（面板据此决定是否追加"自定义"选项；空存储值视为默认款）。 */
    fun isStock(stored: String): Boolean = effectiveModel(stored) in STOCK_MODELS

    /** 下拉/展示用的选项标签，如 "hey jarvis（hey_jarvis_v0.1）"。 */
    fun optionLabel(model: String): String = "${displayName(model)}（${effectiveModel(model)}）"

    // ============================================================
    // 灵敏度阈值：官方量程/步进/滑杆方向（dashboard.jsx WAKE_T_* + reflectT，
    // 对照设备端 device/internal/config/config.go 的 OWW_THRESHOLD 默认 0.5）
    // ============================================================

    /** 设备端阈值缺省（上游 config.go：`OWW_THRESHOLD` 默认 0.5）。 */
    const val DEFAULT_THRESHOLD = 0.5

    /**
     * 官方灵敏度量程与步进（dashboard.jsx `WAKE_T_MIN/MAX/STEP` = 0.1/0.975/0.025）：
     * - 上限 0.975 而非 1.0：openWakeWord 分数是低于 1.0 即饱和的 sigmoid，设备端
     *   shadow.go 与控制器侧的判定都是 `score >= threshold`，1.0 是任何帧都够不到
     *   的门槛（官方实测 18,021 帧峰值 0.999），0.975 严格但可达；
     * - 步进 0.025：精确/召回的权衡集中在 0.9 以上，0.1 步进会直接跳过该区间。
     * 服务端白名单仍是 (0,1]：轨外历史值允许保存，仅滑杆按官方量程钉扎（见下）。
     */
    const val THRESHOLD_MIN = 0.1
    const val THRESHOLD_MAX = 0.975
    const val THRESHOLD_STEP = 0.025

    /**
     * 滑杆位置（Kuikly Slider 的 0..1 progress）。官方滑杆左端「Precise（阈值高）」
     * 右端「Eager（阈值低）」，与阈值反向（官方 reflectT 是自身逆函数），这里换算成
     * progress = (MAX - t) / (MAX - MIN)。轨外存储值（旧面板可写 0.05 或 1.0）按官方
     * wakeTrackFor 钉在端点：读数仍显示存储值，直到拖动才写入轨内新值。
     */
    fun trackProgress(threshold: Double): Float {
        val t = if (threshold.isFinite()) threshold else DEFAULT_THRESHOLD
        return ((THRESHOLD_MAX - t) / (THRESHOLD_MAX - THRESHOLD_MIN)).toFloat().coerceIn(0f, 1f)
    }

    /** 拖动换算：progress → 阈值，按官方 0.025 步进量化并限官方量程。 */
    fun quantizeTrack(progress: Float): Double {
        val p = (if (progress.isFinite()) progress else 0f).coerceIn(0f, 1f).toDouble()
        val raw = THRESHOLD_MAX - p * (THRESHOLD_MAX - THRESHOLD_MIN)
        return (kotlin.math.round(raw / THRESHOLD_STEP) * THRESHOLD_STEP)
            .coerceIn(THRESHOLD_MIN, THRESHOLD_MAX)
    }

    // ============================================================
    // 唤醒模式：官方「Wake word detection」下拉（dashboard.jsx）
    // ============================================================

    /**
     * 模式选项，标签与顺序对齐官方 Select：`On this Echo` / `On the controller`
     * 常驻；`shadow` 是开发者诊断档（持续推流、双端同时打分），官方仅在该设备已
     * 处于 shadow 时作为第三项显示，避免误开——本面板同规则，服务端仍接受三值。
     */
    fun buildModeOptions(current: String): List<Pair<String, String>> = buildList {
        add("on" to "On this Echo（设备本地识别，官方推荐）")
        add("off" to "On the controller（控制器侧识别）")
        if (current == "shadow") add("shadow" to "Diagnostic（诊断：持续推流双端打分）")
    }

    /** 选中模式的官方解释（dashboard.jsx Select sub 文案移植）。 */
    fun modeNote(mode: String): String = when (mode) {
        "on" -> "Echo 自己监听唤醒词，听到之前不向外发送任何音频；触发后只上传你说的话，直到你停止（官方推荐）。"
        "off" -> "麦克风持续推流到控制器（本服务端），由控制器侧 openWakeWord 打分监听唤醒词。"
        "shadow" -> "官方诊断档：持续推流且设备与控制器同时打分、可对比核对（仅诊断用）。"
        else -> ""
    }

    /**
     * 唤醒词模型下拉选项（纯函数，进唤醒词面板时由 [EchoMuseState] 调用并重建）：
     * - 官方四款在前（默认款带「默认」标记）；
     * - 服务端 `oww-models/` 已上传的自定义模型（[uploadedModelFiles] 里的 `.onnx` 文件名）
     *   依次追加：value = 去扩展名的模型主干（设备按 `<stem>.onnx` 加载），
     *   label = `自定义（<stem>，oww-models/<文件名>）`；非 `.onnx` 文件跳过；
     * - 管理面已存的非官方值（[stored]）原样保留（设备可能被单独预装过该模型），
     *   与官方款/上传模型同名时去重，不重复出现；
     * - 上传 ≠ 安装：选项可点不代表设备已装该模型，卡内说明文案保留这一警告。
     */
    fun buildOwwOptions(stored: String, uploadedModelFiles: List<String>): List<Pair<String, String>> {
        val options = mutableListOf<Pair<String, String>>()
        val seen = mutableSetOf<String>()
        for (model in STOCK_MODELS) {
            val mark = if (model == DEFAULT_MODEL) "，默认" else ""
            options.add(model to "${optionLabel(model)}$mark")
            seen.add(model)
        }
        for (file in uploadedModelFiles) {
            if (!FilesModel.isModelFile(file)) continue
            val stem = FilesModel.modelStem(file)
            if (stem.isEmpty() || !seen.add(stem)) continue
            options.add(stem to "自定义（$stem，oww-models/$file）")
        }
        val storedTrimmed = stored.trim()
        if (storedTrimmed.isNotEmpty() && storedTrimmed !in STOCK_MODELS && seen.add(storedTrimmed)) {
            options.add(storedTrimmed to "自定义（$storedTrimmed，需设备已预装）")
        }
        return options
    }
}
