package com.xiaozhi.admin

import com.xiaozhi.admin.ui.*

import com.tencent.kuikly.core.coroutines.Job
import com.tencent.kuikly.core.coroutines.delay
import com.tencent.kuikly.core.coroutines.launch
import com.tencent.kuikly.core.nvi.serialization.json.JSONObject
import com.tencent.kuikly.core.pager.Pager
import com.tencent.kuikly.core.reactive.handler.observable

/**
 * 设备详情卡的**分区状态**：进了哪个 tab、进 tab 时的懒加载、`runtime` 结构化快照、
 * `system/info` 折叠区、音量下发、诊断汇总、OWW 资产现状，以及控制台会话的归属。
 *
 * 与旧的 `EchoMuseManagementState` 的关系：那份把"状态与音量/系统信息/日志/维护"四个
 * 分区塞在一个对象里，日志与诊断混在一起、进入即抓取、离开不停轮询。本类按设计
 * §6.4/§7-T03 拆开：
 * - **懒加载**：进哪个 tab 才抓哪个，抓过一次不重复抓（`runtimeLoaded`/`infoLoaded`）；
 * - **进 tab 开、离 tab 停**：控制台会话与 `oww/state` 的 30 s 轮询都在 [enterTab] 里
 *   起、在离开时停（[leave] / 切走 tab），页面离开后不留后台请求；
 * - **日志独立**：[EchoMuseLogsState] 只管来源白名单与输出，设备事实与单操作闸门
 *   仍在这里（同一台设备只有一把槽，见 §8.4）。
 *
 * T04 之后 [tab] 是**唯一**的视图入口：旧的卡内左菜单（`section`/`manageTabIndex`/
 * 七个分区 id 常量）已随六 tab 落地删除。
 */
class EchoMuseDetailState(private val scope: Pager) {

    // ---- 六 tab（§6.2 归位表；`tab` 存的就是 tabBar 的标签文本，见 §6.6 示例）----

    /** 当前 tab（默认「状态」，与官方 dashboard 打开设备即看状态一致）。 */
    var tab by scope.observable(EM_TAB_STATUS)

    /** 当前展开的设备号（空串 = 未展开）。切设备会清掉所有设备域快照。 */
    var target by scope.observable("")
        private set

    /**
     * 状态 tab「系统诊断（原始输出）」是否展开（默认收起：原文可能几百行，
     * 一进状态页就铺开会把资源块挤到屏外）。
     */
    var infoOpen by scope.observable(false)

    // ---- 设备事实（列表同步而来，不由本类请求）----

    var approved by scope.observable(false)
        private set
    var online by scope.observable(false)
        private set
    var emos by scope.observable(false)
        private set
    var speaker by scope.observable(false)
        private set

    /** 当前 control 会话标识：变化即"设备重连过"，旧快照一律标历史。 */
    var sessionId by scope.observable("")
        private set

    // ---- 单操作闸门（控制台之外的 `/system/…`、音量、配置下发共用一把槽）----

    /** 进行中的操作名（空 = 空闲）；`system/info`、`volume` 等直接当加载态用。 */
    var busy by scope.observable("")
        private set
    var error by scope.observable("")
        private set
    var result by scope.observable("")
        private set

    // ---- 运行状态快照 ----

    var runtime by scope.observable<JSONObject?>(null)
        private set
    var runtimeLoaded by scope.observable(false)
        private set

    /** 快照是否代表**当前**会话（设备在线 + 同一 session）；false = 历史快照。 */
    var runtimeCurrent by scope.observable(false)
        private set

    // ---- 系统信息（手动一次的原文快照）----

    var infoOutput by scope.observable("")
        private set
    var infoLoaded by scope.observable(false)
        private set
    var infoHistorical by scope.observable(false)
        private set

    // ---- OWW 资产现状（更新 tab，30 s 轮询；只读，不占单操作闸门）----

    /** `GET …/oww/state` 的原始回包（五态 `installed|outdated|blocked|unknown|missing`）。 */
    var owwState by scope.observable<JSONObject?>(null)
        private set
    var owwLoaded by scope.observable(false)
        private set
    var owwLoading by scope.observable(false)
        private set

    // ---- 维护 ----

    var confirmReboot by scope.observable(false)

    /** 输出音量草稿（0～127；读取运行状态不覆盖它）。 */
    var volumeDraft by scope.observable("")

    /** 控制台会话（T05 的 tab 视图直接读它渲染输出区）。 */
    val console = EchoMuseConsoleState(scope)

