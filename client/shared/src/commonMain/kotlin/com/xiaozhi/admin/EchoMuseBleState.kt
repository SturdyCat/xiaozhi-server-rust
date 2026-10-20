package com.xiaozhi.admin

import com.tencent.kuikly.core.base.PagerScope
import com.tencent.kuikly.core.nvi.serialization.json.JSONObject
import com.tencent.kuikly.core.pager.Pager
import com.tencent.kuikly.core.reactive.handler.observable
import com.tencent.kuikly.core.reactive.handler.observableList

internal data class BleAdvertRow(val address: String, val addressType: Int?, val rssi: String, val seen: String, val data: String)
internal data class BleAttributeRow(
    val kind: String, val uuid: String, val label: String, val readWriteHandle: Int?, val raw: String,
)

/** Explicit snapshots only. Every connection/target change invalidates pending network callbacks. */
class EchoMuseBleState(scope: PagerScope) {
    var target by scope.observable("")
        private set
    private var base = ""
    private var token = ""
    private var generation = 0
    private var requestSerial = 0
    private var activeSerial = 0
    private var sessionId = ""
    private var approved = false
    private var online = false
    private var capable = false
    private var savedEnabled = true
    private var savedConnections = true
    var busy by scope.observable("")
        private set
    var error by scope.observable("")
        private set
    var result by scope.observable("")
        private set
    var settingsResult by scope.observable("")
        private set
    var status by scope.observable("")
        private set
    var enabledDraft by scope.observable(true)
        private set
    var connectionsDraft by scope.observable(true)
        private set
    var address by scope.observable("")
        private set
    var addressType by scope.observable(0)
        private set
    var handleDraft by scope.observable("")
        private set
    var hexDraft by scope.observable("")
        private set
    var responseDraft by scope.observable(true)
        private set
    internal var confirmed by scope.observable<BleWriteIntent?>(null)
        private set
    internal val adverts by scope.observableList<BleAdvertRow>()
    internal val attributes by scope.observableList<BleAttributeRow>()
    var advertsSummary by scope.observable("尚未读取广播快照")
        private set
    var slotsOutput by scope.observable("尚未读取槽位；GET state 中的槽位也只是最后一次上报")
        private set
    var eventsOutput by scope.observable("尚未读取事件；不会自动轮询")
        private set
    var gattOutput by scope.observable("")
        private set
    var readOutput by scope.observable("")
        private set

    // Observable eligibility fields are read through statusRevision in UI attr/enabled closures.
    private var statusRevision by scope.observable(0)
    val idle: Boolean get() = busy.isEmpty()
    val canRead: Boolean get() { statusRevision; return target.isNotEmpty() && approved && online && capable && sessionId.isNotEmpty() }
    val canGatt: Boolean get() { statusRevision; return canRead && savedEnabled && savedConnections }

    fun configureConnection(baseUrl: String, authToken: String) {
        if (base != baseUrl || token != authToken) {
            invalidate()
            target = ""
            sessionId = ""
            settingsResult = ""
            clearOutputs()
        }
        base = baseUrl
        token = authToken
    }

    /** 菜单选中语义：target 已是本设备则保留输出不重置，否则走 [toggle] 重置并登记目标。 */
    fun select(row: EchoMuseDeviceRow) {
        if (target != row.deviceId) toggle(row)
    }

    fun toggle(row: EchoMuseDeviceRow) {
        invalidate()
        target = if (target == row.deviceId) "" else row.deviceId
        settingsResult = ""
        clearOutputs()
        address = ""
        handleDraft = ""
        hexDraft = ""
        addressType = 0
        responseDraft = true
        enabledDraft = row.settings.optBoolean("bleProxyEnabled", true)
        connectionsDraft = row.settings.optBoolean("bleProxyConnections", true)
        sync(listOf(row))
    }

