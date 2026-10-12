package com.xiaozhi.admin

import com.tencent.kuikly.core.base.PagerScope
import com.tencent.kuikly.core.module.NetworkModule
import com.tencent.kuikly.core.nvi.serialization.json.JSONArray
import com.tencent.kuikly.core.nvi.serialization.json.JSONObject
import com.tencent.kuikly.core.pager.Pager
import com.tencent.kuikly.core.reactive.collection.ObservableList
import com.tencent.kuikly.core.reactive.handler.observable
import com.tencent.kuikly.core.reactive.handler.observableList

/**
 * 管理台「概览 / 设备 / 会话 / 对话记录 / 用量」五个页面的数据源。
 *
 * 对应服务端端点：`GET /api/stats`、`/api/devices`、`/api/sessions`、`/api/conversations`、`/api/usage`。
 * 数据源是**进程内注册表**（`crate::app::registry`）：全部为"自本次服务启动以来"的累计值，
 * 重启清零、在线态由会话生命周期派生。这个口径由服务端随响应下发（`note` / `scope_note`），
 * 页面**直接展示服务端的原话**——客户端不自己编一句"今日统计"，那会与事实不符。
 *
 * 设计取舍（与 [McpToolsState] 完全一致，因此读代码时可比照）：
 * - **只读投影**：五个数据源都不参与配置表单，绝不随 `POST /api/config` 回传；
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
 */
internal fun network(ctx: Pager): NetworkModule = ctx.acquireModule(NetworkModule.MODULE_NAME)

/**
 * 只读端点的通用 GET：成功回调 `onDone(true, data, "")`，失败回调 `onDone(false, data, 原因)`。
 *
 * 五个数据源共用同一套错误口径（HTTP 状态码 + 网络错误），避免各写一份后行为漂移
 * ——"有的页面失败有提示、有的页面永远转圈"正是这么来的。
 */
internal fun getJson(
    ctx: Pager,
    url: String,
    onDone: (ok: Boolean, data: JSONObject, message: String) -> Unit,
) {
    network(ctx).requestGet(url, JSONObject()) { data, success, errorMsg, resp ->
        val code = resp.statusCode
        val ok = success && (code == null || code in 200..299)
        onDone(ok, data, if (ok) "" else (if (errorMsg.isNotEmpty()) errorMsg else "HTTP $code"))
    }
}

/**
 * 只读数据源的公共加载模板（基类）。
 *
 * StatsState / DevicesState / SessionsState / McpToolsState / PluginMetaState /
 * UsageState / ConversationsState 七个类此前各自持有逐字相同的同构骨架：
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

    var loading by scope.observable(false)
    var loaded by scope.observable(false)
    var loadError by scope.observable("")

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

    var uptimeText by scope.observable("")
    var devicesTotal by scope.observable(0)
    var devicesOnline by scope.observable(0)
    var sessionsLive by scope.observable(0)
    var sessionsTest by scope.observable(0)
    var turnsUser by scope.observable(0)
    var turnsAssistant by scope.observable(0)
    var conversationsRetained by scope.observable(0)
    var usageSummary by scope.observable("")
    /** 已计量的轮次数（`usage.turns`）——与"缓冲内轮次"不同：那个受环形上界影响，这个是累计。 */
    var usageTurns by scope.observable(0L)
    var scopeNote by scope.observable("")
    var version by scope.observable("")

    /** 计数摘要（只含非零项）：顶栏一行就能看到"现在什么状态"。 */
    var summary by scope.observable("")

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

    var note by scope.observable("")
    var summary by scope.observable("")

    val devices: ObservableList<DeviceRow> by scope.observableList()

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
}

// ============================================================
// 会话
// ============================================================

/** 一条活会话（`GET /api/sessions` 的一行）。 */
class SessionRow(
    val sessionId: String,
    val deviceId: String,
    val isTest: Boolean,
    val mcp: Boolean,
    val phase: String,
    val phaseText: String,
    val durationText: String,
    val idleText: String,
    val turnCount: Long,
    val lastUserText: String,
    val lastError: String,
) {
    /** 阶段 → [statusBadge] 状态词：聆听/思考/说话都是"正在忙"，空闲走中性。 */
    val badgeState: String
        get() = when (phase) {
            "listening" -> "recording"
            "thinking" -> "busy"
            "speaking" -> "connected"
            else -> "idle"
        }

    val subtitle: String
        get() = buildString {
            append("连接 ${durationText} · 空闲 $idleText · 轮次 $turnCount")
            if (mcp) append(" · MCP")
        }
}

/** `GET /api/sessions` 的数据源。 */
class SessionsState(scope: PagerScope) : LoadableState(scope) {

    var note by scope.observable("")
    var summary by scope.observable("")

    val sessions: ObservableList<SessionRow> by scope.observableList()

    override fun endpoint(baseUrl: String): String = "$baseUrl/api/sessions"

    override val errorLabel: String = "会话列表加载失败"

    override fun apply(data: JSONObject) {
        note = data.text("note")
        sessions.clear()
        val arr = data.array("sessions")
        if (arr == null) {
            loadError = "会话列表为空（服务端返回异常）"
            return
        }
        var test = 0
        for (i in 0 until arr.length()) {
            val s = arr.optJSONObject(i) ?: continue
            val row = SessionRow(
                sessionId = s.text("session_id"),
                deviceId = s.text("device_id"),
                isTest = s.optBoolean("is_test", false),
                mcp = s.optBoolean("mcp", false),
                phase = s.text("phase"),
                phaseText = s.text("phase_display"),
                durationText = humanDuration(s.optLong("duration_ms") ?: 0L),
                idleText = humanAge(s.optLong("idle_ms") ?: 0L),
                turnCount = s.optLong("turn_count") ?: 0L,
                lastUserText = s.text("last_user_text"),
                lastError = s.text("last_error"),
            )
            sessions.add(row)
            if (row.isTest) test++
        }
        summary = if (sessions.isEmpty()) {
            "当前没有连接"
        } else {
            "当前 ${sessions.size} 条连接" + if (test > 0) "（含 $test 条测试台）" else ""
        }
        loaded = true
        loadError = ""
    }
}

