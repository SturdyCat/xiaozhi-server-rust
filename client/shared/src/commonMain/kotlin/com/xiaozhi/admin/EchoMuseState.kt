package com.xiaozhi.admin

import com.tencent.kuikly.core.base.PagerScope
import com.tencent.kuikly.core.nvi.serialization.json.JSONObject
import com.tencent.kuikly.core.pager.Pager
import com.tencent.kuikly.core.reactive.collection.ObservableList
import com.tencent.kuikly.core.reactive.handler.observable
import com.tencent.kuikly.core.reactive.handler.observableList

/**
 * 「EchoMuse 设备」页的数据源（`GET /api/echomuse/devices` + 批准/下发指令/删除记录三个写动作）。
 *
 * 与小智协议设备（[DevicesState]，进程内注册表）不同：EchoMuse 设备由服务端 `[echomuse]`
 * 能力持久登记（批准制），走独立的 `/api/echomuse/` 端点。`enabled=false` 表示服务端
 * 未开启该能力，页面据此显示"未启用"提示而不是空列表。
 *
 * 列表是**请求时刻的快照**，但页面不需要轮询：服务端设备面有变化（上线/离线/
 * 批准/设置/删除）会经管理台 WS（`GET /api/admin/ws`，各平台自实现 WebSocket 接入
 * 共享层 [AdminRealtime]）推送 `echomuse_devices_changed` 事件，本页随即刷新——
 * 见 [ensureLiveUpdates]。
 *
 * 设计口径与 [DevicesState] 一致：
 * - 加载失败就地显示（`loadError` + 「刷新」），不弹 toast；
 * - 写动作（批准/重启/WiFi/删除）结果走顶部 toast（调用方回调）；
 * - 409（设备离线）单独给"设备离线"提示——指令不会下发，需要先把设备唤醒。
 *
 * ⚠️ 本类不渲染：渲染见 `EchoMusePage.kt`，页面壳体见 `AdminPage.kt`。
 */

/**
 * EchoMuse 卡片内菜单项 id（[EchoMuseState.menuItem] 的取值域，渲染见 `EchoMuseDeviceCard.kt`）。
 * 未批准设备只有 概览/批准 两项；已批准为 概览/WiFi 配置/唤醒词设置/设备管理/蓝牙代理。
 */
const val ECHOMUSE_MENU_OVERVIEW = "overview"
const val ECHOMUSE_MENU_APPROVE = "approve"
const val ECHOMUSE_MENU_WIFI = "wifi"
const val ECHOMUSE_MENU_WAKE = "wake"
const val ECHOMUSE_MENU_MANAGE = "manage"
const val ECHOMUSE_MENU_BLE = "ble"

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
    val capabilities: List<String> = emptyList(),
    /** 当前 control 会话标识，不持久化；旧服务端缺省为空。 */
    val sessionId: String = "",
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

/** 设备列表/审批/WiFi/唤醒设置；emOS 系统管理见 [EchoMuseManagementState]。 */
class EchoMuseState(scope: PagerScope) {
    var loading by scope.observable(false)
        private set
    var loaded by scope.observable(false)
        private set
    var loadError by scope.observable("")
        private set
    private var base = ""
    private var connectionGeneration = 0

    /** 服务端 `[echomuse]` 能力开关（false 时页面显示"未启用"提示）。 */
    var enabled by scope.observable(true)

    val devices: ObservableList<EchoMuseDeviceRow> by scope.observableList()
    val management = EchoMuseManagementState(scope)
    val ble = EchoMuseBleState(scope)
    private var accessToken = ""

    /** 连接参数是一个整体；同 token 跨服务器也必须使旧请求失效。 */
    fun configureAccess(token: String, baseUrl: String = "") {
        if (accessToken != token || base != baseUrl) {
            connectionGeneration++
            loading = false
            loaded = false
            loadError = ""
            devices.clear()
            menuDevice = ""
            menuItem = ECHOMUSE_MENU_OVERVIEW
            actionBusy = false
            confirmDeleteId = ""
            approveDraft = ""
            wifiSsid = ""
            wifiPsk = ""
            // 实时订阅断开：base/token 都变了，旧连接（旧凭据）立即退订，
            // 下次渲染经 ensureLiveUpdates 以新参数重订（退订即退共享组引用）。
            liveSub?.close()
            liveSub = null
            liveSubBase = null
            liveActive = null
        }
        accessToken = token
        base = baseUrl
        management.configureConnection(baseUrl, token)
        ble.configureConnection(baseUrl, token)
    }

