package com.xiaozhi.admin


import com.tencent.kuikly.core.base.PagerScope
import com.tencent.kuikly.core.nvi.serialization.json.JSONArray
import com.tencent.kuikly.core.nvi.serialization.json.JSONObject
import com.xiaozhi.admin.state.stateVar
import com.xiaozhi.admin.state.stateListOf

/**
 * 配置 tab 的**全部表单草稿**（唤醒词 Stage 02 + 音频链 Stage 01/03/04/05 + Sendspin Stage 07），
 * 以及"设备快照 → 草稿"的填充与"草稿 → 请求体"的组装。
 *
 * 为什么单独成文件：这些字段原先挤在 `EchoMuseState`（1039 行）里，跟列表加载/审批/下发/音乐
 * 混在一起，任何一次「进设备卡」都会重填 50 多个 observable。拆出来后：
 * - [fill] 只在**换设备**或用户点「重新填充」时调用；
 * - [buildConfigBody] 是**纯函数**（只读草稿、只写返回值），因此「留空不发该键」这条部分更新
 *   语义可以逐条对账，不被 HTTP 回调打散；
 * - 真正的 POST 仍在 [EchoMuseState.saveConfig]（传输层与其它写动作同一处，不复制 URL/错误映射）。
 *
 * **部分更新语义（与固件"指针类型区分未设置 / 设为零"一致）**：数值与 JSON 字段留空 =
 * 不发该键（不把设备默认值写死成显式设置）；开关与枚举恒发（有明确视觉状态）。
 */

/** Stage 01 EQ 预设（官方在 EQ 卡上给的就是这几个档，这里按"写进 JSON 文本域"的方式生效）。 */
private val EM_EQ_PRESETS = listOf(
    "flat" to "平直 [0,0,0,0,0]",
    "vocal" to "人声 [2,0,-1,0,2]",
    "warm" to "暖 [3,1,0,-1,-2]",
    "bright" to "亮 [-2,-1,0,2,3]",
)

/** Stage 01 预设 → `eqBands` 文本（与 `EchoMuseConfigTab` 的下拉共用，避免两处各写一份）。 */
internal val EM_EQ_PRESET_VALUES: Map<String, String> = mapOf(
    "flat" to "[0,0,0,0,0]",
    "vocal" to "[2,0,-1,0,2]",
    "warm" to "[3,1,0,-1,-2]",
    "bright" to "[-2,-1,0,2,3]",
)

/** Stage 01 预设的可选清单（id ↔ 文案）。 */
internal fun emEqPresetOptions(): List<Pair<String, String>> = EM_EQ_PRESETS

class EchoMuseConfigDrafts(scope: PagerScope) {

    // ============================================================
    // Stage 02 唤醒词
    // ============================================================

    var wakeWordEnabled by stateVar(true)
    var owwOnDevice by stateVar("on")
    var owwModeOpen by stateVar(false)
    val owwModes: MutableList<Pair<String, String>> = stateListOf()
    var wakeLevelOpen by stateVar(false)
    val wakeLevels: MutableList<Pair<String, String>> = stateListOf()
    var owwModelOpen by stateVar(false)

    /** 模型下拉选项：官方四款 + 服务端 `oww-models/` 已上传模型 + 已存自定义值。 */
    val owwModelOptions: MutableList<Pair<String, String>> = stateListOf()

    /** 管理面存储的原始 `owwModel`；空 = 设备默认（hey jarvis），展示时要标出来。 */
    var owwModelStored by stateVar("")

    /** 下拉草稿：恒为生效模型名（存储空时解析为设备默认，见 [EchoMuseWakeWord]）。 */
    var owwModel by stateVar(EchoMuseWakeWord.DEFAULT_MODEL)

    /** 唤醒灵敏度阈值（读数保真：轨外历史值原样保留，滑杆端点钉扎见官方 wakeTrackFor）。 */
    var owwThreshold by stateVar(EchoMuseWakeWord.DEFAULT_THRESHOLD)
    var wakeSound by stateVar(true)

