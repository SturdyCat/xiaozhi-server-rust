package com.xiaozhi.admin

import com.tencent.kuikly.core.base.PagerScope
import com.tencent.kuikly.core.nvi.serialization.json.JSONObject
import com.tencent.kuikly.core.pager.Pager
import com.tencent.kuikly.core.reactive.handler.observable
import com.tencent.kuikly.core.reactive.handler.observableList

/**
 * emOS 管理子集；API 合同由服务端约束，不接收 shell 命令、路径或 OTA 输入。
 * 一次只展开一台设备，只在用户点击时请求。切设备/连接状态变化使在途回包失效，
 * 避免把旧设备的输出、离线前的 runtime 误展示成当前状态。
 */
class EchoMuseManagementState(scope: PagerScope) {
    var target by scope.observable("")
        private set
    var approved by scope.observable(false)
        private set
    var online by scope.observable(false)
        private set
    var emos by scope.observable(false)
        private set
    var speaker by scope.observable(false)
        private set
    var busy by scope.observable("")
        private set
    var error by scope.observable("")
        private set
    var result by scope.observable("")
        private set
    var confirmReboot by scope.observable(false)
    var logSource by scope.observable("messages")
    var sourceOpen by scope.observable(false)
    val logSources by scope.observableList<Pair<String, String>>()

    init {
        logSources.addAll(listOf(
            "messages" to "当前系统日志（messages）",
            "messages_last" to "上次系统日志（messages.last）",
            "last_kmsg_prev" to "上次内核日志（last_kmsg.prev）",
            "supervisor" to "启动监督日志（supervisor.log）",
        ))
    }
    var infoOutput by scope.observable("")
        private set
    var infoLoaded by scope.observable(false)
        private set
    var logsOutput by scope.observable("")
        private set
    var logsLoaded by scope.observable(false)
        private set
    var loadedLogSource by scope.observable("")
        private set
    var runtime by scope.observable<JSONObject?>(null)
        private set
    var runtimeLoaded by scope.observable(false)
        private set
    var runtimeCurrent by scope.observable(false)
        private set
    var infoHistorical by scope.observable(false)
        private set
    var logsHistorical by scope.observable(false)
        private set
    var volumeDraft by scope.observable("")

    private var generation = 0
    private var sessionId = ""
    private var base = ""
    private var token = ""

    fun configureConnection(baseUrl: String, authToken: String) {
        if (base != baseUrl || token != authToken) {
            generation++
            target = ""
            sessionId = ""
            busy = ""
            error = ""
            result = ""
            runtime = null
            runtimeLoaded = false
            runtimeCurrent = false
            infoLoaded = false
            logsLoaded = false
            infoOutput = ""
            logsOutput = ""
            loadedLogSource = ""
            infoHistorical = false
            logsHistorical = false
            volumeDraft = ""
            sourceOpen = false
            confirmReboot = false
        }
        base = baseUrl
        token = authToken
    }

    val canSystem: Boolean get() = approved && emos && online
    val canVolume: Boolean get() = approved && emos && online && speaker
    val idle: Boolean get() = busy.isEmpty()

    /**
     * 菜单选中语义（区别于 [toggle] 的翻转）：target 已是本设备则保留诊断输出不重置，
     * 否则走 toggle 重置并登记目标。由 `EchoMuseState.selectMenu` 驱动。
     */
    fun select(row: EchoMuseDeviceRow, baseUrl: String) {
        if (target != row.deviceId) toggle(row, baseUrl)
    }

    fun toggle(row: EchoMuseDeviceRow, baseUrl: String) {
        generation++
        target = if (target == row.deviceId) "" else row.deviceId
        base = baseUrl
        busy = ""
        error = ""
        result = ""
        confirmReboot = false
        sourceOpen = false
        logSource = "messages"
        infoLoaded = false
        logsLoaded = false
        infoOutput = ""
        logsOutput = ""
        loadedLogSource = ""
        runtime = null
        runtimeLoaded = false
        runtimeCurrent = false
        infoHistorical = false
        logsHistorical = false
        volumeDraft = ""
        sync(listOf(row))
    }

    /** 列表长挂仍可更新在线态；管理数据不随列表自动抓取。 */
    fun sync(rows: List<EchoMuseDeviceRow>) {
        if (target.isEmpty()) return
        val row = rows.firstOrNull { it.deviceId == target }
        val nextApproved = row?.approved == true
        val nextOnline = row?.online == true
        val nextEmos = row?.baseOs == "emos"
        val nextSpeaker = row?.capabilities?.contains("speaker") == true
        val nextSessionId = row?.sessionId.orEmpty()
        if (approved != nextApproved || online != nextOnline || emos != nextEmos ||
            speaker != nextSpeaker || sessionId != nextSessionId) {
            generation++
            busy = ""
            runtimeCurrent = false
            infoHistorical = infoLoaded
            logsHistorical = logsLoaded
            confirmReboot = false
            result = ""
        }
        approved = nextApproved
        online = nextOnline
        emos = nextEmos
        speaker = nextSpeaker
        sessionId = nextSessionId
        if (row == null) {
            target = ""
            sourceOpen = false
        }
    }

