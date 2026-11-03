package com.xiaozhi.admin


import com.tencent.kuikly.core.base.PagerScope
import com.tencent.kuikly.core.module.NetworkModule
import com.tencent.kuikly.core.nvi.serialization.json.JSONArray
import com.tencent.kuikly.core.nvi.serialization.json.JSONObject
import com.tencent.kuikly.core.pager.Pager
import com.xiaozhi.admin.state.stateVar
import com.xiaozhi.admin.state.stateListOf

/**
 * 管理台「概览 / 设备」页面的数据源。
 *
 * 对应服务端端点：`GET /api/stats`、`/api/devices`，外加设备注入
 * `POST /api/devices/{id}/inject`（[DevicesState.inject]）。
 * （2026-10-29 范围削减：会话页的 `SessionRow` / `SessionsState` 与对话/用量/延迟三个
 * state 类已随 docs/plans/gearui-kit-scope-reduction.md §二 移除。）
 * 数据源是**进程内注册表**（`crate::app::registry`）：全部为"自本次服务启动以来"的累计值，
 * 重启清零、在线态由会话生命周期派生。这个口径由服务端随响应下发（`note` / `scope_note`），
 * 页面**直接展示服务端的原话**——客户端不自己编一句"今日统计"，那会与事实不符。
 *
 * 设计取舍（与 [McpToolsState] 完全一致，因此读代码时可比照）：
 * - **只读投影**：这些数据源都不参与配置表单，绝不随 `POST /api/config` 回传；
 * - **失败就地显示**：首次加载失败 → `loadError` + 「刷新」按钮，不弹 toast（toast 只用于
 *   用户动作的结果）；
 * - **空列表给可行动的解释**：用服务端 `note`，并说清"怎么样才会有数据"；
 * - **时间用服务端给的相对毫秒**（`age_ms` / `idle_ms` / `duration_ms`）：客户端不必知道
 *   "现在几点"，也就不会因浏览器时钟漂移把"3 分钟前"显示成"3 小时前"。
 *
 * ⚠️ 这些类**没有** `@Page`，也不直接渲染：渲染见 `OpsPages.kt`，页面壳体见 `AdminPage.kt`。
 * 拆成三个文件是 `AGENTS.md` §5.10 的文件规模约定（单文件 ≤ 600 行）。
 */

// ============================================================
// 公共：网络、JSON 取值、人话格式化
// ============================================================

/**
 * 全 admin 包共用的 Module 获取入口（原为 OpsState.kt 文件级 private，因 7 个 State 类各持
 * 一份逐字相同的实现而统一至此）：`network(ctx).requestGet(...)` 即可发起请求。
 * 各文件同包免 import；OtaState 的 token/authQuery 逻辑与 [getJson] 不同型，仍在类内自持。
 *
 * ⚠️ 管理台 API 通讯**始终走 WS**（大文件传输除外）：直连 HTTP 的 `requestGet`/
 * `requestPost` 仅剩大文件上传/下载等场景，常规 JSON API 一律经 [adminTransport]。
 */
internal fun network(ctx: Pager): NetworkModule = ctx.acquireModule(NetworkModule.MODULE_NAME)

/**
 * 管理台 WS RPC 共享传输层（包级单例）：`AdminRealtime` 按 base|token 分组共享连接，
 * 跨页面复用同一条 WS（订阅与 RPC 混用同组）。管理台与服务器之间的 JSON API 全部经它。
 */
internal val adminTransport = AdminRealtime()

/**
 * 从完整 url 剥出 `(base, path)`：path 取 `/api/` 起始段（RPC 服务端仅接受 API 前缀；
 * path 可带查询参数，axum 侧按完整 URI 匹配）。web 同域时 base 为空串。
 */
internal fun splitAdminUrl(url: String): Pair<String, String> {
    val idx = url.indexOf("/api/")
    if (idx < 0) return "" to url
    return url.substring(0, idx) to url.substring(idx)
}

/**
 * 管理台页面的鉴权 token（URL 参数带入，与 [AdminPage.refreshAll] 同源）。
 * token 为空 = 服务端未设管理令牌。
 */
internal fun adminToken(ctx: Pager): String = ctx.pagerData.params.optString("token", "")