    /** 固件提示音档位（不是输出音量 0～127）。 */
    var wakeSoundLevel by stateVar("medium")

    /** 0=中心麦克风，范围 0~6。 */
    var wakeMic by stateVar("0")

    // ============================================================
    // Stage 01 / 03 / 05 音频链
    // ============================================================

    var vadThresholdDraft by stateVar("")
    var vadSpeechMsDraft by stateVar("")
    var vadSilenceMsDraft by stateVar("")
    var adcMicpgaDraft by stateVar("")
    var adcDigitalGainDraft by stateVar("")
    var micGainDbDraft by stateVar("")
    var startupVolumeDraft by stateVar("")
    var aecEnabled by stateVar(true)
    var aecDelayMsDraft by stateVar("")
    var aecTailMsDraft by stateVar("")
    var aecRefSource by stateVar("auto")
    var aecRefSourceOpen by stateVar(false)
    val aecRefSources: MutableList<Pair<String, String>> = stateListOf()
    var agcEnabled by stateVar(true)
    var beamformingEnabled by stateVar(true)
    var beamAngleDraft by stateVar("")
    var bargeInEnabled by stateVar(true)
    var bargeInThresholdDraft by stateVar("")

    /** 语音相对音乐的提升电平（官方 0/+6/+12 dB）。 */
    var responseLevel by stateVar("low")
    var responseLevelOpen by stateVar(false)
    val responseLevels: MutableList<Pair<String, String>> = stateListOf()
    var volumeButtonSound by stateVar(true)

    /** 遥控调音量时灯环青色弧（Stage 04）。 */
    var remoteVolumeArc by stateVar(false)
    var eqLoudness by stateVar(false)
    var bassGuardEnabled by stateVar(true)
    var bassGuardDbDraft by stateVar("")
    var limiterEnabled by stateVar(true)
    var limiterThresholdDraft by stateVar("")
    var limiterReleaseDraft by stateVar("")
    var duckDbDraft by stateVar("")

    /** EQ 频段 JSON（数字数组，如 `[2,0,-1,0,2]`）；空 = 不修改。 */
    var eqBandsDraft by stateVar("")
    var eqPresetOpen by stateVar(false)
    val eqPresets: MutableList<Pair<String, String>> = stateListOf()

    /**
     * 设备上已存的 `listeningAnim`（只读展示，Stage 04）。
     *
     * 一期**不给独立控件**（设计 §6.3 Stage 04：`listeningAnim` 是 P2 控件），要改走
     * Stage 05 末尾的「进阶 JSON」手填入口——那条路径同样过服务端白名单与形状校验。
     */
    var listeningAnimStored by stateVar("")

    /** Stage 05「进阶（JSON）」手填体（整对象，键名由服务端白名单裁定）；空 = 不发。 */
    var extraJsonDraft by stateVar("")

    // ============================================================
    // Stage 07 Sendspin
    // ============================================================

    var sendspinEnabled by stateVar(false)
    var sendspinName by stateVar("")

    /** 设备 `ConfigMessage` 的第三键（40 键白名单自 2026-10 起含它）。 */
    var sendspinUnpaired by stateVar(false)

    init {
        rebuildModeOptions()
        wakeLevels.addAll(
            listOf("quiet" to "轻柔（quiet）", "medium" to "适中（medium）", "loud" to "响亮（loud）"),
        )
        aecRefSources.addAll(
            listOf("auto" to "自动检测（推荐）", "hw" to "硬件环回（hw）", "sw" to "软件参考（sw）"),
        )
        responseLevels.addAll(listOf("low" to "低（0 dB）", "medium" to "中（+6 dB）", "high" to "高（+12 dB）"))
        eqPresets.addAll(emEqPresetOptions())
    }