    private var base = ""
    private var token = ""
    private var generation = 0
    /** 更新 tab 的资产状态轮询句柄（离开 tab / 页面销毁即 cancel）。 */
    private var owwPollJob: Job? = null

    /** 能否执行系统类一次性操作（已批准 + emOS + 在线 + 无并发操作）。 */
    val canSystem: Boolean get() = approved && emos && online
    val canVolume: Boolean get() = approved && emos && online && speaker
    val idle: Boolean get() = busy.isEmpty()

    /**
     * 连接参数变化（换服务器/换令牌）：所有在途回包与快照立即失效——否则旧服务器
     * 的 runtime 会被当成本服务器设备的状态展示。
     */
    fun configureConnection(baseUrl: String, authToken: String) {
        if (base == baseUrl && token == authToken) return
        base = baseUrl
        token = authToken
        generation++
        console.close()
        resetDeviceState()
        target = ""
    }

    /**
     * 列表刷新后同步设备事实（列表是唯一事实来源，本类不自己猜在线态）。
     *
     * 在线/批准/系统/会话任一变化都要**作废在途回包并给旧快照打历史标**：
     * 设备重连后 `session_id` 会变，旧的 runtime 已经不是这台设备"现在"的状态了。
     */
    fun sync(rows: List<EchoMuseDeviceRow>) {
        if (target.isEmpty()) return
        val row = rows.firstOrNull { it.deviceId == target }
        val nextApproved = row?.approved == true
        val nextOnline = row?.online == true
        val nextEmos = row?.baseOs == "emos"
        val nextSpeaker = row?.capabilities?.contains("speaker") == true
        val nextSession = row?.sessionId.orEmpty()
        if (approved != nextApproved || online != nextOnline || emos != nextEmos ||
            speaker != nextSpeaker || sessionId != nextSession
        ) {
            generation++
            busy = ""
            runtimeCurrent = false
            infoHistorical = infoLoaded
            confirmReboot = false
            result = ""
        }
        approved = nextApproved
        online = nextOnline
        emos = nextEmos
        speaker = nextSpeaker
        sessionId = nextSession
        if (row == null) {
            target = ""
            leave()
        }
    }

    // ============================================================
    // 进入 / 离开（六 tab）
    // ============================================================

    /**
     * 切到某个 tab（**幂等**：同设备同 tab 重复调用不重复抓取、不重开会话）。
     *
     * 每个需要设备数据的 tab 在这里决定自己的第一件事：
     * - 状态/活动：`runtime` 抓一次（两个 tab 共用同一份快照）；
     * - 更新：`oww/state` 抓一次 + 起 30 s 轮询；
     * - 控制台：开会话（离线则直接 `offline` 态，不发 WS）；
     * - 配置/日志：不预取（配置读的是列表里的 `settings`，日志要用户点"读取"）。
     *
     * 离开时反着收：停了轮询、关了控制台会话——**离开立刻停**（§7-T03）。
     */
    fun enterTab(label: String, row: EchoMuseDeviceRow, ctx: Pager) {
        if (target != row.deviceId) bind(row)
        if (tab != label) tab = label
        if (label != EM_TAB_UPDATES) stopOwwPolling()
        if (label != EM_TAB_CONSOLE) console.close()

        when (label) {
            EM_TAB_STATUS, EM_TAB_ACTIVITY -> if (!runtimeLoaded) readRuntime(ctx)
            EM_TAB_UPDATES -> {
                readOwwState(ctx)
                armOwwPolling(ctx)
            }
            EM_TAB_CONSOLE -> console.open(row.deviceId, row.online, base, token)
            else -> Unit
        }
    }

    /** 离开设备详情卡（收起/换页/销毁）：停轮询、关控制台、忘掉当前设备。 */
    fun leave() {
        stopOwwPolling()
        console.close()
        if (target.isNotEmpty()) {
            target = ""
            tab = EM_TAB_STATUS
            infoOpen = false
        }
    }

    // ============================================================
    // 读取与下发
    // ============================================================

    /** 读一次运行状态快照（不自动刷新；在线才代表当前值）。 */
    fun readRuntime(ctx: Pager) {
        if (!approved || !idle) return
        run("runtime", system = false) { data ->
            if (data.text("device_id") != target) {
                runtimeCurrent = false
                error = "运行状态的设备标识不匹配，请刷新列表后重试"
                return@run
            }
            runtime = data
            runtimeLoaded = true
            val sameSession = data.text("session_id") == sessionId
            if (!sameSession) {
                confirmReboot = false
                infoHistorical = infoLoaded
            }
            runtimeCurrent = online && data.optBoolean("online", false) && sameSession
            result = when {
                runtimeCurrent -> "运行状态快照已读取；不会自动刷新"
                !sameSession -> "控制会话已变化：此快照不代表当前状态，请刷新设备列表后重新读取"
                else -> "设备离线：仅展示历史上报，不代表当前状态"
            }
        }
    }