    fun load(ctx: Pager, baseUrl: String = "") {
        if (base != baseUrl) configureAccess(accessToken, baseUrl)
        if (loading) return
        val generation = connectionGeneration
        loading = true
        loadError = ""
        getJson(ctx, echoMuseAuthenticatedUrl("$base/api/echomuse/devices", accessToken)) { ok, data, reason ->
            if (generation != connectionGeneration) return@getJson
            loading = false
            if (ok) {
                apply(data)
                // 顺带拉一次服务端 oww-models/ 的自定义模型清单（失败静默，见 modelFilesRefresh）
                modelFilesRefresh(ctx)
            } else loadError = "EchoMuse 设备列表加载失败：$reason"
        }
    }

    fun refresh(ctx: Pager) = load(ctx, base)

    private fun apply(data: JSONObject) {
        enabled = data.optBoolean("enabled", true)
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
                        firstSeen = (d.opt("first_seen") as? Number)?.toLong() ?: -1L,
                        lastSeen = (d.opt("last_seen") as? Number)?.toLong() ?: -1L,
                        online = d.optBoolean("online", false),
                        connectedSecs = (d.opt("connected_secs") as? Number)?.toLong() ?: -1L,
                        settings = d.optJSONObject("settings") ?: JSONObject(),
                        sessionId = d.text("session_id"),
                        capabilities = d.array("capabilities")?.let { values ->
                            (0 until values.length()).mapNotNull { values.optString(it)?.takeIf { value -> value.isNotEmpty() } }
                        } ?: emptyList(),
                    ),
                )
            }
        }
        management.sync(devices)
        ble.sync(devices)
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
    // 服务端自定义唤醒词模型（oww-models/，经「文件」页上传）
    // ============================================================

    /**
     * 服务端 `oww-models/` 目录里已上传的自定义唤醒词模型（`.onnx` 文件名）。
     * 获取失败静默保持旧值/空——文件接口不可用（旧服务端/未部署）不应让设备页报错。
     */
    var uploadedOwwModels: List<String> = emptyList()
        private set

    /**
     * 拉取 `GET /api/files?path=oww-models` 并收集 `.onnx` 文件名（唤醒词下拉选项用，
     * 见 [EchoMuseWakeWord.buildOwwOptions]）。**刻意安静**：失败只保持现状，
     * 不置 loadError、不弹 toast——模型清单是增强信息，不是设备管理的依赖。
     */
    fun modelFilesRefresh(ctx: Pager, baseUrl: String = base) {
        getJson(ctx, echoMuseAuthenticatedUrl("$baseUrl/api/files?path=${echoMuseUrlComponent(OWW_MODELS_DIR)}", accessToken)) { ok, data, _ ->
            if (!ok) return@getJson
            val arr = data.array("entries") ?: return@getJson
            uploadedOwwModels = (0 until arr.length()).mapNotNull { i ->
                val e = arr.optJSONObject(i) ?: return@mapNotNull null
                val name = e.text("name")
                if (name.isNotEmpty() && !e.optBoolean("dir", false) && FilesModel.isModelFile(name)) name else null
            }
        }
    }

    // ============================================================
    // 卡片内菜单（全局单选：左列菜单 + 右侧内容面板，渲染见 EchoMuseDeviceCard.kt）
    // ============================================================

    /** 当前选中菜单的设备号（空串 = 无；不匹配当前行的卡片一律显示概览面板）。 */
    var menuDevice by scope.observable("")

    /** 当前选中的菜单项 id（全局单选：切换卡片或菜单项时更新）。 */
    var menuItem by scope.observable(ECHOMUSE_MENU_OVERVIEW)

    /**
     * 选中某台设备的某个菜单项。切换目标时按需重置对应表单草稿（保持原 toggle*
     * 的清空语义）：批准清 label 草稿、WiFi 清 ssid/psk、唤醒词按设备 settings 重填；
     * 管理/蓝牙代理把对应子状态的 target 定到本设备（已选中则保留诊断输出不重置）。
     */
    fun selectMenu(row: EchoMuseDeviceRow, item: String) {
        if (menuDevice == row.deviceId && menuItem == item) return
        menuDevice = row.deviceId
        menuItem = item
        when (item) {
            ECHOMUSE_MENU_APPROVE -> approveDraft = ""
            ECHOMUSE_MENU_WIFI -> {
                wifiSsid = ""
                wifiPsk = ""
            }
            ECHOMUSE_MENU_WAKE -> fillSettingsDraft(row.deviceId)
            ECHOMUSE_MENU_MANAGE -> management.select(row, base)
            ECHOMUSE_MENU_BLE -> ble.select(row)
        }
    }

    /**
     * 当前卡片生效的菜单项：[menuDevice] 不匹配本卡、或菜单项不适用当前批准态
     * （已批准没有「批准」、未批准没有 WiFi/唤醒词/管理/蓝牙）一律回落「概览」。
     */
    fun activeMenuItem(row: EchoMuseDeviceRow): String {
        if (menuDevice != row.deviceId) return ECHOMUSE_MENU_OVERVIEW
        val approvedOnly = menuItem == ECHOMUSE_MENU_WIFI ||
            menuItem == ECHOMUSE_MENU_WAKE ||
            menuItem == ECHOMUSE_MENU_MANAGE ||
            menuItem == ECHOMUSE_MENU_BLE
        return when {
            approvedOnly && !row.approved -> ECHOMUSE_MENU_OVERVIEW
            menuItem == ECHOMUSE_MENU_APPROVE && row.approved -> ECHOMUSE_MENU_OVERVIEW
            else -> menuItem
        }
    }

    // ============================================================
    // 批准（POST /api/echomuse/devices/{id}/approve）
    // ============================================================

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
        post(ctx, "/api/echomuse/devices/${echoMuseUrlComponent(deviceId)}/approve", body) { ok, code, reason, _ ->
            actionBusy = false
            if (ok) {
                approveDraft = ""
                // 批准后面板回落概览（刷新后该卡的菜单项已切为已批准集合，「批准」项消失）
                menuItem = ECHOMUSE_MENU_OVERVIEW
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
        post(ctx, "/api/echomuse/devices/${echoMuseUrlComponent(deviceId)}/command", body) { ok, code, reason, _ ->
            actionBusy = false
            if (ok) {
                onDone(true, "WiFi 指令已排队下发，执行结果请在设备上核对")
            } else {
                onDone(false, when (code) {
                    409 -> "设备离线，指令未下发：请先唤醒设备再试"
                    404 -> "设备不存在（列表可能已过期，请刷新后重试）"
                    else -> "指令下发失败：${reason.ifEmpty { "HTTP $code" }}"
                })
            }
        }
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
                wifiSsid = ""
                wifiPsk = ""
            }
            onDone(ok, msg)
        }
    }

    // ============================================================
    // 删除记录（POST /api/echomuse/devices/{id}/delete）
    // ============================================================

    /** 删除行内二次确认的设备号（空串 = 无）；重启确认由 management 独立持有。 */
    var confirmDeleteId by scope.observable("")

    /**
     * 删除登记记录（清理 mock/联调设备）。只清静态登记、不断开在线会话；
     * 设备再次 register 会重新出现在列表里。
     */
    fun delete(ctx: Pager, deviceId: String, onDone: (ok: Boolean, msg: String) -> Unit) {
        if (actionBusy) return
        actionBusy = true
        post(ctx, "/api/echomuse/devices/${echoMuseUrlComponent(deviceId)}/delete", JSONObject()) { ok, code, reason, _ ->
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

    // ============================================================
    // 唤醒词设置（POST /api/echomuse/devices/{id}/settings，部分更新）
    // ============================================================

    // —— 表单草稿：进入唤醒词面板时从设备 settings 快照填充，缺省走默认值 ——
    var wakeWordEnabled by scope.observable(true)
    var owwOnDevice by scope.observable("on")
    var owwModeOpen by scope.observable(false)
    val owwModes by scope.observableList<Pair<String, String>>()
    var wakeLevelOpen by scope.observable(false)
    val wakeLevels by scope.observableList<Pair<String, String>>()

    init {
        rebuildModeOptions()
        wakeLevels.addAll(listOf(
            "quiet" to "轻柔（quiet）", "medium" to "适中（medium）", "loud" to "响亮（loud）",
        ))
    }
    /**
     * 模式下拉选项：shadow 为官方诊断档，仅当前值已是 shadow 时显示（官方
     * dashboard.jsx 同规则）；保存成功或重填草稿后按最新值重建，保证选中项
     * 总能找到标签（否则下拉显示「未知」）。
     */
    private fun rebuildModeOptions() {
        owwModes.clear()
        for ((value, label) in EchoMuseWakeWord.buildModeOptions(owwOnDevice)) {
            owwModes.add(value to label)
        }
    }
    var owwModelOpen by scope.observable(false)
    /** 唤醒词模型下拉选项：官方四款 + oww-models/ 已上传模型 + 已存自定义值，进面板时重建。 */
    val owwModelOptions by scope.observableList<Pair<String, String>>()
    /** 管理面存储的原始 owwModel；空 = 设备默认（hey jarvis），展示时要标出来。 */
    var owwModelStored by scope.observable("")
    /** 下拉草稿：恒为生效模型名（存储空时解析为设备默认，见 [EchoMuseWakeWord]）。 */
    var owwModel by scope.observable(EchoMuseWakeWord.DEFAULT_MODEL)
    /** 唤醒灵敏度阈值（读数保真：轨外历史值原样保留，滑杆端点钉扎见官方 wakeTrackFor）。 */
    var owwThreshold by scope.observable(EchoMuseWakeWord.DEFAULT_THRESHOLD)
    var wakeSound by scope.observable(true)
    /** 固件提示音档位（不是输出音量 0～127）。 */
    var wakeSoundLevel by scope.observable("medium")
    /** 0=中心麦克风，范围 0~6。 */
    var wakeMic by scope.observable("0")

    /** 进入「唤醒词设置」面板时按该设备 settings 快照重填草稿（缺省走默认值）。 */
    private fun fillSettingsDraft(deviceId: String) {
        val s = devices.firstOrNull { it.deviceId == deviceId }?.settings ?: JSONObject()
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
        // 存储空 = 设备默认 hey jarvis（上游 OWW_MODEL 缺省），面板必须显示真实生效值。
        owwModelStored = s.optString("owwModel", "").trim()
        owwModel = EchoMuseWakeWord.effectiveModel(owwModelStored)
        owwModelOpen = false
        // 官方四款 + oww-models/ 已上传自定义模型 + 已存自定义值（去重），纯函数可测
        //（EchoMuseWakeWord.buildOwwOptions）。自定义值可点 ≠ 设备已装：上传/选择只是
        // 管理面配置，设备只认本地已装模型，说明文案保留该警告。
        owwModelOptions.clear()
        for ((value, label) in EchoMuseWakeWord.buildOwwOptions(owwModelStored, uploadedOwwModels)) {
            owwModelOptions.add(value to label)
        }
        // 官方读数保真：轨外历史值（旧面板可写 0.05 或 1.0）不静默改写，
        // 滑杆把把手钉在端点、数值显示存储值，拖动才写入官方量程内的新值。
        owwThreshold = when (val raw = s.opt("owwThreshold")) {
            is Number -> raw.toDouble().takeIf { it.isFinite() } ?: EchoMuseWakeWord.DEFAULT_THRESHOLD
            else -> EchoMuseWakeWord.DEFAULT_THRESHOLD
        }
        wakeSound = s.optBoolean("wakeSound", true)
        wakeSoundLevel = when (val raw = s.opt("wakeSoundLevel")) {
            "quiet", "medium", "loud" -> raw.toString()
            is Number -> "medium" // 旧数字从未被固件接受，不能猜测有效档位。
            else -> "medium"
        }
        wakeMic = intText(s.opt("wakeMic"), "0")
    }

    /**
     * 保存唤醒词设置。202（设备在线）= 已排队下发；200 = 已保存、设备下次接入时自动推送。
     * 数值字段解析失败/越界就不发请求（不发半截更新）。
     */
    fun saveSettings(ctx: Pager, deviceId: String, onDone: (ok: Boolean, msg: String) -> Unit) {
        if (owwOnDevice !in setOf("off", "shadow", "on") || wakeSoundLevel !in setOf("quiet", "medium", "loud")) {
            onDone(false, "请选择有效的本地唤醒模式和提示音档位")
            return
        }
        if (!owwThreshold.isFinite() || owwThreshold <= 0.0 || owwThreshold > 1.0) {
            onDone(false, "唤醒阈值必须大于 0 且不超过 1")
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
            put("wakeSoundLevel", wakeSoundLevel)
            put("wakeMic", mic)
        }
        post(ctx, "/api/echomuse/devices/${echoMuseUrlComponent(deviceId)}/settings", body) { ok, code, reason, data ->
            actionBusy = false
            if (ok) {
                val delivered = data?.optBoolean("delivered", false) == true ||
                    data?.optBoolean("queued", false) == true
                val msg = if (delivered) "设置已保存并排队下发，实际生效请在设备上核对" else "设置已保存，设备下次接入时自动推送"
                // 模式切到/离开 shadow 后选项集合随当前值变化，按新值重建（官方同规则）。
                rebuildModeOptions()
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
    // 实时更新：管理台 WS 事件（GET /api/admin/ws 推送），无轮询、无长挂
    // ============================================================

    /** 管理台实时通道（本页一份；同 base 的订阅由共享层合流复用连接）。 */
    private val realtime = AdminRealtime()

    /** 当前事件订阅（页面停留期间持有）；null = 未订阅。 */
    private var liveSub: AdminRealtime.Subscription? = null

    /** 订阅所用的服务端基址（base 变了要断旧连新，如 mac「切换服务器」后）。 */
    private var liveSubBase: String? = null

    /** 上次 [ensureLiveUpdates] 看到的页面驻留态（只在变化沿动作，重组幂等）。 */
    private var liveActive: Boolean? = null

    /**
     * 页面停留（[isActive]）期间订阅管理台 WS `GET /api/admin/ws`：服务端设备面有
     * 变化（上线/离线/批准/设置/删除）推 `echomuse_devices_changed` 事件，本页随即
     * 刷新列表。**无轮询、无长挂**——连接由 [AdminRealtime] 共享持有并自动重连
     * （退避 3s→30s，连接期间每 25s 应用层心跳）。渲染层每次重组都会调用本方法：
     * 仅在 isActive 变化沿动作（变 true 订阅、变 false 退订）；页面停留期间 base
     * 变了则断旧连新。订阅失败/平台无 WS 能力自动退化为「进页刷新一次」。
     */
    fun ensureLiveUpdates(ctx: Pager, isActive: () -> Boolean) {
        val active = isActive()
        if (active == liveActive && liveSubBase == base) return
        liveActive = active
        if (!active) {
            liveSub?.close()
            liveSub = null
            liveSubBase = null
            return
        }
        if (liveSub != null && liveSubBase == base) return
        liveSub?.close() // base 变了：断旧连新（旧组引用计数归零自动断开旧连接）
        liveSubBase = base
        liveSub = realtime.subscribe(ctx, base, accessToken) { type, _ ->
            if (type == "echomuse_devices_changed") refresh(ctx)
        }
    }

    /** 服务端麦克风编号转草稿；未设置时采用设备默认值。 */
    private fun intText(raw: Any?, fallback: String): String =
        (raw as? Number)?.let { it.toInt().toString() } ?: fallback

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
        val generation = connectionGeneration
        val headers = echoMuseHeaders(accessToken)
        network(ctx).httpRequest("$base$path", true, body, headers) { data, success, errorMsg, resp ->
            if (generation != connectionGeneration) return@httpRequest
            val code = resp.statusCode
            val ok = success && (code == null || code in 200..299)
            val reason = data.text("error").takeIf { it.isNotEmpty() }
                ?: data.text("message").takeIf { it.isNotEmpty() }
                ?: errorMsg
            onDone(ok, code, reason, data)
        }
    }
}