    fun readRuntime(ctx: Pager) {
        if (!approved || !idle) return
        request(ctx, "runtime", false) { data ->
            if (data.text("device_id") != target) {
                runtimeCurrent = false
                error = "运行状态的设备标识不匹配，请刷新列表后重试"
                return@request
            }
            runtime = data
            runtimeLoaded = true
            val sameSession = data.text("session_id") == sessionId
            if (!sameSession) {
                confirmReboot = false
                infoHistorical = infoLoaded
                logsHistorical = logsLoaded
            }
            runtimeCurrent = online && data.optBoolean("online", false) && sameSession
            result = when {
                runtimeCurrent -> "运行状态快照已读取；不会自动刷新"
                !sameSession -> "控制会话已变化：此快照不代表当前状态，请刷新设备列表后重新读取"
                else -> "设备离线：仅展示历史上报，不代表当前状态"
            }
        }
    }

    fun readInfo(ctx: Pager) {
        if (!canSystem || !idle) return
        request(ctx, "system/info", true) { data ->
            infoOutput = data.text("output")
            infoLoaded = true
            infoHistorical = false
            result = "系统信息已读取（设备命令退出码 0）"
        }
    }

    fun readLogs(ctx: Pager) {
        if (!canSystem || !idle || logSource !in logSources.map { it.first }) return
        val source = logSource
        request(ctx, "system/logs?source=$source", true) { data ->
            logsOutput = data.text("output")
            loadedLogSource = data.text("source").ifEmpty { source }
            logsLoaded = true
            logsHistorical = false
            result = "日志已读取（设备命令退出码 0）"
        }
    }

    fun reboot(ctx: Pager) {
        if (!canSystem || !idle || !confirmReboot) return
        request(ctx, "system/reboot", true, JSONObject().apply { put("confirm", true) }) { data ->
            confirmReboot = false
            runtimeCurrent = false
            infoHistorical = infoLoaded
            logsHistorical = logsLoaded
            result = if (data.optBoolean("dispatched", false)) {
                "重启请求已发送；尚未确认设备实际重启，请稍后核对在线状态"
            } else {
                "接口已响应，但未确认请求已发送；请核对设备状态"
            }
        }
    }

    fun setVolume(ctx: Pager) {
        if (!canVolume || !idle) return
        val level = volumeDraft.trim().toIntOrNull()
        if (level == null || level !in 0..127) {
            error = "音量必须是 0～127 的整数（0 为静音）"
            return
        }
        request(ctx, "volume", false, JSONObject().apply { put("level", level) }) { data ->
            result = if (data.optBoolean("queued", false)) {
                "音量 $level 已排队下发，尚未确认生效；请点击“读取运行状态”核对实际上报"
            } else {
                "接口已响应，但未确认排队；请读取运行状态核对"
            }
        }
    }

    /** timeout 与服务端 shell 超时兼容；只发送固定路径和受限数据，不暴露任意命令。 */
    private fun request(
        ctx: Pager,
        operation: String,
        system: Boolean,
        body: JSONObject? = null,
        onSuccess: (JSONObject) -> Unit,
    ) {
        val requestGeneration = ++generation
        val deviceId = target
        busy = operation
        error = ""
        result = ""
        val headers = echoMuseHeaders(token)
        network(ctx).httpRequest(
            "$base/api/echomuse/devices/${echoMuseUrlComponent(deviceId)}/$operation",
            body != null, body ?: JSONObject(), headers, timeout = 40,
        ) { data, success, errorMsg, response ->
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
                    503 -> "系统管理不可用：请配置非空 server.expected_token（管理令牌）和 echomuse.token（设备令牌），并用 ?token=管理令牌 打开管理台"
                    504 -> "设备命令超时：未确认执行结果，请核对设备状态后再重试"
                    502 -> "设备命令执行失败：$reason"
                    else -> "操作失败：$reason"
                } + detail
                if (operation == "runtime") runtimeCurrent = false
                if (system) {
                    infoHistorical = infoLoaded
                    logsHistorical = logsLoaded
                }
                return@httpRequest
            }
            onSuccess(data)
        }
    }

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

    val diagnosticText: String
        get() = buildList {
            add("设备：$target")
            add("列表连接状态：${if (online) "在线" else "离线"}；仅 emOS 系统管理：$emos")
            add(runtimeText)
            if (infoLoaded) add("系统信息${if (infoHistorical) "（历史）" else "（手动快照）"}：\n$infoOutput")
            if (logsLoaded) add("日志 $loadedLogSource${if (logsHistorical) "（历史）" else "（手动快照）"}：\n$logsOutput")
            if (result.isNotEmpty()) add(result)
            if (error.isNotEmpty()) add(error)
        }.joinToString("\n\n")

    private fun fieldText(data: JSONObject?, key: String): String =
        data?.opt(key)?.toString()?.takeUnless { it == "null" || it.isEmpty() } ?: "未知"
}