    /** 当前 EQ 文本对应的预设名（匹配不上显示"自定义"）。 */
    val eqPresetLabel: String
        get() {
            val text = eqBandsDraft.trim()
            if (text.isEmpty()) return "（未修改）"
            val hit = EM_EQ_PRESET_VALUES.entries.firstOrNull { it.value == text }
            return hit?.let { key -> EM_EQ_PRESETS.firstOrNull { it.first == key.key }?.second } ?: "自定义"
        }

    /**
     * 模式下拉选项：shadow 为官方诊断档，仅当前值已是 shadow 时显示（官方同规则）；
     * 保存成功或重填草稿后按最新值重建，保证选中项总能找到标签。
     */
    fun rebuildModeOptions() {
        owwModes.clear()
        for ((value, label) in EchoMuseWakeWord.buildModeOptions(owwOnDevice)) {
            owwModes.add(value to label)
        }
    }

    /**
     * 按设备快照重填**全部**草稿（进 config tab / 换设备时调一次）。
     *
     * [uploadedModels] 是服务端 `oww-models/` 里的自定义模型清单（[EchoMuseState.modelFilesRefresh]
     * 拉取，失败保持空）。
     */
    fun fill(settings: JSONObject, uploadedModels: List<String>) {
        fillWake(settings, uploadedModels)
        fillAudio(settings)
        sendspinEnabled = settings.optBoolean("sendspinEnabled", false)
        sendspinName = settings.optString("sendspinName", "")
        sendspinUnpaired = settings.optBoolean("sendspinUnpaired", false)
        extraJsonDraft = ""
        eqPresetOpen = false
    }

    private fun fillWake(s: JSONObject, uploadedModels: List<String>) {
        wakeWordEnabled = s.optBoolean("wakeWordEnabled", true)
        owwOnDevice = when (val raw = s.opt("owwOnDevice")) {
            true -> "on"
            false -> "off"
            "off", "shadow", "on" -> raw.toString()
            else -> "on"
        }
        rebuildModeOptions()
        owwModeOpen = false
        wakeLevelOpen = false
        // 存储空 = 设备默认 hey jarvis（上游 OWW_MODEL 缺省），面板必须显示真实生效值
        owwModelStored = s.optString("owwModel", "").trim()
        owwModel = EchoMuseWakeWord.effectiveModel(owwModelStored)
        owwModelOpen = false
        // 官方四款 + oww-models/ 已上传自定义模型 + 已存自定义值（去重），纯函数可测
        owwModelOptions.clear()
        for ((value, label) in EchoMuseWakeWord.buildOwwOptions(owwModelStored, uploadedModels)) {
            owwModelOptions.add(value to label)
        }
        // 读数保真：轨外历史值（旧面板可写 0.05 或 1.0）不静默改写，滑杆把手钉在端点
        owwThreshold = when (val raw = s.opt("owwThreshold")) {
            is Number -> raw.toDouble().takeIf { it.isFinite() } ?: EchoMuseWakeWord.DEFAULT_THRESHOLD
            else -> EchoMuseWakeWord.DEFAULT_THRESHOLD
        }
        wakeSound = s.optBoolean("wakeSound", true)
        wakeSoundLevel = when (val raw = s.opt("wakeSoundLevel")) {
            "quiet", "medium", "loud" -> raw.toString()
            is Number -> "medium" // 旧数字从未被固件接受，不能猜测有效档位
            else -> "medium"
        }
        wakeMic = intText(s.opt("wakeMic"), "0")
    }

