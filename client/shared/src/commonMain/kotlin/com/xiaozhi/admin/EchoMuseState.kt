package com.xiaozhi.admin

import com.tencent.kuikly.core.base.PagerScope
import com.tencent.kuikly.core.nvi.serialization.json.JSONObject
import com.tencent.kuikly.core.pager.Pager
import com.tencent.kuikly.core.reactive.collection.ObservableList
import com.tencent.kuikly.core.reactive.handler.observable
import com.tencent.kuikly.core.reactive.handler.observableList
import com.tencent.kuikly.core.timer.setTimeout

/**
 * 「EchoMuse 设备」页的数据源（`GET /api/echomuse/devices` + 批准/下发指令/删除记录三个写动作）。
 *
 * 与小智协议设备（[DevicesState]，进程内注册表）不同：EchoMuse 设备由服务端 `[echomuse]`
 * 能力持久登记（批准制），走独立的 `/api/echomuse/` 端点。`enabled=false` 表示服务端
 * 未开启该能力，页面据此显示"未启用"提示而不是空列表。
 *
 * 列表是**请求时刻的快照**，但页面不需要轮询：服务端设备面有变化（上线/离线/
 * 批准/设置/删除）会经 `GET /api/echomuse/devices/wait`（事件保持：有变化才回包，
 * 无变化服务端挂 25s 超时）立即唤醒本页刷新——见 [ensureLiveUpdates]。
 *
 * 设计口径与 [DevicesState] 一致：
 * - 加载失败就地显示（`loadError` + 「刷新」），不弹 toast；
 * - 写动作（批准/重启/WiFi/删除）结果走顶部 toast（调用方回调）；
 * - 409（设备离线）单独给"设备离线"提示——指令不会下发，需要先把设备唤醒。
 *
 * ⚠️ 本类不渲染：渲染见 `EchoMusePage.kt`，页面壳体见 `AdminPage.kt`。
 */

/** 长挂失败后的重试间隔：服务端不可达时退避，不打爆网络。 */
private const val ECHOMUSE_WAIT_RETRY_MS = 5_000

/** 一台 EchoMuse 设备（`GET /api/echomuse/devices` 的一行）。字段不可变——刷新时整表重建。 */
class EchoMuseDeviceRow(
    val deviceId: String,
    val label: String,
    val approved: Boolean,
    val version: String,
    val board: String,
    val baseOs: String,
    /** 首次接入（unix 秒；服务端未给为 -1）。展示经 [formatEpochSeconds] 换成本地时间。 */
    val firstSeen: Long,
    /** 最近一次接入（unix 秒；服务端未给为 -1）。 */
    val lastSeen: Long,
    val online: Boolean,
    /** 已连接时长（秒；服务端未给为 null → -1）。 */
    val connectedSecs: Long,
    /** 服务端保存的唤醒词设置（可能为空对象；缺省键以页面默认值兜底）。 */
    val settings: JSONObject,
) {
    /** 标题：label 优先，未命名时回退设备号。 */
    val title: String get() = label.ifEmpty { deviceId }

    /** 是否改过名：改名后标题是 label，副标题行露出原始 device_id 供核对。 */
    val renamed: Boolean get() = label.isNotEmpty() && label != deviceId

    /** 副标题：版本 · 板型 · base_os。缺失项自动省略。 */
    val subtitle: String
        get() = listOfNotNull(
            version.takeIf { it.isNotEmpty() }?.let { "版本 $it" },
            board.takeIf { it.isNotEmpty() },
            baseOs.takeIf { it.isNotEmpty() }?.let { "系统 $it" },
        ).joinToString(" · ")

    /** 元信息行：最近在线 / 首次接入 / 已连接时长（时间戳一律本地格式化，不出原始 epoch）。 */
    val metaText: String
        get() = listOfNotNull(
            lastSeenText.takeIf { it.isNotEmpty() }?.let { "最近在线 $it" },
            firstSeenText.takeIf { it.isNotEmpty() }?.let { "首次接入 $it" },
            if (connectedSecs >= 0) "已连接 ${humanDuration(connectedSecs * 1000L)}" else null,
        ).joinToString(" · ")

    private val lastSeenText: String get() = formatEpochSeconds(lastSeen)
    private val firstSeenText: String get() = formatEpochSeconds(firstSeen)

    /** 「复制信息」的纯文本全量快照（设备号/版本/时间/状态，粘到聊天或工单可直接读）。 */
    val copyText: String
        get() = buildList {
            add("设备: $title")
            if (renamed) add("device_id: $deviceId")
            if (subtitle.isNotEmpty()) add(subtitle)
            if (metaText.isNotEmpty()) add(metaText)
            add("状态: ${if (online) "在线" else "离线"}${if (approved) "" else "（待批准）"}")
        }.joinToString("\n")
}