    fun sync(rows: List<EchoMuseDeviceRow>) {
        if (target.isEmpty()) return
        val row = rows.firstOrNull { it.deviceId == target }
        val nextApproved = row?.approved == true
        val nextOnline = row?.online == true
        val nextCapable = row?.capabilities?.contains("ble_connect") == true
        val nextSession = row?.sessionId.orEmpty()
        val nextEnabled = row?.settings?.optBoolean("bleProxyEnabled", true) ?: false
        val nextConnections = row?.settings?.optBoolean("bleProxyConnections", true) ?: false
        if (approved != nextApproved || online != nextOnline || capable != nextCapable || sessionId != nextSession ||
            savedEnabled != nextEnabled || savedConnections != nextConnections) {
            invalidate()
            clearOutputs()
        }
        approved = nextApproved
        online = nextOnline
        capable = nextCapable
        sessionId = nextSession
        savedEnabled = nextEnabled
        savedConnections = nextConnections
        statusRevision++
        status = when {
            !approved -> "设备尚未批准，蓝牙代理不可用"
            !online -> "设备离线：旧广播/GATT/事件已清除；请唤醒设备并刷新列表"
            !capable -> "设备未声明 ble_connect 能力，不能使用蓝牙代理"
            sessionId.isEmpty() -> "缺少当前 control session_id：请更新服务端并刷新列表；禁止危险写入"
            !savedEnabled || !savedConnections -> "每设备 BLE 开关尚未同时开启；可读取广播/状态，GATT 不可用"
            else -> "需服务端开启 echomuse.ble_proxy 并配置管理/设备令牌；在线仅指控制连接存在"
        }
        if (row == null) target = ""
    }

    private fun invalidate(resetBusy: Boolean = true) {
        generation++
        if (resetBusy) {
            activeSerial = 0
            busy = ""
        }
        clearConfirmation()
    }

    /** Draft edits cannot cancel a dispatched device operation or release its busy ownership. */
    private fun clearConfirmation() {
        confirmed = null
        error = ""
        result = ""
    }

    private fun clearOutputs() {
        adverts.clear()
        attributes.clear()
        advertsSummary = "尚未读取当前会话的广播快照"
        slotsOutput = "尚未读取当前会话的槽位"
        eventsOutput = "尚未读取当前会话的事件；仅手动刷新"
        gattOutput = ""
        readOutput = ""
    }

    fun setAddress(value: String) {
        if (address == value) return
        invalidate(resetBusy = false)
        address = value
        attributes.clear()
        gattOutput = ""
        readOutput = ""
    }
    fun setAddressType(value: Int) {
        if (value !in 0..1 || addressType == value) return
        invalidate(resetBusy = false)
        addressType = value
        attributes.clear()
        gattOutput = ""
        readOutput = ""
    }
    fun setHandle(value: String) { if (value != handleDraft) { clearConfirmation(); handleDraft = value } }
    fun setHex(value: String) { if (value != hexDraft) { clearConfirmation(); hexDraft = value } }
    fun toggleResponse() { clearConfirmation(); responseDraft = !responseDraft }
    fun toggleEnabled() { clearConfirmation(); enabledDraft = !enabledDraft }
    fun toggleConnections() { clearConfirmation(); connectionsDraft = !connectionsDraft }
    fun cancelWrite() { confirmed = null }

    internal fun selectAdvert(row: BleAdvertRow) {
        val addr = bleNormalizeAddress(row.address)
        if (addr == null || row.addressType !in listOf(0, 1)) {
            error = "广播的 MAC/地址类型不合法，不能选择；请核对原始数据"
            return
        }
        setAddress(addr)
        setAddressType(row.addressType!!)
    }

    fun readAdverts(ctx: Pager) {
        if (!canRead || !idle) return
        request(ctx, "ble/adverts") { data ->
            adverts.clear()
            val values = data.optJSONArray("adverts")
            if (values != null) for (i in 0 until values.length()) {
                val item = values.optJSONObject(i) ?: continue
                adverts.add(BleAdvertRow(item.text("addr"), bleStrictInt(item.opt("addrType"), 0, 1),
                    field(item, "rssi"), epoch(item, "seenAt"), field(item, "data")))
            }
            advertsSummary = "广播合并快照 ${adverts.size} 条；最近批次 ${epoch(data, "updatedAt")}；非实时扫描"
        }
    }

    fun readState(ctx: Pager) {
        if (!canRead || !idle) return
        request(ctx, "ble/state") { data ->
            slotsOutput = data.optJSONObject("slots")?.let(::slotsText) ?: "未收到槽位上报（未知，不代表0）"
            val events = data.optJSONArray("events")
            eventsOutput = if (events == null || events.length() == 0) "当前快照无事件；不代表外设从未通知/断开" else buildList {
                for (i in 0 until events.length()) {
                    val event = events.optJSONObject(i) ?: continue
                    add(event.toString())
                    if (event.text("t") == "notify") add("通知 hex: ${bleBase64ToHex(event.text("value")) ?: "非法 base64"}")
                }
            }.joinToString("\n")
            result = "状态快照已读取；仅保留服务端有界事件窗口，不是实时订阅或完整历史"
        }
    }