/**
 * 只读端点的通用 GET：成功回调 `onDone(true, data, "")`，失败回调 `onDone(false, data, 原因)`。
 *
 * **走管理台 WS RPC**（[adminTransport]，`GET /api/admin/ws` 的 rpc 帧）：与服务端
 * REST 路由同一批 handler（服务端 tower oneshot 复用），错误口径与 REST 逐字一致
 * ——404 的成因说明由 [AdminRealtime] 的回执层给出（"端点不存在/未启用对应功能"），
 * 非 HTTP 失败（断线/超时）reason 为可读中文。
 *
 * 五个数据源共用同一套错误口径，避免各写一份后行为漂移——"有的页面失败有提示、
 * 有的页面永远转圈"正是这么来的。
 *
 * ⚠️ 注释里不要出现斜杠加星号的组合（Kotlin 块注释可嵌套，会吞掉后面整个文件）。
 */
internal fun getJson(
    ctx: Pager,
    url: String,
    onDone: (ok: Boolean, data: JSONObject, message: String) -> Unit,
) {
    val (base, path) = splitAdminUrl(url)
    adminTransport.rpc(ctx, base, adminToken(ctx), "GET", path, null) { ok, data, _, message ->
        onDone(ok, data, message)
    }
}

/**
 * 通用 JSON RPC（GET/POST/PUT/DELETE）：[getJson] 的全方法版，供散点写操作
 * （配置保存、preset、memory、EchoMuse、NLU、测试台等）从 `network(ctx).httpRequest`
 * 切到 WS 用。口径与 REST 一致：`ok` = HTTP 2xx，`status` 透传状态码（404/405
 * 语义保留），`message` = 可读失败原因（非 HTTP 失败 status=0）。
 *
 * @param body 仅写方法携带（服务端要求写方法必须带 JSON 对象 body，空对象 `{}` 也要传）；
 *   GET 传 null。
 * @param timeoutMs 本地超时（默认 30s）；长端点（NLU 模型测试、节点测试）传 60_000——
 *   帧会带上服务端执行超时（本地 + 2s），无需服务端配合。
 */
internal fun rpcCall(
    ctx: Pager,
    url: String,
    method: String,
    body: JSONObject?,
    timeoutMs: Int = RT_RPC_TIMEOUT_MS,
    onDone: (ok: Boolean, data: JSONObject, status: Int, message: String) -> Unit,
) {
    val (base, path) = splitAdminUrl(url)
    adminTransport.rpc(ctx, base, adminToken(ctx), method, path, body, timeoutMs, onDone)
}

/**
 * 只读数据源的公共加载模板（基类）。
 *
 * StatsState / DevicesState / McpToolsState / PluginMetaState 等类此前各自持有逐字相同的同构骨架
 *（SessionsState / UsageState / ConversationsState 也曾共用，2026-10-29 随范围削减移除）：
 * `loading/loaded/loadError + base + load(){if(loading)return;…} + refresh`。收敛到本基类后，
 * 子类只需声明三个差异点：
 * - [endpoint]：请求 URL（含查询参数）；
 * - [errorLabel]：失败就地文案前缀（最终展示 `"$errorLabel: $message"`）；
 * - [apply]：成功回包 → 本类字段的投影（子类在 apply 内置 `loaded = true; loadError = ""`）。
 *
 * 语义与收敛前逐字一致：进页先 `if (loading) return` 防重入；失败只置 `loadError`
 * （就地错误 + 「刷新」按钮，不弹 toast）。
 *
 * ⚠️ Kuikly observable 与 PagerScope 绑定：scope 由**每个子类实例**各自持有并传给基类，
 * 基类不引入任何跨 Pager / 跨页面共享状态。
 * ⚠️ [OtaState] **不**继承本类：它多 token 鉴权（authQuery）与上传/删除写动作，
 * load 签名（authToken 参数）与错误口径也不同型——硬套基类会虚增抽象层，保持自持。
 */
abstract class LoadableState(private val scope: PagerScope) {

    var loading by stateVar(false)
    var loaded by stateVar(false)
    var loadError by stateVar("")

    /** 上次使用的服务端基址：[refresh] 复用（web 同域为空串，macApp 连接流程传绝对地址）。 */
    protected var base = ""

    /** 请求 URL（相对 baseUrl 的完整路径，含查询参数）。 */
    protected abstract fun endpoint(baseUrl: String): String

    /** 失败就地文案前缀（如 "设备列表加载失败"；最终展示 `"$errorLabel: $message"`）。 */
    protected abstract val errorLabel: String