    private fun fillAudio(s: JSONObject) {
        fun num(key: String): String = (s.opt(key) as? Number)?.let {
            if (it.toDouble() % 1.0 == 0.0) it.toLong().toString() else it.toString()
        } ?: ""
        vadThresholdDraft = num("vadThreshold")
        vadSpeechMsDraft = num("vadSpeechMs")
        vadSilenceMsDraft = num("vadSilenceMs")
        adcMicpgaDraft = num("adcMicpga")
        adcDigitalGainDraft = num("adcDigitalGain")
        micGainDbDraft = num("micGainDb")
        startupVolumeDraft = num("startupVolume")
        aecEnabled = s.optBoolean("aecEnabled", true)
        aecDelayMsDraft = num("aecDelayMs")
        aecTailMsDraft = num("aecTailMs")
        aecRefSource = s.optString("aecRefSource", "auto").takeIf { it in setOf("auto", "hw", "sw") } ?: "auto"
        aecRefSourceOpen = false
        agcEnabled = s.optBoolean("agcEnabled", true)
        beamformingEnabled = s.optBoolean("beamformingEnabled", true)
        beamAngleDraft = num("beamAngle")
        bargeInEnabled = s.optBoolean("bargeInEnabled", true)
        bargeInThresholdDraft = num("bargeInThreshold")
        responseLevel = s.optString("responseLevel", "low").takeIf { it in setOf("low", "medium", "high") } ?: "low"
        responseLevelOpen = false
        volumeButtonSound = s.optBoolean("volumeButtonSound", true)
        remoteVolumeArc = s.optBoolean("remoteVolumeArc", false)
        eqLoudness = s.optBoolean("eqLoudness", false)
        bassGuardEnabled = s.optBoolean("bassGuardEnabled", true)
        bassGuardDbDraft = num("bassGuardDb")
        limiterEnabled = s.optBoolean("limiterEnabled", true)
        limiterThresholdDraft = num("limiterThreshold")
        limiterReleaseDraft = num("limiterRelease")
        duckDbDraft = num("duckDb")
        eqBandsDraft = s.optJSONArray("eqBands")?.toString().orEmpty()
        listeningAnimStored = s.optJSONObject("listeningAnim")?.toString().orEmpty()
    }

