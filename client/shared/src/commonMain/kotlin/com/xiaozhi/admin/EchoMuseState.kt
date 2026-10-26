package com.xiaozhi.admin

import com.xiaozhi.admin.ui.*

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
 * ⚠️ 本类不渲染：渲染见 `EchoMusePage.kt` / `EchoMuseDeviceCard.kt` / `EchoMuse*Tab.kt`。
 *
 * 2026-10 两次拆分：
 * - T03：卡片内的分区选择态与运行状态/系统信息/音量/控制台搬到 [EchoMuseDetailState]，
 *   日志搬到 [EchoMuseLogsState]；
 * - T04：**配置草稿**（唤醒词 + 音频链 + Sendspin 共 48 个字段）搬到
 *   [EchoMuseConfigDrafts]，本类只剩列表、审批、WiFi 配网闭环与实时订阅；卡内左菜单的
 *   `openSection` 随之删除，六 tab 的唯一入口是 [enterTab]；**音乐面**搬到
 *   [EchoMuseMusicState]（本文件已到 600 行上限，而音乐面自成一个写动作集合）。
 */
class EchoMuseState(scope: Pager) {
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

    /** 详情卡分区状态（六 tab + 运行状态/系统信息/音量/OWW 资产/控制台会话）。 */
    val detail = EchoMuseDetailState(scope)

    /** 日志 tab 状态（固定来源白名单 + 读取 + 复制）。 */
    val logs = EchoMuseLogsState(scope, detail)

    /** 配置 tab 的全部表单草稿（Stage 01–07 的键）与"草稿 → 请求体"组装。 */
    val drafts = EchoMuseConfigDrafts(scope)

    val ble = EchoMuseBleState(scope)
    private var accessToken = ""

    /**
     * 音乐面状态（配置 tab 的 Stage 08：DLNA 扫描/浏览/播放/停止）。
     *
     * 基址与令牌以 lambda 传入：音乐面自成一个写动作集合（不占设备 shell 槽，也不跟
     * [actionBusy] 抢），但"切服务器后旧回包必须作废"这条要跟主状态一致。
     */
    val music = EchoMuseMusicState(scope, { base }, { accessToken })