    /** 成功后把回包投影到本类字段（子类各自实现）。 */
    protected abstract fun apply(data: JSONObject)

    /** 拉取数据。失败只置 `loadError`（就地错误 + 「刷新」按钮，不弹 toast）。 */
    fun load(ctx: Pager, baseUrl: String = "") {
        if (loading) return
        base = baseUrl
        loading = true
        loadError = ""
        getJson(ctx, endpoint(baseUrl)) { ok, data, message ->
            loading = false
            if (!ok) {
                loadError = "$errorLabel: $message"
                return@getJson
            }
            apply(data)
        }
    }

    /** 按上次的基址重新拉取（页内「刷新」按钮 / 数据动作成功后）。 */
    fun refresh(ctx: Pager) = load(ctx, base)
}

/**
 * 取字符串字段，把 JSON `null` 归一成空串。
 *
 * 服务端会把"设备从未 MCP 连接过"的 `board` / `firmware_version` 显式序列化成 `null`；
 * Kuikly 的 JSON 实现在该键存在但值为 null 时可能回字面量 `"null"`。不归一的话界面上会
 * 出现一个写着 "null" 的板型字段，看起来像坏数据。
 */
internal fun JSONObject.text(key: String): String =
    optString(key, "").let { if (it == "null") "" else it }

/** 取数组字段（缺省返回 null，调用方自行决定"空 vs 缺失"）。 */
internal fun JSONObject.array(key: String): JSONArray? = optJSONArray(key)

/**
 * 相对毫秒 → 人话（"刚刚 / 3 分钟前 / 2 小时前"）。
 *
 * 与 `McpToolsState` 内那份**刻意重复**：那一份是 private、语义绑定"探测时间"；
 * 这一份是五个页面的公共展示口径。两份都只是 6 行 when，合并不值得引入一个共享模块。
 */
internal fun humanAge(ms: Long): String = when {
    ms < 60_000L -> "刚刚"
    ms < 3_600_000L -> "${ms / 60_000L} 分钟前"
    ms < 86_400_000L -> "${ms / 3_600_000L} 小时前"
    else -> "${ms / 86_400_000L} 天前"
}

/** 时长（毫秒）→ "12 秒 / 3 分 20 秒 / 1 小时 5 分"。用于会话持续时长与进程运行时长。 */
internal fun humanDuration(ms: Long): String {
    val totalSec = ms / 1000L
    val h = totalSec / 3600L
    val m = (totalSec % 3600L) / 60L
    val s = totalSec % 60L
    return when {
        h > 0 -> "$h 小时 $m 分"
        m > 0 -> "$m 分 $s 秒"
        else -> "$s 秒"
    }
}

// ============================================================
// 概览 / 看板
// ============================================================

/** 令牌用量累计（`TokenUsage::to_json` 的投影）。 */
class UsageTotals(
    /** 计量的轮次数（上游返回 usage 的轮次）。 */
    val turns: Long,
    val input: Long,
    val output: Long,
    val cacheRead: Long,
    val cacheWrite: Long,
    /** 上下文压力 = input + cache_read + cache_write（"推给模型的上下文"规模）。 */
    val pressure: Long,
    val total: Long,
) {
    /** 一句话摘要；全零时返回空串（页面据此走"还没有数据"分支）。 */
    val summary: String
        get() = if (total == 0L) {
            ""
        } else {
            "输入 $input · 输出 $output · 缓存读 $cacheRead · 缓存写 $cacheWrite · 合计 $total"
        }
}

/** `GET /api/stats` 的快照。
 *
 * ⚠️ 这里**刻意不解析** `capabilities`：能力清单已由 [PluginMetaState]（`GET /api/plugins`）
 * 提供，且字段更全（实现展示名 / 生效语义文案 / 降级原因）。同一份事实投影两次，
 * 只会让"能力总览"卡在两个页面上长得不一样——概览页因此直接复用 `overviewCard(meta, ctx)`。
 */
class StatsState(scope: PagerScope) : LoadableState(scope) {

    var uptimeText by stateVar("")
    var devicesTotal by stateVar(0)
    var devicesOnline by stateVar(0)
    var sessionsLive by stateVar(0)
    var sessionsTest by stateVar(0)
    var turnsUser by stateVar(0)
    var turnsAssistant by stateVar(0)
    var conversationsRetained by stateVar(0)
    var usageSummary by stateVar("")
    /** 已计量的轮次数（`usage.turns`）——与"缓冲内轮次"不同：那个受环形上界影响，这个是累计。 */
    var usageTurns by stateVar(0L)
    var scopeNote by stateVar("")
    var version by stateVar("")