    fun querySlots(ctx: Pager) = gatt(ctx, "slots", withAddress = false) { slotsOutput = slotsText(it) }
    fun connect(ctx: Pager) = gatt(ctx, "connect") { result = "设备报告连接成功；请查询槽位核对" }
    fun disconnect(ctx: Pager) = gatt(ctx, "disconnect") {
        attributes.clear()
        confirmed = null
        result = "设备报告已断开；请查询槽位核对"
    }
    fun discover(ctx: Pager) = gatt(ctx, "services") { data -> parseServices(data) }
    fun read(ctx: Pager) {
        val handle = bleParseHandle(handleDraft) ?: return fail("handle 必须是十进制整数 1～65535")
        val capturedAddress = bleNormalizeAddress(address).orEmpty()
        gatt(ctx, "read", handle = handle) { data ->
            val value = data.opt("value") as? String
            val hex = value?.let(::bleBase64ToHex)
            if (value == null || hex == null) {
                readOutput = ""
                error = "设备GATT协议错误：read缺少字符串value或base64非法，请检查原始响应"
            } else {
                readOutput = "外设 $capturedAddress / handle $handle\nhex: $hex\nbase64: $value"
                result = "特征值已读取；字节含义依赖外设协议"
            }
        }
    }

    internal fun currentWrite(): BleWriteIntent? {
        if (!canGatt) return null
        return BleWriteIntent(target, sessionId, bleNormalizeAddress(address) ?: return null, addressType,
            bleParseHandle(handleDraft) ?: return null, bleHexToBase64(hexDraft) ?: return null, responseDraft)
    }

    fun prepareWrite() {
        if (!idle) return
        confirmed = currentWrite()
        error = if (confirmed == null) "无法写入：请核对在线会话、BLE配置、MAC、十进制handle及非空hex（最多512字节）" else ""
        result = ""
    }

    fun confirmWrite(ctx: Pager) {
        val intent = confirmed
        if (!idle || !bleWriteStillConfirmed(intent, currentWrite())) {
            confirmed = null
            fail("草稿或设备会话已变化，请重新检查并确认写入")
            return
        }
        confirmed = null
        val captured = intent ?: return
        val body = JSONObject().apply {
            put("t", "write"); put("addr", captured.address); put("handle", captured.handle)
            put("value", captured.valueBase64); put("response", captured.response)
        }
        request(ctx, "ble/gatt", body, gattType = "write") {
            result = if (captured.response) "设备报告 ATT 写入成功；不等于 IoT 业务动作已执行，请读回或核对外设" else
                "设备报告无响应写已发送；未获得外设 ATT 确认，更不代表 IoT 业务动作已执行"
        }
    }

    fun saveSettings(ctx: Pager, onSaved: () -> Unit) {
        if (!canRead || !idle) return
        val body = JSONObject().apply {
            put("bleProxyEnabled", enabledDraft)
            put("bleProxyConnections", connectionsDraft)
        }
        val enabled = enabledDraft
        val connections = connectionsDraft
        request(ctx, "settings", body, strictIdentity = false) {
            settingsResult = "上次保存：扫描=$enabled、连接=$connections；部分设置已保存/在线推送排队，不代表设备已应用。请查询槽位核对"
            onSaved()
        }
    }

    private fun gatt(ctx: Pager, type: String, withAddress: Boolean = true, handle: Int? = null, done: (JSONObject) -> Unit) {
        if (!canGatt || !idle) return
        val addr = if (withAddress) bleNormalizeAddress(address) ?: return fail("请填写有效 MAC，如 aa:bb:cc:dd:ee:ff") else null
        confirmed = null
        val body = JSONObject().apply {
            put("t", type)
            if (addr != null) put("addr", addr)
            if (type == "connect") put("addr_type", addressType)
            if (handle != null) put("handle", handle)
        }
        request(ctx, "ble/gatt", body, gattType = type, done = done)
    }