    /** 读一次系统信息原文（设备命令退出码非 0 视为失败）。 */
    fun readInfo(ctx: Pager) {
        if (!canSystem || !idle) return
        run("system/info", system = true) { data ->
            infoOutput = data.text("output")
            infoLoaded = true
            infoHistorical = false
            result = "系统信息已读取（设备命令退出码 0）"
        }
    }

    /** 只读资产现状（更新 tab 的 30 s 轮询用它；**不占**单操作闸门，失败静默保留旧值）。 */
    fun readOwwState(ctx: Pager) {
        if (owwLoading || target.isEmpty()) return
        owwLoading = true
        val deviceId = target
        val url = echoMuseAuthenticatedUrl(
            "$base/api/echomuse/devices/${echoMuseUrlComponent(deviceId)}/oww/state",
            token,
        )
        getJson(ctx, url) { ok, data, _ ->
            owwLoading = false
            if (ok && data.text("device_id") == deviceId) {
                owwState = data
                owwLoaded = true
            }
        }
    }

    /** 下发输出音量（0～127；202 只代表排队，不代表已生效）。 */
    fun setVolume(ctx: Pager) {
        if (!canVolume || !idle) return
        val level = volumeDraft.trim().toIntOrNull()
        if (level == null || level !in 0..127) {
            error = "音量必须是 0～127 的整数（0 为静音）"
            return
        }
        run("volume", system = false, body = JSONObject().apply { put("level", level) }) { data ->
            result = if (data.optBoolean("queued", false)) {
                "音量 $level 已排队下发，尚未确认生效；请点击“读取运行状态”核对实际上报"
            } else {
                "接口已响应，但未确认排队；请读取运行状态核对"
            }
        }
    }

    /** 请求重启（必须先点一次二次确认；`dispatched` 也不等于"已重启"）。 */
    fun reboot(ctx: Pager) {
        if (!canSystem || !idle || !confirmReboot) return
        run("system/reboot", system = true, body = JSONObject().apply { put("confirm", true) }) { data ->
            confirmReboot = false
            runtimeCurrent = false
            infoHistorical = infoLoaded
            result = if (data.optBoolean("dispatched", false)) {
                "重启请求已发送；尚未确认设备实际重启，请稍后核对在线状态"
            } else {
                "接口已响应，但未确认请求已发送；请核对设备状态"
            }
        }
    }

    /**
     * 重新安装 VAD 资产（`POST …/oww/install-vad`，**同步执行**：装完才回包，可能十几秒）。
     *
     * 这不是"升级固件"：只把服务端模型卷里那份 **typed 版** `silero_vad.onnx` 推到设备
     * `/data/local/share/echomuse/oww/`，设备下一轮开轮时 stat 到即生效（不需要重启进程）。
     * 缺它时设备的语音门退化成本底 RMS 门限，现场表现就是「按了键、说了话、没反应」。
     */
    fun reinstallVad(ctx: Pager, onDone: (ok: Boolean, msg: String) -> Unit) {
        if (!canSystem || !idle) {
            onDone(false, "设备离线或正忙：资产安装需要已批准的在线 emOS 设备")
            return
        }
        run(
            "oww/install-vad",
            system = true,
            body = JSONObject(),
            onFail = { onDone(false, error.ifEmpty { "安装失败：请稍后重试" }) },
        ) { data ->
            val bytes = (data.opt("bytes") as? Number)?.toLong() ?: 0L
            val digest = data.text("sha256")
            result = buildString {
                append("VAD 已写入设备（")
                append(bytes)
                append(" 字节")
                if (digest.isNotEmpty()) append("，sha256 ${digest.take(12)}…")
                append("）；设备下一轮开轮自动生效，无需重启")
            }
            onDone(true, result)
        }
    }