    /** 计数摘要（只含非零项）：顶栏一行就能看到"现在什么状态"。 */
    var summary by stateVar("")

    override fun endpoint(baseUrl: String): String = "$baseUrl/api/stats"

    override val errorLabel: String = "看板数据加载失败"

    override fun apply(data: JSONObject) {
        version = data.text("version")
        scopeNote = data.text("scope_note")
        uptimeText = humanDuration(data.optLong("uptime_ms") ?: 0L)

        val devices = data.optJSONObject("devices")
        devicesTotal = devices?.optInt("total", 0) ?: 0
        devicesOnline = devices?.optInt("online", 0) ?: 0

        val sessions = data.optJSONObject("sessions")
        sessionsLive = sessions?.optInt("live", 0) ?: 0
        sessionsTest = sessions?.optInt("test", 0) ?: 0

        val turns = data.optJSONObject("turns")
        turnsUser = turns?.optInt("user", 0) ?: 0
        turnsAssistant = turns?.optInt("assistant", 0) ?: 0

        conversationsRetained = data.optJSONObject("conversations")?.optInt("retained", 0) ?: 0

        usageSummary = data.optJSONObject("usage")?.let { u ->
            usageTurns = u.optLong("turns") ?: 0L
            UsageTotals(
                turns = u.optLong("turns") ?: 0L,
                input = u.optJSONObject("usage")?.optLong("input") ?: 0L,
                output = u.optJSONObject("usage")?.optLong("output") ?: 0L,
                cacheRead = u.optJSONObject("usage")?.optLong("cache_read") ?: 0L,
                cacheWrite = u.optJSONObject("usage")?.optLong("cache_write") ?: 0L,
                pressure = u.optJSONObject("usage")?.optLong("pressure") ?: 0L,
                total = u.optJSONObject("usage")?.optLong("total") ?: 0L,
            ).summary
        } ?: ""

        val parts = mutableListOf<String>()
        parts.add("在线设备 $devicesOnline / 见过 $devicesTotal")
        parts.add("活会话 $sessionsLive")
        if (turnsUser + turnsAssistant > 0) parts.add("缓冲内轮次 ${turnsUser + turnsAssistant}")
        if (usageSummary.isNotEmpty()) parts.add("已计量轮次 ${data.optJSONObject("usage")?.optLong("turns") ?: 0L}")
        summary = "自启动 ${uptimeText} · " + parts.joinToString(" · ")
        loaded = true
        loadError = ""
    }
}

/**
 * 服务器出口 IP 与地理信息（`GET /api/server/geo` + 管理台 WS 实时推送，只读快照）。
 *
 * 服务端启动时解析一次出口 IP：仅**国内**出口写入默认地区（天气等定位功能的回退值）；
 * 境外出口（翻墙/代理）**只展示不采用**。三种状态都在界面上说清：
 *
 * - `resolved && domestic`：IP + 地区 + 来源，并注明"已作为默认地区"；
 * - `resolved && !domestic`：IP + 地区照常显示，但标注境外、未写入默认定位；
 * - `!resolved`：显示服务端给的原因（[note]；空串 = 启动解析尚未完成）。
 *
 * 除首次 HTTP 拉取外，管理台 WS（[AdminRealtime]）会推两路更新（[applyRealtime]）：
 * 连接首帧 `hello` 内嵌 `server_status` 快照（首连即有数据），出口 IP 解析完成时推
 * `server_status_changed`——**不需要手动刷新**，「刷新」按钮只是兜底。
 */
class GeoState(scope: PagerScope) : LoadableState(scope) {

    /** 是否收到过管理台 WS 的实时快照（hello 或 server_status_changed）。 */
    var liveApplied by stateVar(false)

    /** 服务端是否成功解析过出口 IP（含境外）。 */
    var resolved by stateVar(false)

    /** 是否为国内出口（仅 `true` 才写入了默认地区）。 */
    var domestic by stateVar(false)

    /** 出口 IP（境外解析成功时也会给出）。 */
    var ip by stateVar("")

    /** "国家 省 市" 拼接的地区文本（缺项自动省略）。 */
    var region by stateVar("")

    /** 数据来源（如 `myip.ipip.net`）。 */
    var source by stateVar("")