/** `GET /api/echomuse/devices` 的数据源 + 批准 / 下发指令（reboot、wifi）/ 删除记录。 */
class EchoMuseState(scope: PagerScope) : LoadableState(scope) {

    /** 服务端 `[echomuse]` 能力开关（false 时页面显示"未启用"提示）。 */
    var enabled by scope.observable(true)

    /**
     * 服务端设备面变化版本号（`devices` 回包携带）：`/wait` 长挂的游标。
     * -1 = 尚未拿到（服务端过旧没有该字段——此时不挂通知链，退回进页刷新一次）。
     */
    private var serverVersion = -1L

    val devices: ObservableList<EchoMuseDeviceRow> by scope.observableList()

    override fun endpoint(baseUrl: String): String = "$baseUrl/api/echomuse/devices"

    override val errorLabel: String = "EchoMuse 设备列表加载失败"

    override fun apply(data: JSONObject) {
        enabled = data.optBoolean("enabled", true)
        // 版本游标只在服务端给了才更新（老服务端没有该字段，保持 -1）。
        data.optLong("version")?.let { serverVersion = it }
        devices.clear()
        val arr = data.array("devices")
        if (arr != null) {
            for (i in 0 until arr.length()) {
                val d = arr.optJSONObject(i) ?: continue
                devices.add(
                    EchoMuseDeviceRow(
                        deviceId = d.text("device_id"),
                        label = d.text("label"),
                        approved = d.optBoolean("approved", false),
                        version = d.text("version"),
                        board = d.text("board"),
                        baseOs = d.text("base_os"),
                        firstSeen = d.optLong("first_seen") ?: -1L,
                        lastSeen = d.optLong("last_seen") ?: -1L,
                        online = d.optBoolean("online", false),
                        connectedSecs = d.optLong("connected_secs") ?: -1L,
                        settings = d.optJSONObject("settings") ?: JSONObject(),
                    ),
                )
            }
        }
        loaded = true
        loadError = ""
    }

    /** 列表摘要（头部行）：未启用 / 共 N 台 · 在线 M。 */
    fun summaryText(): String =
        if (!enabled) {
            "EchoMuse 未启用"
        } else {
            val online = devices.count { it.online }
            "共 ${devices.size} 台 · $online 台在线"
        }

    // ============================================================
    // 批准（POST /api/echomuse/devices/{id}/approve）
    // ============================================================

    /** 当前展开批准表单的设备号（空串 = 全部收起）。 */
    var approveTarget by scope.observable("")

    /** 批准表单的 label 草稿。 */
    var approveDraft by scope.observable("")

    /**
     * 批准一台设备。成功后回调 `onDone(true, msg)` 并由调用方刷新列表（approved 状态以
     * 服务端为准）；404 = 设备不存在（列表过期，刷新即可）。
     */
    fun approve(ctx: Pager, deviceId: String, onDone: (ok: Boolean, msg: String) -> Unit) {
        if (actionBusy) return
        actionBusy = true
        val body = JSONObject().apply { put("label", approveDraft.trim()) }
        post(ctx, "/api/echomuse/devices/$deviceId/approve", body) { ok, code, reason, _ ->
            actionBusy = false
            if (ok) {
                approveTarget = ""
                approveDraft = ""
                onDone(true, "已批准 ${displayName(deviceId)}")
            } else {
                onDone(false, when (code) {
                    404 -> "设备不存在（列表可能已过期，请刷新后重试）"
                    else -> "批准失败：${reason.ifEmpty { "HTTP $code" }}"
                })
            }
        }
    }