    /**
     * 一次性操作（`/system/…`、音量、日志、资产安装）的**唯一出口**：占闸门、映射 HTTP 错误、
     * 按代际作废在途回包。日志（[EchoMuseLogsState]）也走这里——同一台设备一把槽。
     *
     * 错误口径与原 `EchoMuseManagementState.request` 逐字一致（401/403/404/409/422/
     * 502/503/504 各有专门文案），只是把"历史标"改成回调交给调用方自己定。
     */
    internal fun run(
        operation: String,
        system: Boolean,
        body: JSONObject? = null,
        onFail: () -> Unit = {},
        onSuccess: (JSONObject) -> Unit,
    ) {
        val requestGeneration = ++generation
        val deviceId = target
        busy = operation
        error = ""
        result = ""
        val headers = echoMuseHeaders(token)
        val url = "$base/api/echomuse/devices/${echoMuseUrlComponent(deviceId)}/$operation"
        // 一次性操作沿用 40 s 超时（与设备端 shell 期限兼容）；GET 无请求体。
        network(scope).httpRequest(url, body != null, body ?: JSONObject(), headers, timeout = 40) {
                data, success, errorMsg, response ->
            if (generation != requestGeneration || target != deviceId) return@httpRequest
            busy = ""
            val code = response.statusCode
            val ok = success && (code == null || code in 200..299)
            val exitCode = (data.opt("exit_code") as? Number)?.toInt()
            if (!ok || (system && body == null && exitCode != 0)) {
                val reason = data.text("error").ifEmpty { errorMsg }.ifEmpty { "HTTP $code" }
                val detail = data.text("code").takeIf { it.isNotEmpty() }?.let { " [$it]" } ?: ""
                error = when (code) {
                    401 -> "管理鉴权失败：请确认管理令牌，并用 ?token=管理令牌 打开管理台；重新连接后刷新列表"
                    403 -> "设备尚未批准，操作被拒绝"
                    404 -> "设备或接口不存在，请刷新列表并确认服务端版本"
                    409 -> "设备离线或正在执行其他管理操作，请稍后重试"
                    422 -> "该设备不支持此操作（系统管理仅支持 emOS）"
                    503 -> "系统管理暂不可用：设备的控制连接不可用或已被替换，请唤醒设备后重试"
                    504 -> "设备命令超时：未确认执行结果，请核对设备状态后再重试"
                    502 -> "设备命令执行失败：$reason"
                    else -> "操作失败：$reason"
                } + detail
                if (operation == "runtime") runtimeCurrent = false
                onFail()
                return@httpRequest
            }
            onSuccess(data)
        }
    }

    // ============================================================
    // 展示文本
    // ============================================================