    /** 未成功定位的原因（`resolved=false` 时服务端给的解释；空串 = 尚未完成解析）。 */
    var note by stateVar("")

    /** 服务端下发的口径说明（展示原样，避免两端文案漂移）。 */
    var scopeNote by stateVar("")

    override fun endpoint(baseUrl: String): String = "$baseUrl/api/server/geo"

    override val errorLabel: String = "出口定位加载失败"

    override fun apply(data: JSONObject) {
        applySnapshot(data)
        loaded = true
        loadError = ""
    }

    /**
     * 应用管理台 WS 的服务器状态帧（hello 的 `server_status` 或 `server_status_changed`
     * 的 `status`，两者同形：`schema_version` + `geo` 段）。找不到 geo 段的帧（旧服务端
     * 的裸 hello）直接忽略——不碰现有状态，避免把 Geo 卡刷成空白。
     */
    fun applyRealtime(frame: JSONObject) {
        val payload = frame.optJSONObject("status")
            ?: frame.optJSONObject("server_status")
            ?: frame.optJSONObject("state")
            ?: frame.optJSONObject("server")
            ?: frame.optJSONObject("data")
            ?: frame
        val geo = payload.optJSONObject("geo")
            ?: payload.optJSONObject("server_geo")
            ?: payload.optJSONObject("serverGeo")
        when {
            geo != null -> applySnapshot(geo)
            hasGeoFields(payload) -> applySnapshot(payload)
            else -> return
        }
        liveApplied = true
        loaded = true
        loadError = ""
    }

    private fun applySnapshot(data: JSONObject) {
        val hasResolved = data.opt("resolved") != null
        if (hasResolved) resolved = data.optBoolean("resolved", resolved)
        data.text("scope_note").takeIf { it.isNotEmpty() }?.let { scopeNote = it }
        if (hasResolved && !resolved) {
            // 服务端明确"未解析"（重启后首连 / 解析未完成 / 失败落 note）：
            // 清掉旧定位，不展示上一进程的残留；note 以本次快照为准（可为空 = 解析中）。
            ip = ""
            region = ""
            source = ""
            domestic = false
            note = data.text("note")
            return
        }
        data.text("ip").takeIf { it.isNotEmpty() }?.let { ip = it }
        if (data.opt("domestic") != null) domestic = data.optBoolean("domestic", domestic)
        val nextRegion = listOfNotNull(
            data.text("country").ifEmpty { null },
            data.text("province").ifEmpty { null },
            data.text("city").ifEmpty { null },
        ).joinToString(" ")
        if (nextRegion.isNotEmpty()) region = nextRegion
        data.text("source").takeIf { it.isNotEmpty() }?.let { source = it }
        data.text("note").takeIf { it.isNotEmpty() }?.let { note = it }
        data.text("scope_note").takeIf { it.isNotEmpty() }?.let { scopeNote = it }
    }

    private fun hasGeoFields(data: JSONObject): Boolean =
        data.opt("resolved") != null || data.text("ip").isNotEmpty() ||
            data.text("country").isNotEmpty() || data.text("province").isNotEmpty() ||
            data.text("city").isNotEmpty()
}

// ============================================================
// 设备
// ============================================================

/** 一台设备的快照（`GET /api/devices` 的一行）。字段全部不可变——刷新时整表重建。 */
class DeviceRow(
    val deviceId: String,
    val clientId: String,
    val online: Boolean,
    /** 距最后活跃多久（"3 分钟前"）。 */
    val ageText: String,
    val sessionCount: Long,
    val turnCount: Long,
    val liveSessions: Int,
    val isTest: Boolean,
    val mcp: Boolean,
    val lastError: String,
    /** 板型（MCP 探测得来；无探测为空串）。 */
    val board: String,
    val firmwareVersion: String,
    val uplinkSr: Int,
    val downlinkSr: Int,
    val frameMs: Int,
    val binVersion: Int,
) {
    /** 副标题：板型 · 固件 · 协商参数。缺失项自动省略。 */
    val subtitle: String
        get() = listOfNotNull(
            board.ifEmpty { null },
            firmwareVersion.takeIf { it.isNotEmpty() }?.let { "固件 $it" },
            "上行 ${uplinkSr / 1000}k / 下行 ${downlinkSr / 1000}k·${frameMs}ms",
            "协议 v$binVersion",
        ).joinToString(" · ")

    val activity: String
        get() = "会话 $sessionCount 次 · 轮次 $turnCount · 最后活跃 $ageText"
}

