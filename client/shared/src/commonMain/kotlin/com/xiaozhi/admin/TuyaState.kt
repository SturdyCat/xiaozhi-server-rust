package com.xiaozhi.admin


import com.tencent.kuikly.core.nvi.serialization.json.JSONArray
import com.tencent.kuikly.core.nvi.serialization.json.JSONObject
import com.tencent.kuikly.core.pager.Pager
import com.xiaozhi.admin.state.stateVar
import com.xiaozhi.admin.state.stateListOf

/**
 * 「涂鸦设备」页的数据源（`/api/tuya` 系列端点：凭证 + 同步 + 设备目录 + DP 控制）。
 *
 * 与 EchoMuse（批准制登记）/ ESP（进程内注册表）不同：涂鸦设备目录来自**云同步**
 * （`POST /api/tuya/sync`），控制走服务端的**纯局域网**通道（UI 只见三归类错误与
 * 建议动作）。服务端 `[tuya].enabled=false` 时全部端点 404，页面据此显示"未启用"。
 *
 * ## 凭证铁律：secret 永不回显
 *
 * 服务端在任何响应里都不返回 access_secret 原值；本类只在 `configured` +
 * `accessIdMasked` 上展示"已配置"。录入表单的 secret 输入框保存成功后**立即清空**，
 * 再次进入只显示"已配置（掩码）"。
 *
 * ## 请求纪律（参照 NluSkillCatalogState 的 generation ticket 模式）
 *
 * 每个请求带三重凭证：`generation` 票号 + `base` + `adminAccessRevision`——
 * 迟到的回包（切服务器 / 重新连接后）一律丢弃，避免旧服务器数据覆盖新会话状态。
 *
 * ⚠️ 本类不渲染：渲染见 `DeviceManagementCenter.kt` / `TuyaPage.kt`（零件）/ `TuyaDeviceDialog.kt`。
 */
class TuyaState(private val scope: Pager) {

    /** 请求票据：configureAccess / 每次发起新请求时递增，回包对不上号即丢弃。 */
    private var generation = 0

    /** 上次使用的服务端基址（web 同域为空串；macApp 连接流程传绝对地址）。 */
    private var base = ""

    // ---- 列表 / 状态 ----
    var loading by stateVar(false)
        private set
    var loaded by stateVar(false)
        private set
    var loadError by stateVar("")
        private set

    /** 服务端 `[tuya]` 开关（false 时页面显示"未启用"）。 */
    var enabled by stateVar(true)

    /** 凭证是否已配置（secret 原值服务端永不返回）。 */
    var configured by stateVar(false)
    var accessIdMasked by stateVar("")
    var region by stateVar("cn")
    var lastSyncText by stateVar("")
    var lastSyncResult by stateVar("")
    /** 最近一次同步的具体消息（失败原因，如 code 1108；服务端 last_sync.message）。 */
    var lastSyncMessage by stateVar("")
    var syncing by stateVar(false)
    var storePath by stateVar("")

    val devices: MutableList<TuyaDeviceRow> = stateListOf()

    /** LAN 扫描发现的未登记设备快照（不纳管，目录以云同步为准；服务端内存态）。 */
    val lanUnregistered: MutableList<TuyaLanDevice> = stateListOf()

    // ---- 凭证录入表单（secret 保存成功后立即清空，绝不回显）----
    var formRegion by stateVar("cn")
    var formAccessId by stateVar("")
    var formAccessSecret by stateVar("")
    var credentialsBusy by stateVar(false)

    // ---- 详情（单开一台设备）----
    var detailId by stateVar("")
    var detailName by stateVar("")
    var detailModel by stateVar("")
    var detailOnline by stateVar(false)
    var detailLoading by stateVar(false)
    var detailError by stateVar("")
    val dps: MutableList<TuyaDpRow> = stateListOf()
    var dpBusy by stateVar(false)
    var dpMessage by stateVar("")
    var aliasDraft by stateVar("")

    /** 数值 DP 的行内编辑目标（-1 = 无）；草稿共用一个输入框（一次改一个点）。 */
    var numberDp by stateVar(-1)
    var numberDraft by stateVar("")

    // ============================================================
    // 连接与列表
    // ============================================================

    /** 连接参数是一个整体；base 变化即作废全部在途请求与已加载状态（token 每请求现取）。 */
    fun configureAccess(token: String, baseUrl: String = "") {
        if (base != baseUrl) {
            generation++
            loading = false
            loaded = false
            loadError = ""
            devices.clear()
            leaveDetail()
            syncing = false
        }
        base = baseUrl
    }