    /** 运行状态快照的人话版（缺测一律"未知"，不与 0 混）。 */
    val runtimeText: String
        get() {
            val data = runtime ?: return "尚未读取运行状态；未上报的字段显示为未知。"
            val timestamp = (data.opt("updated_at") as? Number)?.toLong()?.let { formatEpochSeconds(it) }.orEmpty()
            return buildList {
                add(if (runtimeCurrent && online) "运行状态：最近一次上报快照（非实时监测）" else "运行状态：历史快照，不代表当前状态")
                add("上报时间：${timestamp.ifEmpty { "未知" }}")
                add("聆听状态：${data.optJSONObject("listen_state")?.text("state")?.ifEmpty { "未知" } ?: "未知"}")
                data.optJSONObject("listen_state")?.text("reason")?.takeIf { it.isNotEmpty() }?.let { add("原因：$it") }
                add("音量（0～127）：${fieldText(data.optJSONObject("volume_state"), "level")}")
                val muted = data.optJSONObject("mute_state")?.opt("muted") as? Boolean
                add("静音：${when (muted) { true -> "是"; false -> "否"; null -> "未知" }}")
                // `link` 是 TLS/token 三态（§3.2），RSSI/频段在 stats 里，两者不是一回事
                val link = data.optJSONObject("link")
                if (link != null) {
                    val authed = link.opt("authenticated") as? Boolean
                    val needToken = link.opt("token_required") as? Boolean
                    add(
                        "链路：${when {
                            authed == true && needToken == true -> "明文 ws · token 已校验"
                            authed == true -> "明文 ws · 未设 token（局域网信任）"
                            authed == false -> "未认证"
                            else -> "未知"
                        }}",
                    )
                }
                val stats = data.optJSONObject("stats")
                if (stats == null || stats.length() == 0) add("设备统计：未知（尚未上报）")
                else {
                    add("设备统计（原始上报快照，缺测为未知）：")
                    val labels = listOf(
                        "cpuPct" to "CPU 使用率（%）", "cpuTempC" to "CPU 温度（℃）",
                        "maxTempC" to "最高温度（℃）", "memUsedMb" to "已用内存（MB）",
                        "memTotalMb" to "总内存（MB）", "storageUsedMb" to "已用存储（MB）",
                        "storageTotalMb" to "总存储（MB）", "wifiRssi" to "WiFi 信号（dBm）",
                        "linkSpeedMbps" to "链路速率（Mbps）", "wifiFreqMhz" to "WiFi 频率（MHz）",
                        "coresOnline" to "在线 CPU 核心", "coresTotal" to "总 CPU 核心",
                        "aecRef" to "AEC 参考信号",
                    )
                    labels.forEach { (key, label) -> add("$label: ${fieldText(stats, key)}") }
                    val keys = stats.keys()
                    while (keys.hasNext()) {
                        val key = keys.next()
                        if (labels.none { it.first == key }) add("$key: ${fieldText(stats, key)}")
                    }
                }
            }.joinToString("\n")
        }

    /** 诊断汇总（"复制诊断内容"用；日志段落由调用方追加 [EchoMuseLogsState.diagnosticSection]）。 */
    val diagnosticText: String
        get() = buildList {
            add("设备：$target")
            add("列表连接状态：${if (online) "在线" else "离线"}；仅 emOS 系统管理：$emos")
            add(runtimeText)
            val oww = owwState
            if (owwLoaded && oww != null) {
                add(
                    "OWW 资产：VAD=${oww.optJSONObject("vad")?.text("state") ?: "unknown"}" +
                        "；ORT=${oww.optJSONObject("ort")?.text("state") ?: "unknown"}" +
                        "；剩余 ${(oww.opt("free_mb") as? Number)?.toString() ?: "未知"} MB",
                )
            }
            if (infoLoaded) add("系统信息${if (infoHistorical) "（历史）" else "（手动快照）"}：\n$infoOutput")
            if (result.isNotEmpty()) add(result)
            if (error.isNotEmpty()) add(error)
        }.joinToString("\n\n")

    // ============================================================
    // 内部
    // ============================================================

    /** 绑定一台设备：换设备时清掉上一台的所有设备域快照（绝不串台）。 */
    private fun bind(row: EchoMuseDeviceRow) {
        if (target.isNotEmpty() && target != row.deviceId) {
            generation++
            resetDeviceState()
        }
        target = row.deviceId
        approved = row.approved
        online = row.online
        emos = row.baseOs == "emos"
        speaker = row.capabilities.contains("speaker")
        sessionId = row.sessionId
    }

    private fun resetDeviceState() {
        runtime = null
        runtimeLoaded = false
        runtimeCurrent = false
        infoOutput = ""
        infoLoaded = false
        infoHistorical = false
        owwState = null
        owwLoaded = false
        owwLoading = false
        busy = ""
        error = ""
        result = ""
        confirmReboot = false
        volumeDraft = ""
        stopOwwPolling()
    }

    /** 30 s 轮询资产状态（更新 tab 在屏时才有意义，离开即停）。 */
    private fun armOwwPolling(ctx: Pager) {
        stopOwwPolling()
        owwPollJob = ctx.lifecycleScope.launch {
            while (tab == EM_TAB_UPDATES) {
                delay(EM_OWW_POLL_MS.toInt())
                if (tab != EM_TAB_UPDATES) return@launch
                readOwwState(ctx)
            }
        }
    }

    private fun stopOwwPolling() {
        owwPollJob?.cancel()
        owwPollJob = null
    }

    private fun fieldText(data: JSONObject?, key: String): String =
        data?.opt(key)?.toString()?.takeUnless { it == "null" || it.isEmpty() } ?: "未知"
}

/** 更新 tab 的资产状态轮询间隔（PRD §5.5：30 s，只读、可随时离开）。 */
private const val EM_OWW_POLL_MS = 30_000

// ============================================================
// 六 tab 标签（§6.2 归位表；`detail.tab` 存的就是这些文本，见 §6.6 的 tabBar 示例）
// ============================================================

const val EM_TAB_STATUS = "状态"
const val EM_TAB_ACTIVITY = "活动"
const val EM_TAB_CONFIG = "配置"
const val EM_TAB_CONSOLE = "控制台"
const val EM_TAB_UPDATES = "更新"
const val EM_TAB_LOGS = "日志"

/** 六个 tab 的等宽标签序列（`tabBar(tabs = EM_DETAIL_TABS, …)`）。 */
val EM_DETAIL_TABS: List<String> = listOf(
    EM_TAB_STATUS, EM_TAB_ACTIVITY, EM_TAB_CONFIG, EM_TAB_CONSOLE, EM_TAB_UPDATES, EM_TAB_LOGS,
)