    // ============================================================
    // 下发指令（POST /api/echomuse/devices/{id}/command）
    // ============================================================

    /** 重启行内二次确认的设备号（空串 = 无；同 OtaState 的 confirmKey 范式）。 */
    var confirmRebootId by scope.observable("")

    /** 当前展开 WiFi 配置表单的设备号（空串 = 全部收起）。 */
    var wifiTarget by scope.observable("")

    /** WiFi 表单草稿。 */
    var wifiSsid by scope.observable("")
    var wifiPsk by scope.observable("")

    /** 写动作互斥锁（批准/重启/WiFi 共用：单设备低频管理操作，无需并发）。 */
    var actionBusy by scope.observable(false)

    /**
     * 下发一条指令。202 = 已排队（结果看设备灯环）；409 = 设备离线，指令不会下发。
     */
    fun command(
        ctx: Pager,
        deviceId: String,
        body: JSONObject,
        onDone: (ok: Boolean, msg: String) -> Unit,
    ) {
        if (actionBusy) return
        actionBusy = true
        post(ctx, "/api/echomuse/devices/$deviceId/command", body) { ok, code, reason, _ ->
            actionBusy = false
            if (ok) {
                confirmRebootId = ""
                onDone(true, "指令已下发，结果看设备灯环")
            } else {
                onDone(false, when (code) {
                    409 -> "设备离线，指令未下发：请先唤醒设备再试"
                    404 -> "设备不存在（列表可能已过期，请刷新后重试）"
                    else -> "指令下发失败：${reason.ifEmpty { "HTTP $code" }}"
                })
            }
        }
    }

    /** 重启（确认后调用）。 */
    fun reboot(ctx: Pager, deviceId: String, onDone: (ok: Boolean, msg: String) -> Unit) {
        command(ctx, deviceId, JSONObject().apply { put("action", "reboot") }, onDone)
    }

    /** 下发 WiFi 配置（ssid + psk）。 */
    fun sendWifi(ctx: Pager, deviceId: String, onDone: (ok: Boolean, msg: String) -> Unit) {
        val ssid = wifiSsid.trim()
        if (ssid.isEmpty()) {
            onDone(false, "请先填写 WiFi 名称（SSID）")
            return
        }
        command(ctx, deviceId, JSONObject().apply {
            put("action", "wifi")
            put("ssid", ssid)
            put("psk", wifiPsk)
        }) { ok, msg ->
            if (ok) {
                wifiTarget = ""
                wifiSsid = ""
                wifiPsk = ""
            }
            onDone(ok, msg)
        }
    }

    // ============================================================
    // 删除记录（POST /api/echomuse/devices/{id}/delete）
    // ============================================================

    /** 删除行内二次确认的设备号（空串 = 无；同 [confirmRebootId] 范式）。 */
    var confirmDeleteId by scope.observable("")

    /**
     * 删除登记记录（清理 mock/联调设备）。只清静态登记、不断开在线会话；
     * 设备再次 register 会重新出现在列表里。
     */
    fun delete(ctx: Pager, deviceId: String, onDone: (ok: Boolean, msg: String) -> Unit) {
        if (actionBusy) return
        actionBusy = true
        post(ctx, "/api/echomuse/devices/$deviceId/delete", JSONObject()) { ok, code, reason, _ ->
            actionBusy = false
            confirmDeleteId = ""
            if (ok) {
                onDone(true, "已删除记录 ${displayName(deviceId)}")
            } else {
                onDone(false, when (code) {
                    404 -> "设备不存在（可能已被删除，刷新列表即可）"
                    else -> "删除失败：${reason.ifEmpty { "HTTP $code" }}"
                })
            }
        }
    }

