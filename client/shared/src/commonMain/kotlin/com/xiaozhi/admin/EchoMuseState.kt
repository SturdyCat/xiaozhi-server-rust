package com.xiaozhi.admin

import com.xiaozhi.admin.ui.*

import com.tencent.kuikly.core.base.PagerScope
import com.tencent.kuikly.core.nvi.serialization.json.JSONArray
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
const val ECHOMUSE_MENU_AUDIO = "audio"
const val ECHOMUSE_MENU_MUSIC = "music"
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
    /** 最近一次 register 的 IP（排障用；旧服务端/旧记录缺省为空）。 */
    val ip: String = "",
    /** 内核架构与版本（如 armv7 / 5.10.x）。 */
    val kernelArch: String = "",
    val kernelRelease: String = "",
    /** 本次开机标识与启动原因（重启/断电/看门狗判别的唯一来源）。 */
    val bootId: String = "",
    val bootReason: String = "",
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
            if (ip.isNotEmpty()) add("IP: $ip")
            if (kernelRelease.isNotEmpty()) add("内核: $kernelRelease${if (kernelArch.isNotEmpty()) " ($kernelArch)" else ""}")
            if (bootReason.isNotEmpty()) add("启动原因: $bootReason")
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
                        ip = d.text("ip"),
                        kernelArch = d.text("kernel_arch"),
                        kernelRelease = d.text("kernel_release"),
                        bootId = d.text("boot_id"),
                        bootReason = d.text("boot_reason"),
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
    fun selectMenu(row: EchoMuseDeviceRow, item: String, ctx: Pager) {
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
            ECHOMUSE_MENU_AUDIO -> fillAudioDraft(row.deviceId)
            ECHOMUSE_MENU_MANAGE -> management.select(row, base)
            ECHOMUSE_MENU_BLE -> ble.select(row)
        }
    }

    /**
     * 当前卡片生效的菜单项：[menuDevice] 不匹配本卡、或菜单项不适用当前批准态
     * （已批准没有「批准」、未批准没有 WiFi/唤醒词/音频/音乐/管理/蓝牙）一律回落「概览」。
     */
    fun activeMenuItem(row: EchoMuseDeviceRow): String {
        if (menuDevice != row.deviceId) return ECHOMUSE_MENU_OVERVIEW
        val approvedOnly = menuItem == ECHOMUSE_MENU_WIFI ||
            menuItem == ECHOMUSE_MENU_WAKE ||
            menuItem == ECHOMUSE_MENU_AUDIO ||
            menuItem == ECHOMUSE_MENU_MUSIC ||
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
    // WiFi 配网闭环（wifi_scan → 设备上报 → wifi_commit；对齐官方三步）
    // ============================================================

    /**
     * 最近一次为「WiFi 配置」面板拉的运行状态快照（`GET …/runtime` 的原样 JSON）。
     * 扫描结果在 `wifi_scan.networks`（`[{ssid,signal}]`），切网结果在 `wifi_result`
     * （`{ok,ssid,error?}`）——都是设备上行、进 runtime 的有界遥测。
     */
    var wifiRuntime by scope.observable<JSONObject?>(null)
        private set
    var wifiRuntimeBusy by scope.observable(false)
        private set

    /** 读取运行状态快照（WiFi 面板展示扫描/切网结果用；离线时服务端回历史快照）。 */
    fun readWifiRuntime(ctx: Pager, deviceId: String) {
        if (wifiRuntimeBusy) return
        wifiRuntimeBusy = true
        getJson(ctx, echoMuseAuthenticatedUrl("$base/api/echomuse/devices/${echoMuseUrlComponent(deviceId)}/runtime", accessToken)) { ok, data, reason ->
            wifiRuntimeBusy = false
            if (ok) wifiRuntime = data
            else wifiRuntimeError = "运行状态读取失败：${reason.ifEmpty { "请确认设备在线" }}"
        }
    }
    var wifiRuntimeError by scope.observable("")

    /** 下发 `wifi_scan`（扫描周边网络）。202 = 已排队；结果经「读取扫描结果」拉 runtime。 */
    fun sendWifiScan(ctx: Pager, deviceId: String, onDone: (ok: Boolean, msg: String) -> Unit) {
        command(ctx, deviceId, JSONObject().apply { put("action", "wifi_scan") }) { ok, msg ->
            if (ok) readWifiRuntime(ctx, deviceId)
            onDone(ok, if (ok) "扫描指令已下发，稍候点击「读取扫描结果」查看周边网络" else msg)
        }
    }

    /**
     * 下发 `wifi_commit`（确认新网络）。设备切网重连成功后处于回滚窗口，commit 前先
     * 在 `wifi_result.ok` 里确认新网可用；超时未 commit 设备自动回滚旧网。
     */
    fun sendWifiCommit(ctx: Pager, deviceId: String, onDone: (ok: Boolean, msg: String) -> Unit) {
        command(ctx, deviceId, JSONObject().apply { put("action", "wifi_commit") }) { ok, msg ->
            onDone(ok, if (ok) "已确认新网络；设备将固定使用新网接入" else msg)
        }
    }

    /** WiFi 扫描结果（`wifi_scan.networks` 解析为「显示行 + 信号值」）；未扫描为空表。 */
    private val wifiScanEntries: List<Pair<String, Int?>>
        get() {
            val networks = wifiRuntime?.optJSONObject("wifi_scan")?.array("networks") ?: return emptyList()
            return (0 until networks.length()).mapNotNull { i ->
                val n = networks.optJSONObject(i) ?: return@mapNotNull null
                val ssid = n.text("ssid").ifEmpty { "(隐藏网络)" }
                val signal = (n.opt("signal") as? Number)?.toInt()
                val line = if (signal != null) "$ssid · $signal dBm" else ssid
                line to signal
            }
        }

    /** 扫描结果展示行（按信号强度降序：强在前）。 */
    val wifiScanRows: List<String>
        get() = wifiScanEntries
            .sortedWith(compareByDescending<Pair<String, Int?>> { it.second }.thenBy { it.first })
            .map { it.first }

    /** 最近一次切网结果的一行摘要（`wifi_result`）；设备未上报为 null。 */
    val wifiResultText: String?
        get() {
            val r = wifiRuntime?.optJSONObject("wifi_result") ?: return null
            if (r.length() == 0) return null
            val ok = r.opt("ok") as? Boolean
            val ssid = r.text("ssid")
            val error = r.text("error")
            return when {
                ok == true -> "新网络连接成功${if (ssid.isNotEmpty()) "（$ssid）" else ""}——可点击「确认新网络」固定"
                ok == false -> "新网络连接失败${if (error.isNotEmpty()) "：$error" else ""}；回滚窗口结束后设备回到旧网"
                else -> null
            }
        }

    // ============================================================
    // 音频调优（settings 的采集/回声/打断/输出链键；面板见 EchoMuseAudioPanel.kt）
    // ============================================================

    // —— 表单草稿：进入面板时按设备 settings 快照重填；数值**留空 = 不修改**（不写死默认值）——
    var vadThresholdDraft by scope.observable("")
    var vadSpeechMsDraft by scope.observable("")
    var vadSilenceMsDraft by scope.observable("")
    var adcMicpgaDraft by scope.observable("")
    var adcDigitalGainDraft by scope.observable("")
    var micGainDbDraft by scope.observable("")
    var startupVolumeDraft by scope.observable("")
    var aecEnabled by scope.observable(true)
    var aecDelayMsDraft by scope.observable("")
    var aecTailMsDraft by scope.observable("")
    var aecRefSource by scope.observable("auto")
    var aecRefSourceOpen by scope.observable(false)
    val aecRefSources by scope.observableList<Pair<String, String>>()
    var agcEnabled by scope.observable(true)
    var beamformingEnabled by scope.observable(true)
    var beamAngleDraft by scope.observable("")
    var bargeInEnabled by scope.observable(true)
    var bargeInThresholdDraft by scope.observable("")
    var responseLevel by scope.observable("medium")
    var responseLevelOpen by scope.observable(false)
    val responseLevels by scope.observableList<Pair<String, String>>()
    var volumeButtonSound by scope.observable(true)
    var remoteVolumeArc by scope.observable(true)
    var eqLoudness by scope.observable(false)
    var bassGuardEnabled by scope.observable(true)
    var bassGuardDbDraft by scope.observable("")
    var limiterEnabled by scope.observable(true)
    var limiterThresholdDraft by scope.observable("")
    var limiterReleaseDraft by scope.observable("")
    var duckDbDraft by scope.observable("")
    /** EQ 频段 JSON（数字数组，如 `[2,0,-1,0,2]`）；空 = 不修改。 */
    var eqBandsDraft by scope.observable("")
    /** 私听态灯环动画 spec JSON；空 = 不修改。 */
    var listeningAnimDraft by scope.observable("")

    init {
        aecRefSources.addAll(listOf(
            "auto" to "自动检测（推荐）", "hw" to "硬件环回（hw）", "sw" to "软件参考（sw）",
        ))
        responseLevels.addAll(listOf(
            "low" to "低（0 dB）", "medium" to "中（+6 dB）", "high" to "高（+12 dB）",
        ))
    }

    /** 进入「音频调优」面板时按该设备 settings 快照重填草稿；数值键缺省留空（= 不修改）。 */
    private fun fillAudioDraft(deviceId: String) {
        val s = devices.firstOrNull { it.deviceId == deviceId }?.settings ?: JSONObject()
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
        responseLevel = s.optString("responseLevel", "medium").takeIf { it in setOf("low", "medium", "high") } ?: "medium"
        responseLevelOpen = false
        volumeButtonSound = s.optBoolean("volumeButtonSound", true)
        remoteVolumeArc = s.optBoolean("remoteVolumeArc", true)
        eqLoudness = s.optBoolean("eqLoudness", false)
        bassGuardEnabled = s.optBoolean("bassGuardEnabled", true)
        bassGuardDbDraft = num("bassGuardDb")
        limiterEnabled = s.optBoolean("limiterEnabled", true)
        limiterThresholdDraft = num("limiterThreshold")
        limiterReleaseDraft = num("limiterRelease")
        duckDbDraft = num("duckDb")
        eqBandsDraft = s.optJSONArray("eqBands")?.toString().orEmpty()
        listeningAnimDraft = s.optJSONObject("listeningAnim")?.toString().orEmpty()
    }

    /**
     * 保存音频调优。**部分更新语义**：数值/JSON 字段留空 = 不发该键（不把默认值写死成
     * 显式设置——协议按指针类型区分"未设置"与"设为零"，这一点与固件口径一致）；
     * 开关/枚举恒发（有明确视觉状态）。202 = 设备在线已排队下发，200 = 保存待接入。
     */
    fun saveAudioSettings(ctx: Pager, deviceId: String, onDone: (ok: Boolean, msg: String) -> Unit) {
        val body = JSONObject()
        fun int(key: String, draft: String, range: IntRange, label: String): Boolean {
            val t = draft.trim()
            if (t.isEmpty()) return true
            val v = t.toIntOrNull()
            if (v == null || v !in range) {
                onDone(false, "$label 需要 $range 范围内的整数")
                return false
            }
            body.put(key, v)
            return true
        }
        fun dec(key: String, draft: String, lo: Double, hi: Double, label: String): Boolean {
            val t = draft.trim()
            if (t.isEmpty()) return true
            val v = t.toDoubleOrNull()
            if (v == null || !v.isFinite() || v < lo || v > hi) {
                onDone(false, "$label 需要 $lo ～ $hi 之间的数字")
                return false
            }
            body.put(key, v)
            return true
        }
        // 语音结束判定
        if (!dec("vadThreshold", vadThresholdDraft, 0.0001, 1.0, "VAD 门限")) return
        if (!int("vadSpeechMs", vadSpeechMsDraft, 0..60000, "开口判定时长")) return
        if (!int("vadSilenceMs", vadSilenceMsDraft, 0..60000, "收尾静音时长")) return
        // 采集增益
        if (!int("adcMicpga", adcMicpgaDraft, 0..127, "模拟前端增益")) return
        if (!int("adcDigitalGain", adcDigitalGainDraft, 0..127, "数字增益")) return
        if (!int("micGainDb", micGainDbDraft, 0..42, "处理增益（dB）")) return
        if (!int("startupVolume", startupVolumeDraft, 0..127, "开机音量")) return
        // AEC
        if (!int("aecDelayMs", aecDelayMsDraft, 0..1000, "AEC 参考延时")) return
        if (!int("aecTailMs", aecTailMsDraft, 0..500, "AEC 尾长")) return
        // 波束与打断
        if (!dec("beamAngle", beamAngleDraft, -1.0, 360.0, "波束角度")) return
        if (!dec("bargeInThreshold", bargeInThresholdDraft, 0.0, 1.0, "打断阈值")) return
        // 输出链
        if (!dec("bassGuardDb", bassGuardDbDraft, -60.0, 24.0, "低音保护阈值")) return
        if (!dec("limiterThreshold", limiterThresholdDraft, -60.0, 0.0, "限幅阈值")) return
        if (!dec("limiterRelease", limiterReleaseDraft, 10.0, 2000.0, "限幅释放时间")) return
        if (!dec("duckDb", duckDbDraft, -60.0, 0.0, "音乐压低深度")) return
        // JSON 进阶键：非空时必须能解析且形状正确（构造失败 = 文案报错，不发半截更新；
        // 服务端白名单 kind 仍是权威校验）。
        val eqText = eqBandsDraft.trim()
        if (eqText.isNotEmpty()) {
            val bands = try { JSONArray(eqText) } catch (_: Exception) { null }
            if (bands == null || bands.length() == 0 ||
                (0 until bands.length()).any { bands.opt(it) !is Number }
            ) {
                onDone(false, "EQ 频段需要是非空数字数组，如 [2,0,-1,0,2]")
                return
            }
            body.put("eqBands", bands)
        }
        val animText = listeningAnimDraft.trim()
        if (animText.isNotEmpty()) {
            val anim = try { JSONObject(animText) } catch (_: Exception) { null }
            if (anim == null) {
                onDone(false, "聆听灯环动画需要是 JSON 对象（如 {\"pattern\":\"spin\",\"colors\":[[0,180,0]],\"ttlSec\":30}）")
                return
            }
            body.put("listeningAnim", anim)
        }
        // 开关/枚举恒发（有明确视觉状态，写回现值无害）
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
        if (actionBusy) return
        actionBusy = true
        post(ctx, "/api/echomuse/devices/${echoMuseUrlComponent(deviceId)}/settings", body) { ok, code, reason, data ->
            actionBusy = false
            if (ok) {
                val delivered = data?.optBoolean("delivered", false) == true
                onDone(
                    true,
                    if (delivered) "音频设置已保存并排队下发，实际生效请在设备上核对"
                    else "音频设置已保存，设备下次接入时自动推送",
                )
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
    // 音乐面（POST …/music scan/browse/play_url/stop；音源仅 DLNA，
    // 扫描/浏览/拉流/推流全部在服务端完成，管理台只「选服务器 → 浏览 →
    // 选曲目 → 点播放」）
    // ============================================================

    /** 局域网 DLNA 媒体服务器（value=location 设备描述地址，label=友好名）。 */
    var dlnaServers by scope.observableList<Pair<String, String>>()
        private set
    var dlnaServersLoaded by scope.observable(false)
        private set
    var dlnaServerOpen by scope.observable(false)
    /** 已选服务器（location；非空时面板展示曲目区）。 */
    var dlnaServerSelection by scope.observable("")

    /** 当前服务器的音频曲目（value=直链 url，label=标题（艺术家 · 专辑））。 */
    var dlnaTracks by scope.observableList<Pair<String, String>>()
        private set
    var dlnaTracksLoaded by scope.observable(false)
        private set
    var dlnaTrackOpen by scope.observable(false)
    /** 已选曲目（直链 url）。 */
    var dlnaTrackSelection by scope.observable("")

    /** 选中服务器（下拉 onSelect）：切换服务器必须清空上一台的曲目，避免串台。 */
    fun dlnaSelectServer(location: String) {
        dlnaServerSelection = location
        dlnaTracks.clear()
        dlnaTracksLoaded = false
        dlnaTrackSelection = ""
    }

    /**
     * 扫描局域网 DLNA 媒体服务器（POST music {action:scan}）。SSDP 发现与设备描述
     * 解析都在服务端完成（约 4s 窗口），不依赖设备在线；容器部署收不到组播时服务端
     * 响应会带 hint，原样透出给用户。
     */
    fun dlnaScan(ctx: Pager, deviceId: String, onDone: (ok: Boolean, msg: String) -> Unit) {
        if (actionBusy) return
        actionBusy = true
        post(ctx, "/api/echomuse/devices/${echoMuseUrlComponent(deviceId)}/music", JSONObject().apply {
            put("action", "scan")
        }) { ok, _, reason, data ->
            actionBusy = false
            if (!ok) {
                onDone(false, "扫描失败：${reason.ifEmpty { "HTTP 异常" }}")
                return@post
            }
            val servers = data?.array("servers")?.let { arr ->
                (0 until arr.length()).mapNotNull { i ->
                    val e = arr.optJSONObject(i) ?: return@mapNotNull null
                    val location = e.text("location")
                    if (location.isEmpty()) null else location to e.text("name")
                }
            } ?: emptyList()
            dlnaServers.clear()
            dlnaServers.addAll(servers)
            dlnaServersLoaded = true
            if (servers.isEmpty()) {
                onDone(true, data?.text("hint")?.takeIf { it.isNotEmpty() }
                    ?: "局域网未发现 DLNA 媒体服务器")
            } else {
                onDone(true, "发现 ${servers.size} 台 DLNA 媒体服务器，请在下拉中选择")
            }
        }
    }

    /** 浏览已选服务器的音频曲目（POST music {action:browse}；SOAP 递归在服务端，大库可能数秒）。 */
    fun dlnaBrowse(ctx: Pager, deviceId: String, onDone: (ok: Boolean, msg: String) -> Unit) {
        val location = dlnaServerSelection
        if (location.isEmpty()) {
            onDone(false, "请先扫描并选择一台 DLNA 媒体服务器")
            return
        }
        if (actionBusy) return
        actionBusy = true
        post(ctx, "/api/echomuse/devices/${echoMuseUrlComponent(deviceId)}/music", JSONObject().apply {
            put("action", "browse")
            put("location", location)
        }) { ok, _, reason, data ->
            actionBusy = false
            if (!ok) {
                onDone(false, "浏览失败：${reason.ifEmpty { "HTTP 异常" }}")
                return@post
            }
            val tracks = data?.array("tracks")?.let { arr ->
                (0 until arr.length()).mapNotNull { i ->
                    val e = arr.optJSONObject(i) ?: return@mapNotNull null
                    val url = e.text("url")
                    if (url.isEmpty()) return@mapNotNull null
                    val title = e.text("title").ifEmpty { url }
                    val sub = listOf(e.text("artist"), e.text("album")).filter { it.isNotEmpty() }
                        .joinToString(" · ")
                    url to if (sub.isEmpty()) title else "$title（$sub）"
                }
            } ?: emptyList()
            dlnaTracks.clear()
            dlnaTracks.addAll(tracks)
            dlnaTracksLoaded = true
            if (tracks.isEmpty()) {
                onDone(true, "该服务器没有列出音频曲目")
            } else {
                onDone(true, "已列出 ${tracks.size} 个曲目，请选择播放")
            }
        }
    }

    /** 播放所选 DLNA 曲目（POST music {action:play_url}；服务端拉流→解码→复用音乐通道推流）。 */
    fun dlnaPlay(ctx: Pager, deviceId: String, onDone: (ok: Boolean, msg: String) -> Unit) {
        val url = dlnaTrackSelection
        if (url.isEmpty()) {
            onDone(false, "请先浏览并选择要播放的曲目")
            return
        }
        if (actionBusy) return
        actionBusy = true
        post(ctx, "/api/echomuse/devices/${echoMuseUrlComponent(deviceId)}/music", JSONObject().apply {
            put("action", "play_url")
            put("url", url)
        }) { ok, code, reason, _ ->
            actionBusy = false
            if (ok) {
                onDone(true, "已排队播放：服务端正在拉流解码并推送到设备，说话会自动压低音乐")
            } else {
                onDone(false, when (code) {
                    409 -> "设备离线或 data 链未就绪，播放未开始"
                    400 -> "播放失败：${reason.ifEmpty { "曲目不可用" }}"
                    else -> "播放失败：${reason.ifEmpty { "HTTP $code" }}"
                })
            }
        }
    }

    /** 停止音乐并丢弃设备缓冲（POST music {action:stop}；不可恢复，恢复播放请重新点播放）。 */
    fun stopMusic(ctx: Pager, deviceId: String, onDone: (ok: Boolean, msg: String) -> Unit) {
        if (actionBusy) return
        actionBusy = true
        post(ctx, "/api/echomuse/devices/${echoMuseUrlComponent(deviceId)}/music", JSONObject().apply {
            put("action", "stop")
        }) { ok, code, reason, _ ->
            actionBusy = false
            if (ok) onDone(true, "已停止播放并清空设备音乐缓冲")
            else onDone(false, when (code) {
                409 -> "设备离线，无需停止"
                else -> "停止失败：${reason.ifEmpty { "HTTP $code" }}"
            })
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