    /**
     * 草稿 → `POST …/settings` 请求体。**校验失败返回 null** 并回调 [onError]（不发半截更新）。
     *
     * 数值/JSON 留空 = 不发该键；开关与枚举恒发。
     */
    fun buildConfigBody(onError: (String) -> Unit): JSONObject? {
        if (owwOnDevice !in setOf("off", "shadow", "on") ||
            wakeSoundLevel !in setOf("quiet", "medium", "loud")
        ) {
            onError("请选择有效的本地唤醒模式和提示音档位")
            return null
        }
        if (!owwThreshold.isFinite() || owwThreshold <= 0.0 || owwThreshold > 1.0) {
            onError("唤醒阈值必须大于 0 且不超过 1")
            return null
        }
        val mic = wakeMic.trim().toIntOrNull()
        if (mic == null || mic !in 0..6) {
            onError("麦克风编号需要是 0~6 的整数（0=中心麦克风）")
            return null
        }
        val body = JSONObject()
        var failed: String? = null
        fun int(key: String, draft: String, range: IntRange, label: String) {
            val t = draft.trim()
            if (t.isEmpty() || failed != null) return
            val v = t.toIntOrNull()
            if (v == null || v !in range) {
                failed = "$label 需要 $range 范围内的整数"
                return
            }
            body.put(key, v)
        }
        fun dec(key: String, draft: String, lo: Double, hi: Double, label: String) {
            val t = draft.trim()
            if (t.isEmpty() || failed != null) return
            val v = t.toDoubleOrNull()
            if (v == null || !v.isFinite() || v < lo || v > hi) {
                failed = "$label 需要 $lo ～ $hi 之间的数字"
                return
            }
            body.put(key, v)
        }
        // 语音结束判定（Stage 05）
        dec("vadThreshold", vadThresholdDraft, 0.0001, 1.0, "VAD 门限")
        int("vadSpeechMs", vadSpeechMsDraft, 0..60000, "开口判定时长")
        int("vadSilenceMs", vadSilenceMsDraft, 0..60000, "收尾静音时长")
        // 采集增益（Stage 03）
        int("adcMicpga", adcMicpgaDraft, 0..127, "模拟前端增益")
        int("adcDigitalGain", adcDigitalGainDraft, 0..127, "数字增益")
        int("micGainDb", micGainDbDraft, 0..42, "处理增益（dB）")
        int("startupVolume", startupVolumeDraft, 0..127, "开机音量")
        // AEC（Stage 03）
        int("aecDelayMs", aecDelayMsDraft, 0..1000, "AEC 参考延时")
        int("aecTailMs", aecTailMsDraft, 0..500, "AEC 尾长")
        // 波束与打断（Stage 03 / 02）
        dec("beamAngle", beamAngleDraft, -1.0, 360.0, "波束角度")
        dec("bargeInThreshold", bargeInThresholdDraft, 0.0, 1.0, "打断阈值")
        // 输出链（Stage 01）
        dec("bassGuardDb", bassGuardDbDraft, -60.0, 24.0, "低音保护阈值")
        dec("limiterThreshold", limiterThresholdDraft, -60.0, 0.0, "限幅阈值")
        dec("limiterRelease", limiterReleaseDraft, 10.0, 2000.0, "限幅释放时间")
        dec("duckDb", duckDbDraft, -60.0, 0.0, "音乐压低深度")
        failed?.let {
            onError(it)
            return null
        }
        // JSON 进阶键：非空时必须能解析且形状正确（构造失败 = 报错，不发半截更新）
        val eqText = eqBandsDraft.trim()
        if (eqText.isNotEmpty()) {
            val bands = try { JSONArray(eqText) } catch (_: Exception) { null }
            if (bands == null || bands.length() == 0 || (0 until bands.length()).any { bands.opt(it) !is Number }) {
                onError("EQ 频段需要是非空数字数组，如 [2,0,-1,0,2]")
                return null
            }
            body.put("eqBands", bands)
        }
        val extraText = extraJsonDraft.trim()
        if (extraText.isNotEmpty()) {
            val extra = try { JSONObject(extraText) } catch (_: Exception) { null }
            if (extra == null) {
                onError("进阶 JSON 需要是对象，如 {\"listeningAnim\":{\"pattern\":\"spin\"}}")
                return null
            }
            val keys = extra.keys()
            while (keys.hasNext()) {
                val key = keys.next()
                body.put(key, extra.opt(key))
            }
        }
        // 唤醒词（Stage 02）：恒发有视觉状态的字段
        body.put("wakeWordEnabled", wakeWordEnabled)
        body.put("owwOnDevice", owwOnDevice)
        body.put("owwModel", owwModel.trim())
        body.put("owwThreshold", owwThreshold)
        body.put("wakeSound", wakeSound)
        body.put("wakeSoundLevel", wakeSoundLevel)
        body.put("wakeMic", mic)
        // 音频开关/枚举（Stage 01/03/04）：恒发，写回现值无害
        body.put("aecEnabled", aecEnabled)
        body.put("agcEnabled", agcEnabled)
        body.put("beamformingEnabled", beamformingEnabled)
        body.put("bargeInEnabled", bargeInEnabled)
        body.put("responseLevel", responseLevel)
        body.put("volumeButtonSound", volumeButtonSound)
        body.put("remoteVolumeArc", remoteVolumeArc)
        body.put("eqLoudness", eqLoudness)
        body.put("bassGuardEnabled", bassGuardEnabled)
        body.put("limiterEnabled", limiterEnabled)
        // Sendspin（Stage 07）
        body.put("sendspinEnabled", sendspinEnabled)
        body.put("sendspinName", sendspinName.trim())
        body.put("sendspinUnpaired", sendspinUnpaired)
        return body
    }

    /** 服务端麦克风编号转草稿；未设置时采用设备默认值。 */
    private fun intText(raw: Any?, fallback: String): String =
        (raw as? Number)?.let { it.toInt().toString() } ?: fallback
}