    /** 连接参数是一个整体；同 token 跨服务器也必须使旧请求失效。 */
    fun configureAccess(token: String, baseUrl: String = "") {
        if (accessToken != token || base != baseUrl) {
            connectionGeneration++
            loading = false
            loaded = false
            loadError = ""
            devices.clear()
            detail.leave()
            actionBusy = false
            confirmDeleteId = ""
            approveDraft = ""
            wifiSsid = ""
            wifiPsk = ""
            wifiPending = false
            wifiRuntime = null
            wifiRuntimeError = ""
            // 实时订阅断开：base/token 都变了，旧连接（旧凭据）立即退订，
            // 下次渲染经 ensureLiveUpdates 以新参数重订（退订即退共享组引用）。
            liveSub?.close()
            liveSub = null
            liveSubBase = null
            liveActive = null
        }
        accessToken = token
        base = baseUrl
        detail.configureConnection(baseUrl, token)
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
        detail.sync(devices)
        // 日志输出带设备号：切设备后上一台的日志必须作废，不能跟着列表一起"继承"过来
        logs.syncDevice(detail.target)
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
    // tab 进入（六 tab 的唯一入口）
    // ============================================================

    /**
     * 切到某台设备的某个 tab（六 tab 视图只调这一个入口）。
     *
     * 分两半：
     * 1. **状态半**委托 [EchoMuseDetailState.enterTab]：懒加载、开会话、起/停轮询；
     * 2. **草稿半**留在这里：进「配置」时按设备 `settings` 快照重填 [drafts]。
     *
     * 为什么草稿填充不放进 `EchoMuseDetailState`：那个对象刻意只持有"设备域快照"
     * （诊断要用它，不能混进用户的未保存输入）；草稿是另一个生命周期——只要用户在
     * 同一台设备的配置 tab 里，就不该被列表刷新或重渲染覆盖。
     *
     * **同设备同 tab 重复点不重填**（否则用户改了半天的输入会被一次误点清空）。
     * 蓝牙代理的目标在进配置 tab 时一起登记（[EchoMuseBleState.select] 幂等：target
     * 已是本设备就保留已读到的广播/槽位输出）。
     */
    fun enterTab(row: EchoMuseDeviceRow, label: String, ctx: Pager) {
        val refill = detail.target != row.deviceId || detail.tab != label
        detail.enterTab(label, row, ctx)
        if (label == EM_TAB_CONFIG) {
            if (refill) fillConfigDrafts(row)
            ble.select(row)
        }
    }

    /** 按设备 `settings` 快照重填配置草稿（进配置 tab、或用户点「按设备现值重新填充」）。 */
    fun fillConfigDrafts(row: EchoMuseDeviceRow) {
        drafts.fill(row.settings, uploadedOwwModels)
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
                // 批准后该设备从"审批单页"切到六 tab（`row.approved` 刷新后即变），无需改分区态
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

    /** 写动作互斥锁（批准/重启/WiFi/音乐共用：单设备低频管理操作，无需并发）。 */
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

    /** 下发 WiFi 配置（ssid + psk）。成功后置「待确认」横幅（切网结果以设备上报为准）。 */
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
                wifiPending = true
            }
            onDone(ok, msg)
        }
    }

    // ============================================================
    // 删除记录（POST /api/echomuse/devices/{id}/delete）
    // ============================================================

    /** 删除行内二次确认的设备号（空串 = 无）；重启确认由 detail 独立持有。 */
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
    // 配置下发（POST /api/echomuse/devices/{id}/settings，部分更新）
    // ============================================================

    /**
     * 下发配置（配置 tab 底部唯一的写动作）。
     *
     * 请求体由 [EchoMuseConfigDrafts.buildConfigBody] 组装——**校验失败一个字都不发**
     * （不发半截更新），部分更新语义（数值/JSON 留空 = 不发该键）也在那里逐键实现。
     * 202/`delivered` = 设备在线已排队下发；200 = 已保存、设备下次接入自动推送。
     */
    fun saveConfig(ctx: Pager, deviceId: String, onDone: (ok: Boolean, msg: String) -> Unit) {
        if (actionBusy) return
        val body = drafts.buildConfigBody(onError = { onDone(false, it) }) ?: return
        actionBusy = true
        post(ctx, "/api/echomuse/devices/${echoMuseUrlComponent(deviceId)}/settings", body) { ok, code, reason, data ->
            actionBusy = false
            if (ok) {
                val delivered = data?.optBoolean("delivered", false) == true ||
                    data?.optBoolean("queued", false) == true
                val msg = if (delivered) {
                    "配置已保存并排队下发，实际生效请在设备上核对"
                } else {
                    "配置已保存，设备下次接入时自动推送"
                }
                // 模式切到/离开 shadow 后选项集合随当前值变化，按新值重建（官方同规则）。
                drafts.rebuildModeOptions()
                onDone(true, msg)
            } else {
                onDone(false, when (code) {
                    400 -> "配置未保存：${reason.ifEmpty { "字段不合法（不在白名单或形状不对）" }}"
                    404 -> "设备不存在（列表可能已过期，请刷新后重试）"
                    else -> "配置保存失败：${reason.ifEmpty { "HTTP $code" }}"
                })
            }
        }
    }

    // ============================================================
    // WiFi 配网闭环（wifi_scan → 设备上报 → wifi_commit；对齐官方三步）
    // ============================================================

    /**
     * 最近一次为「网络连接」块拉的运行状态快照（`GET …/runtime` 的原样 JSON）。
     * 扫描结果在 `wifi_scan.networks`（`[{ssid,signal}]`），切网结果在 `wifi_result`
     * （`{ok,ssid,error?}`）——都是设备上行、进 runtime 的有界遥测。
     */
    var wifiRuntime by scope.observable<JSONObject?>(null)
        private set
    var wifiRuntimeBusy by scope.observable(false)
        private set

    /**
     * 是否处于「已下发新网、等设备确认」的窗口（配置 tab 的待确认横幅）。
     *
     * 置位在 [sendWifi] 成功之后，清位在 [sendWifiCommit] 成功之后——中间这段既不能
     * 说成功也不能说失败：设备可能正在重连、也可能已经回滚到旧网。
     */
    var wifiPending by scope.observable(false)

    /** 当前连接的网络名（`wifi_runtime.stats.wifiSsid`；未上报 = 空串 = 未知）。 */
    val wifiCurrentSsid: String
        get() = wifiRuntime?.optJSONObject("stats")?.text("wifiSsid").orEmpty()

    /** 读取运行状态快照（WiFi 三块展示扫描/切网结果用；离线时服务端回历史快照）。 */
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
            if (ok) wifiPending = false
            onDone(ok, if (ok) "已确认新网络；设备将固定使用新网接入" else msg)
        }
    }

    /**
     * WiFi 扫描结果（`wifi_scan.networks` 解析为「ssid + 显示行 + 信号值」）；未扫描为空表。
     *
     * 带 ssid 是为了在展示时给当前连接那条打 `← 当前` 标记（判断依据只有设备上报的
     * `stats.wifiSsid`，不按信号强弱猜）。
     */
    private val wifiScanEntries: List<Triple<String, String, Int?>>
        get() {
            val networks = wifiRuntime?.optJSONObject("wifi_scan")?.array("networks") ?: return emptyList()
            return (0 until networks.length()).mapNotNull { i ->
                val n = networks.optJSONObject(i) ?: return@mapNotNull null
                val ssid = n.text("ssid").ifEmpty { "(隐藏网络)" }
                val signal = (n.opt("signal") as? Number)?.toInt()
                val line = if (signal != null) "$ssid · $signal dBm" else ssid
                Triple(ssid, line, signal)
            }
        }

    /** 扫描结果展示行（按信号强度降序：强在前；当前连接那条标 `← 当前`）。 */
    val wifiScanRows: List<String>
        get() {
            val current = wifiCurrentSsid
            return wifiScanEntries
                .sortedWith(compareByDescending<Triple<String, String, Int?>> { it.third }.thenBy { it.first })
                .map { entry ->
                    if (current.isNotEmpty() && entry.first == current) "${entry.second}  ← 当前" else entry.second
                }
        }

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