    /** 展开/收起某台设备的批准表单（切换目标时清空草稿）。 */
    fun toggleApprove(deviceId: String) {
        if (approveTarget == deviceId) {
            approveTarget = ""
        } else {
            approveTarget = deviceId
            approveDraft = ""
        }
    }

    /** 展开/收起某台设备的 WiFi 表单（切换目标时清空草稿）。 */
    fun toggleWifi(deviceId: String) {
        if (wifiTarget == deviceId) {
            wifiTarget = ""
        } else {
            wifiTarget = deviceId
            wifiSsid = ""
            wifiPsk = ""
        }
    }

    // ============================================================
    // 唤醒词设置（POST /api/echomuse/devices/{id}/settings，部分更新）
    // ============================================================

    /** 当前展开唤醒词设置的设备号（空串 = 全部收起）。 */
    var settingsTarget by scope.observable("")

    // —— 表单草稿：展开时从设备 settings 快照填充，缺省走默认值 ——
    var wakeWordEnabled by scope.observable(true)
    var owwOnDevice by scope.observable(true)
    var owwModel by scope.observable("")
    /** 唤醒灵敏度阈值 0~1（越大越不易误触）。 */
    var owwThreshold by scope.observable(0.5)
    var wakeSound by scope.observable(true)
    /** 提示音音量（数字输入草稿，保存时解析校验）。 */
    var wakeSoundLevel by scope.observable("0.5")
    /** 0=中心麦克风，范围 0~6。 */
    var wakeMic by scope.observable("0")

    /** 展开/收起某台设备的唤醒词设置（切换目标时按该设备 settings 重填草稿）。 */
    fun toggleSettings(deviceId: String) {
        if (settingsTarget == deviceId) {
            settingsTarget = ""
            return
        }
        settingsTarget = deviceId
        val s = devices.firstOrNull { it.deviceId == deviceId }?.settings ?: JSONObject()
        wakeWordEnabled = s.optBoolean("wakeWordEnabled", true)
        owwOnDevice = s.optBoolean("owwOnDevice", true)
        owwModel = s.optString("owwModel", "")
        owwThreshold = s.optDouble("owwThreshold", 0.5).coerceIn(0.0, 1.0)
        wakeSound = s.optBoolean("wakeSound", true)
        wakeSoundLevel = doubleText(s.opt("wakeSoundLevel"), "0.5")
        wakeMic = intText(s.opt("wakeMic"), "0")
    }

    /**
     * 保存唤醒词设置。202（设备在线）= 已排队下发；200 = 已保存、设备下次接入时自动推送。
     * 数值字段解析失败/越界就不发请求（不发半截更新）。
     */
    fun saveSettings(ctx: Pager, deviceId: String, onDone: (ok: Boolean, msg: String) -> Unit) {
        val level = wakeSoundLevel.trim().toDoubleOrNull()
        if (level == null) {
            onDone(false, "提示音音量需要是数字（当前：${wakeSoundLevel}）")
            return
        }
        val mic = wakeMic.trim().toIntOrNull()
        if (mic == null || mic !in 0..6) {
            onDone(false, "麦克风编号需要是 0~6 的整数（0=中心麦克风）")
            return
        }
        if (actionBusy) return
        actionBusy = true
        val body = JSONObject().apply {
            put("wakeWordEnabled", wakeWordEnabled)
            put("owwOnDevice", owwOnDevice)
            put("owwModel", owwModel.trim())
            put("owwThreshold", owwThreshold)
            put("wakeSound", wakeSound)
            put("wakeSoundLevel", level)
            put("wakeMic", mic)
        }
        post(ctx, "/api/echomuse/devices/$deviceId/settings", body) { ok, code, reason, data ->
            actionBusy = false
            if (ok) {
                val delivered = data?.optBoolean("delivered", false) == true ||
                    data?.optBoolean("queued", false) == true
                val msg = if (delivered) "设置已即时下发到设备" else "设置已保存，设备下次接入时自动生效"
                onDone(true, msg)
            } else {
                onDone(false, when (code) {
                    400 -> "设置未保存：${reason.ifEmpty { "字段不合法" }}"
                    404 -> "设备不存在（列表可能已过期，请刷新后重试）"
                    else -> "设置保存失败：${reason.ifEmpty { "HTTP $code" }}"
                })
            }
        }
    }