    fun load(ctx: Pager, baseUrl: String = "") {
        if (base != baseUrl) configureAccess("", baseUrl)
        if (loading) return
        val ticket = ++generation
        base = baseUrl
        loading = true
        loadError = ""
        request(ctx, "GET", "$base/api/tuya/status", null) { ok, _, _, data ->
            if (ticket != generation) return@request
            loading = false
            if (!ok) {
                loadError = "涂鸦状态加载失败：${data?.text("error").orEmpty().ifEmpty { "请确认服务端版本" }}"
                return@request
            }
            val d = data ?: JSONObject()
            enabled = d.optBoolean("enabled", true)
            configured = d.optBoolean("configured", false)
            accessIdMasked = d.text("access_id_masked")
            region = d.text("region").ifEmpty { "cn" }
            syncing = d.optBoolean("syncing", false)
            storePath = d.text("store_path")
            val last = d.optJSONObject("last_sync")
            if (last == null) {
                lastSyncText = "从未同步"
                lastSyncResult = ""
                lastSyncMessage = ""
            } else {
                lastSyncResult = last.text("result")
                lastSyncMessage = last.text("message")
                val count = last.optLong("device_count") ?: 0L
                val age = last.optLong("age_ms") ?: 0L
                lastSyncText = "最近同步：${resultLabel(lastSyncResult)} · $count 台 · ${humanAge(age)}"
            }
            lanUnregistered.clear()
            val lan = d.array("lan_unregistered")
            if (lan != null) {
                for (i in 0 until lan.length()) {
                    val u = lan.optJSONObject(i) ?: continue
                    lanUnregistered.add(
                        TuyaLanDevice(
                            deviceId = u.text("device_id"),
                            ip = u.text("ip"),
                            ageMs = (u.opt("age_ms") as? Number)?.toLong() ?: 0L,
                        ),
                    )
                }
            }
            // 状态就绪后拉设备目录（同票据内递进，避免两次全量竞态）
            loadDevices(ctx, ticket)
        }
    }

    fun refresh(ctx: Pager) = load(ctx, base)

    private fun loadDevices(ctx: Pager, ticket: Int) {
        request(ctx, "GET", "$base/api/tuya/devices", null) { ok, _, _, data ->
            if (ticket != generation) return@request
            if (!ok) {
                loadError = "设备目录加载失败：${data?.text("error").orEmpty().ifEmpty { "请稍后重试" }}"
                loaded = true
                return@request
            }
            devices.clear()
            val arr = (data ?: JSONObject()).array("devices")
            if (arr != null) {
                for (i in 0 until arr.length()) {
                    val d = arr.optJSONObject(i) ?: continue
                    devices.add(
                        TuyaDeviceRow(
                            deviceId = d.text("device_id"),
                            name = d.text("name"),
                            model = d.text("model"),
                            online = d.optBoolean("online", false),
                            ip = d.text("ip"),
                            updatedAt = (d.opt("updated_at") as? Number)?.toLong() ?: 0L,
                        ),
                    )
                }
            }
            loaded = true
            loadError = ""
        }
    }

    fun summaryText(): String = when {
        !enabled -> "涂鸦设备未启用"
        !configured -> "凭证未配置 · ${devices.size} 台"
        else -> "共 ${devices.size} 台 · ${devices.count { it.online }} 台在线"
    }

    // ============================================================
    // 凭证（secret 只写不读）
    // ============================================================

    /** 保存凭证。成功后清空 secret 草稿（服务端不再回显，本地草稿也不留）。 */
    fun saveCredentials(ctx: Pager, onDone: (ok: Boolean, msg: String) -> Unit) {
        if (credentialsBusy) return
        val id = formAccessId.trim()
        if (id.isEmpty()) {
            onDone(false, "请先填写 access_id")
            return
        }
        if (!configured && formAccessSecret.trim().isEmpty()) {
            onDone(false, "请填写 access_secret（已配置过可留空保留旧值）")
            return
        }
        credentialsBusy = true
        val ticket = ++generation
        val body = JSONObject().apply {
            put("region", formRegion)
            put("access_id", id)
            put("access_secret", formAccessSecret.trim())
        }
        request(ctx, "POST", "$base/api/tuya/credentials", body) { ok, _, _, data ->
            if (ticket != generation) return@request
            credentialsBusy = false
            if (ok) {
                formAccessSecret = ""
                onDone(true, "凭证已保存（secret 不再回显）")
                load(ctx, base)
            } else {
                onDone(false, "保存失败：${data?.text("error").orEmpty().ifEmpty { "请稍后重试" }}")
            }
        }
    }