    private fun request(
        ctx: Pager, path: String, body: JSONObject? = null, gattType: String? = null,
        strictIdentity: Boolean = true, done: (JSONObject) -> Unit,
    ) {
        val requestGeneration = ++generation
        val serial = ++requestSerial
        activeSerial = serial
        val expectedDevice = target
        val expectedSession = sessionId
        busy = gattType ?: path
        error = ""
        result = ""
        confirmed = null
        if (strictIdentity && body != null) body.put("session_id", expectedSession)
        val suffix = if (body == null) "?session_id=${echoMuseUrlComponent(expectedSession)}" else ""
        network(ctx).httpRequest(
            "$base/api/echomuse/devices/${echoMuseUrlComponent(expectedDevice)}/$path$suffix",
            body != null, body ?: JSONObject(), echoMuseHeaders(token), timeout = 45,
        ) { data, success, errorMsg, response ->
            // Only this request may release its busy slot; a target edit does not cancel I/O.
            if (activeSerial != serial) return@httpRequest
            activeSerial = 0
            busy = ""
            if (generation != requestGeneration || target != expectedDevice || sessionId != expectedSession) return@httpRequest
            val code = response.statusCode
            if (!success || (code != null && code !in 200..299)) {
                error = when (code) {
                    401 -> "管理鉴权失败，请确认URL编码后的管理令牌"
                    403 -> "设备未批准或会话未认证，请刷新列表"
                    404 -> "设备/BLE接口不存在：请开启 echomuse.enabled 与 ble_proxy 并确认服务端版本"
                    409 -> "会话已变化、设备离线、BLE设置未启用或data链路不可用；刷新列表后重新操作"
                    422 -> "设备未声明 ble_connect 能力，或参数不受支持"
                    429 -> "设备已有 BLE 请求正在执行，请等待后重试"
                    503 -> "BLE代理需非空 server.expected_token 与 echomuse.token，并开启 ble_proxy"
                    504 -> "BLE请求超时，未确认执行结果；先查询槽位/读回，勿盲目重复写入"
                    else -> "BLE请求失败 HTTP $code：${data.text("error").ifEmpty { errorMsg }}"
                }
                return@httpRequest
            }
            if (strictIdentity && !bleSameIdentity(expectedDevice, expectedSession, data.text("device_id"), data.text("session_id"))) {
                clearOutputs()
                error = "BLE响应设备/会话不匹配（含旧服务端缺字段）；已丢弃，请刷新列表后重试"
                return@httpRequest
            }
            if (gattType != null) {
                gattOutput = data.toString()
                val ok = data.opt("ok") as? Boolean
                val expectedType = if (gattType == "slots") "slots" else "result"
                if (data.text("t") != expectedType || !bleGattSucceeded(data.text("t"), ok)) {
                    error = "设备GATT失败：error=${field(data, "error")}；att=${field(data, "att")}；detail=${field(data, "detail")}"
                    return@httpRequest
                }
            }
            done(data)
        }
    }

    private fun parseServices(data: JSONObject) {
        attributes.clear()
        val services = data.optJSONArray("services")
        if (services != null) for (i in 0 until services.length()) {
            val service = services.optJSONObject(i) ?: continue
            attributes.add(BleAttributeRow("service", service.text("uuid"),
                "Service 范围 ${field(service, "start")}～${field(service, "end")}", null, service.toString()))
            val chars = service.optJSONArray("chars") ?: continue
            for (j in 0 until chars.length()) {
                val ch = chars.optJSONObject(j) ?: continue
                val valueHandle = bleStrictInt(ch.opt("value_handle"), 1, 65535)
                val props = bleStrictInt(ch.opt("props"), 0, 255)
                attributes.add(BleAttributeRow("characteristic", ch.text("uuid"),
                    "声明 handle ${field(ch, "handle")}；读写 value_handle ${field(ch, "value_handle")}\nprops ${props ?: "未知"}：${props?.let(::bleProperties) ?: "未知"}", valueHandle, ch.toString()))
                val descs = ch.optJSONArray("descs") ?: continue
                for (k in 0 until descs.length()) {
                    val desc = descs.optJSONObject(k) ?: continue
                    val h = bleStrictInt(desc.opt("handle"), 1, 65535)
                    attributes.add(BleAttributeRow("descriptor", desc.text("uuid"), "描述符 handle ${field(desc, "handle")}", h, desc.toString()))
                }
            }
        }
        result = if (attributes.isEmpty()) "服务发现完成：设备未返回任何服务" else "服务发现完成：使用 value_handle 读写特征，描述符使用其 handle"
    }

    private fun slotsText(data: JSONObject): String =
        "槽位：free=${field(data, "free")} / limit=${field(data, "limit")}\naddrs=${field(data, "addrs")}\n原始：$data"
    private fun fail(message: String) { error = message }
    private fun field(data: JSONObject, key: String): String = data.opt(key)?.toString()?.takeUnless { it == "null" } ?: "未知"
    private fun epoch(data: JSONObject, key: String): String =
        (data.opt(key) as? Number)?.toLong()?.let(::formatEpochSeconds)?.ifEmpty { "未知" } ?: "未知"
}