    // ============================================================
    // 实时更新：服务端事件保持（GET /devices/wait 长挂），变化才回包
    // ============================================================

    /** 长挂链只起一次（[ensureLiveUpdates] 会在每次重组时被调用）。 */
    private var liveStarted = false

    /**
     * 页面停留（[isActive]）期间挂住 `GET /api/echomuse/devices/wait?after=<版本>`：
     * 服务端设备面有变化（设备上线/离线、批准、设置、删除）立刻回包，本页随即
     * 刷新列表再挂下一轮；无变化由服务端 25s 超时回 `changed:false`。**没有固定
     * 轮询间隔**——无事件时不产生任何请求流量。链在页面切走后停摆，回到本页时
     * 渲染层再调 [ensureLiveUpdates] 重新起链（挂起期间发生的变化会在第一个回包
     * 立即对齐）。服务端过旧（无版本字段）则不起链，退回"进页刷新一次"的口径。
     */
    fun ensureLiveUpdates(ctx: Pager, isActive: () -> Boolean) {
        if (liveStarted || serverVersion < 0) return
        liveStarted = true
        waitTick(ctx, isActive)
    }

    private fun waitTick(ctx: Pager, isActive: () -> Boolean) {
        getJson(ctx, "$base/api/echomuse/devices/wait?after=$serverVersion") { ok, data, _ ->
            if (!isActive()) {
                liveStarted = false
                return@getJson
            }
            if (!ok) {
                // 网络/服务端暂不可达：退避后重挂，链不断。
                ctx.setTimeout(ECHOMUSE_WAIT_RETRY_MS) {
                    if (isActive()) waitTick(ctx, isActive) else liveStarted = false
                }
                return@getJson
            }
            // 版本以回包为准（refresh 的列表回包还会再对齐一次，值相同）。
            data.optLong("version")?.let { serverVersion = it }
            if (data.optBoolean("changed", false)) refresh(ctx)
            waitTick(ctx, isActive)
        }
    }

    /** 服务端数值转输入框草稿：有值原样展示，缺省用 [fallback]。 */
    private fun doubleText(raw: Any?, fallback: String): String =
        (raw as? Number)?.let { formatNum(it.toDouble()) } ?: fallback

    private fun intText(raw: Any?, fallback: String): String =
        (raw as? Number)?.let { it.toInt().toString() } ?: fallback

    /** 整数去掉小数尾零（1.0 → "1"，0.5 → "0.5"）。 */
    private fun formatNum(v: Double): String {
        val r = (v * 100).let { kotlin.math.round(it) / 100 }
        return if (r == r.toLong().toDouble()) r.toLong().toString() else r.toString()
    }

    /** 展示名：列表里找得到用 label，否则设备号（动作结果提示用）。 */
    private fun displayName(deviceId: String): String =
        devices.firstOrNull { it.deviceId == deviceId }?.let { it.title } ?: deviceId

    /** 全包共用的 JSON POST：成功回调 `onDone(true, code, "")`，失败回调 `onDone(false, code, 原因)`。`data` 为响应体。 */
    private fun post(
        ctx: Pager,
        path: String,
        body: JSONObject,
        onDone: (ok: Boolean, code: Int?, reason: String, data: JSONObject?) -> Unit,
    ) {
        val headers = JSONObject().apply { put("Content-Type", "application/json") }
        network(ctx).httpRequest("$base$path", true, body, headers) { data, success, errorMsg, resp ->
            val code = resp.statusCode
            val ok = success && (code == null || code in 200..299)
            val reason = data?.text("error")?.takeIf { it.isNotEmpty() }
                ?: data?.text("message")?.takeIf { it.isNotEmpty() }
                ?: (errorMsg ?: "")
            onDone(ok, code, reason, data)
        }
    }
}