    /** 清除凭证（行内二次确认后调用）。 */
    fun clearCredentials(ctx: Pager, onDone: (ok: Boolean, msg: String) -> Unit) {
        if (credentialsBusy) return
        credentialsBusy = true
        val ticket = ++generation
        request(ctx, "POST", "$base/api/tuya/credentials/delete", JSONObject()) { ok, _, _, data ->
            if (ticket != generation) return@request
            credentialsBusy = false
            if (ok) {
                formAccessSecret = ""
                onDone(true, "凭证已清除")
                load(ctx, base)
            } else {
                onDone(false, "清除失败：${data?.text("error").orEmpty().ifEmpty { "请稍后重试" }}")
            }
        }
    }

    // ============================================================
    // 同步（60s 闸门在服务端；skipped = 拦下，不算失败）
    // ============================================================

    fun syncNow(ctx: Pager, onDone: (ok: Boolean, msg: String) -> Unit) {
        if (syncing) return
        syncing = true
        val ticket = ++generation
        request(ctx, "POST", "$base/api/tuya/sync", JSONObject()) { ok, _, _, data ->
            if (ticket != generation) return@request
            syncing = false
            val d = data ?: JSONObject()
            val result = d.text("result")
            val msg = d.text("message").ifEmpty { "同步完成" }
            if (ok) {
                onDone(result != "skipped", msg)
                load(ctx, base)
            } else {
                onDone(false, msg)
            }
        }
    }

    // ============================================================
    // 详情 + DP 控制 + 别名
    // ============================================================

    /** 进入详情：行信息先垫底（即时可见），再拉 DP 快照。 */
    fun openDetail(ctx: Pager, row: TuyaDeviceRow) {
        detailId = row.deviceId
        detailName = row.name
        detailModel = row.model
        detailOnline = row.online
        detailLoading = true
        detailError = ""
        dpMessage = ""
        aliasDraft = row.name
        numberDp = -1
        val ticket = ++generation
        request(ctx, "GET", "$base/api/tuya/devices/${urlComponent(row.deviceId)}", null) { ok, _, _, data ->
            if (ticket != generation || detailId != row.deviceId) return@request
            detailLoading = false
            if (!ok) {
                detailError = data?.text("error").orEmpty().ifEmpty { "详情读取失败" }
                return@request
            }
            val d = data ?: JSONObject()
            detailName = d.text("name")
            detailModel = d.text("model")
            detailOnline = d.optBoolean("online", false)
            applyDps(d)
        }
    }

    fun closeDetail() {
        leaveDetail()
    }

    private fun leaveDetail() {
        detailId = ""
        dps.clear()
        detailLoading = false
        detailError = ""
        dpMessage = ""
        numberDp = -1
    }

    private fun applyDps(data: JSONObject) {
        dps.clear()
        val arr = data.array("dps")
        if (arr != null) {
            for (i in 0 until arr.length()) {
                val d = arr.optJSONObject(i) ?: continue
                dps.add(
                    TuyaDpRow(
                        dp = d.optInt("dp", -1),
                        valueText = d.text("value_text"),
                        writable = d.optBoolean("writable", false),
                        control = d.text("control").ifEmpty { "readonly" },
                        rawValue = d.opt("value")?.toString() ?: "",
                    ),
                )
            }
        }
    }

    /** 下发一个 DP：布尔走 "true"/"false"；数值走 numberDraft。成功后用回包刷新 DP 快照。 */
    fun setDp(ctx: Pager, dp: Int, valueText: String, onDone: (ok: Boolean, msg: String) -> Unit) {
        if (dpBusy || detailId.isEmpty()) return
        val value: Any = when (valueText) {
            "true" -> true
            "false" -> false
            else -> valueText.toLongOrNull() ?: valueText.toDoubleOrNull() ?: valueText
        }
        dpBusy = true
        dpMessage = ""
        val ticket = ++generation
        val body = JSONObject().apply {
            put("dp", "$dp")
            put("value", value)
        }
        request(ctx, "POST", "$base/api/tuya/devices/${urlComponent(detailId)}/dp", body) { ok, _, _, data ->
            if (ticket != generation) return@request
            dpBusy = false
            if (ok) {
                applyDps(data ?: JSONObject())
                dpMessage = "已下发"
                numberDp = -1
                detailOnline = true
                onDone(true, "已下发 DP $dp")
            } else {
                val kind = data?.text("kind").orEmpty()
                val error = data?.text("error").orEmpty().ifEmpty { "下发失败" }
                val advice = data?.text("advice").orEmpty()
                dpMessage = if (advice.isEmpty()) error else "$error（建议：$advice）"
                onDone(false, classifyFailure(kind, error, advice))
            }
        }
    }