/** `GET /api/devices` 的数据源。 */
class DevicesState(scope: PagerScope) : LoadableState(scope) {

    var note by stateVar("")
    var summary by stateVar("")

    val devices: MutableList<DeviceRow> = stateListOf()

    override fun endpoint(baseUrl: String): String = "$baseUrl/api/devices"

    override val errorLabel: String = "设备列表加载失败"

    override fun apply(data: JSONObject) {
        note = data.text("note")
        devices.clear()
        val arr = data.array("devices")
        if (arr == null) {
            loadError = "设备列表为空（服务端返回异常）"
            return
        }
        var online = 0
        for (i in 0 until arr.length()) {
            val d = arr.optJSONObject(i) ?: continue
            val audio = d.optJSONObject("audio")
            val row = DeviceRow(
                deviceId = d.text("device_id"),
                clientId = d.text("client_id"),
                online = d.optBoolean("online", false),
                ageText = humanAge(d.optLong("age_ms") ?: 0L),
                sessionCount = d.optLong("session_count") ?: 0L,
                turnCount = d.optLong("turn_count") ?: 0L,
                liveSessions = d.optInt("live_sessions", 0),
                isTest = d.optBoolean("is_test", false),
                mcp = d.optBoolean("mcp", false),
                lastError = d.text("last_error"),
                board = d.text("board"),
                firmwareVersion = d.text("firmware_version"),
                uplinkSr = audio?.optInt("uplink_sample_rate", 0) ?: 0,
                downlinkSr = audio?.optInt("downlink_sample_rate", 0) ?: 0,
                frameMs = audio?.optInt("downlink_frame_duration_ms", 0) ?: 0,
                binVersion = audio?.optInt("binary_protocol_version", 0) ?: 0,
            )
            devices.add(row)
            if (row.online) online++
        }
        summary = "共 ${devices.size} 台 · $online 台在线"
        loaded = true
        loadError = ""
    }

    // ============================================================
    // 设备消息注入（P1-2：POST /api/devices/{id}/inject）
    // ============================================================

    /** 当前展开注入输入的设备号（空串 = 全部收起；一次只展开一台，避免一排输入框）。 */
    var injectTarget by stateVar("")

    /** 注入文本草稿。 */
    var injectDraft by stateVar("")

    /** 注入请求进行中。 */
    var injectBusy by stateVar(false)

    /** 上次注入的就地结果（成功提示或失败原因；空 = 无）。 */
    var injectMsg by stateVar("")

    /** 展开/收起某台设备的注入输入（切换目标时清空草稿与结果）。 */
    fun toggleInject(deviceId: String) {
        if (injectTarget == deviceId) {
            injectTarget = ""
        } else {
            injectTarget = deviceId
            injectDraft = ""
            injectMsg = ""
        }
    }

    /**
     * 注入一段文本：服务端把它**作为用户话语**走完整 LLM→TTS→下行管线（不经 ASR），
     * 用于直接验证上游链路。设备必须在线且有可注入会话（测试台/AIUI 会话服务端会拒绝）。
     */
    fun inject(ctx: Pager, deviceId: String) {
        val text = injectDraft.trim()
        if (text.isEmpty()) {
            injectMsg = "请先输入要注入的文本"
            return
        }
        if (injectBusy) return
        injectBusy = true
        injectMsg = ""
        val body = JSONObject().apply { put("text", text) }
        adminTransport.rpc(
            ctx, base, adminToken(ctx), "POST", "/api/devices/$deviceId/inject", body,
        ) { ok, data, status, message ->
            val reason = data.text("message").takeIf { it.isNotEmpty() } ?: message
            if (ok) {
                injectDraft = ""
                injectMsg = "✓ 已注入，设备正在走完整回复管线"
            } else {
                injectMsg = when (status) {
                    404 -> "设备不在线：请先唤醒设备（建立会话）再注入"
                    409 -> reason.ifEmpty { "会话不支持注入或正忙，请稍后重试" }
                    else -> "注入失败：${reason.ifEmpty { "HTTP $status" }}"
                }
            }
            injectBusy = false
        }
    }
}

// ============================================================
// 会话（2026-10-29 范围削减：SessionRow / SessionsState 已移除，
// 会话页随 docs/plans/gearui-kit-scope-reduction.md §二 砍除）
// ============================================================