    /** 保存本地别名（不影响云端名称）。 */
    fun saveAlias(ctx: Pager, onDone: (ok: Boolean, msg: String) -> Unit) {
        if (dpBusy || detailId.isEmpty()) return
        dpBusy = true
        val ticket = ++generation
        val body = JSONObject().apply { put("alias", aliasDraft.trim()) }
        request(ctx, "POST", "$base/api/tuya/devices/${urlComponent(detailId)}/alias", body) { ok, _, _, data ->
            if (ticket != generation) return@request
            dpBusy = false
            if (ok) {
                detailName = aliasDraft.trim().ifEmpty { detailName }
                devices.firstOrNull { it.deviceId == detailId }?.name = detailName
                onDone(true, "别名已保存")
            } else {
                onDone(false, "保存失败：${data?.text("error").orEmpty().ifEmpty { "请稍后重试" }}")
            }
        }
    }

    // ============================================================
    // 请求底座（generation ticket + WS RPC）
    // ============================================================

    /**
     * 统一请求入口（走管理台 WS RPC，鉴权在连接升级时完成），回包凭 ticket 对号
     * ——对不上即丢弃（切服务器/重连后的迟到回包）。
     */
    private fun request(
        ctx: Pager,
        method: String,
        url: String,
        body: JSONObject?,
        onDone: (ok: Boolean, code: Int?, reason: String, data: JSONObject?) -> Unit,
    ) {
        val token = ctx.adminRequestToken()
        val access = ctx.adminAccessRevision()
        val ticket = generation
        rpcCall(ctx, url, method, body) { ok, data, status, errorMsg ->
            if (ticket != generation || !ctx.isAdminAccessCurrent(access, token)) return@rpcCall
            val reason = data.text("error").takeIf { it.isNotEmpty() } ?: errorMsg
            onDone(ok, status, reason, data)
        }
    }

    /** device_id 是涂鸦分配的字母数字串；只兜底编码 URL 不安全字符（commonMain 无 URLEncoder）。 */
    private fun urlComponent(value: String): String = buildString {
        for (ch in value) {
            when {
                ch.isLetterOrDigit() || ch == '-' || ch == '_' || ch == '.' -> append(ch)
                else -> append('%').append(ch.code.toString(16).padStart(2, '0'))
            }
        }
    }
}

/** 设备列表一行（字段为**不可变快照**——刷新时整表重建；name 可被本地别名更新）。 */
class TuyaDeviceRow(
    val deviceId: String,
    var name: String,
    val model: String,
    val online: Boolean,
    val ip: String,
    val updatedAt: Long,
) {
    val displayModel: String get() = model.ifEmpty { "型号未知" }
}

/** LAN 发现的未登记设备一行（status.lan_unregistered 投影；展示 label= id[@ip]）。 */
class TuyaLanDevice(
    val deviceId: String,
    val ip: String,
    val ageMs: Long,
) {
    val label: String get() = if (ip.isEmpty()) deviceId else "$deviceId @ $ip"
}

/** 详情页一条 DP（dp id + 服务端算好的展示值与控件形态）。 */
class TuyaDpRow(
    val dp: Int,
    val valueText: String,
    val writable: Boolean,
    /** switch / number / readonly（服务端按当前值类型给形态；readonly 表示不渲染控件）。 */
    val control: String,
    val rawValue: String,
)

/** 同步结果 → 中文（UI 状态条）。 */
internal fun resultLabel(result: String): String = when (result) {
    "ok" -> "成功"
    "partial" -> "部分成功"
    "cloud_auth_failed" -> "凭证失败"
    "network_error" -> "网络失败"
    "skipped" -> "已跳过"
    else -> result
}

/** 控制失败三归类 → 人话（服务端已给 advice，这里只兜底未知归类）。 */
internal fun classifyFailure(kind: String, error: String, advice: String): String = when (kind) {
    "offline" -> "设备离线：$error${if (advice.isEmpty()) "" else "（$advice）"}"
    "timeout" -> "设备无响应：$error${if (advice.isEmpty()) "" else "（$advice）"}"
    "unsupported" -> "协议不支持：$error${if (advice.isEmpty()) "" else "（$advice）"}"
    else -> if (advice.isEmpty()) error else "$error（$advice）"
}
