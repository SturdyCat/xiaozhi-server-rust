package com.xiaozhi.admin

import com.tencent.kuikly.core.base.PagerScope
import com.tencent.kuikly.core.module.NetworkModule
import com.tencent.kuikly.core.nvi.serialization.json.JSONObject
import com.tencent.kuikly.core.pager.Pager
import com.tencent.kuikly.core.reactive.collection.ObservableList
import com.tencent.kuikly.core.reactive.handler.observable
import com.tencent.kuikly.core.reactive.handler.observableList

/**
 * 「能力总览」的数据源：`GET /api/plugins`（`docs/plugin-architecture-unification.md` §5.2/§5.3）。
 *
 * 为什么要有这一页：此前管理页只能靠"保存后能不能用"猜后端——`GET /api/plugins` 早就把
 * `enabled`（用户的选择）与 `phase`（实际状态）**分开**上报，并带 `last_error`，
 * 但界面上没有任何地方显示它。于是"配了没生效 / 降级了"只能翻服务端日志。
 *
 * 三条呈现规则（照 DSH，但补上 DSH 没有的原因文本）：
 * 1. **只在偏离时打标**：`active` 行不打徽标，避免一片绿色噪音；
 * 2. **降级/异常必须显示原因**：`last_error` 直接展开在行内（DSH 自陈"只有阶段没有原因"）；
 * 3. **计数摘要只显示非零项**：`共 N 个能力 · X 运行中 · Y 异常`，0 的项不出现。
 *
 * 拆成独立文件的两条理由：① `AGENTS.md` §5.10 文件规模约定；② 它是**只读投影**
 * （不参与表单编辑），与 `ConfigFormState`（可写表单）混在一起会让"哪些字段会被回传"
 * 变得难以判断。
 */
class PluginMetaState(private val scope: PagerScope) {

    /** 必需能力（asr/vad/tts/llm）：坏掉即启动失败，单列一组。 */
    val requiredRows: ObservableList<CapabilityRow> by scope.observableList()

    /** 可选能力：坏掉只降级（记忆 / 灵魂 / 发音人目录 / 固件托管 / AIUI）。 */
    val optionalRows: ObservableList<CapabilityRow> by scope.observableList()

    /** 计数摘要（只含非零项）；未加载时为空串。 */
    var summary by scope.observable("")

    /** 加载失败的就地错误文案（首次加载失败 → 就地错误 + 「重试」，不弹 toast）。 */
    var loadError by scope.observable("")

    var loading by scope.observable(false)
    var loaded by scope.observable(false)

    private fun network(ctx: Pager): NetworkModule = ctx.acquireModule(NetworkModule.MODULE_NAME)

    /** 上次使用的服务端基址：`refresh` 复用（web 同域为空串，macApp 连接流程传绝对地址）。 */
    private var base = ""

    /** 拉取能力清单（+ 实际状态）。失败只置 `loadError`，不影响配置表单本身。 */
    fun load(ctx: Pager, baseUrl: String = "") {
        if (loading) return
        base = baseUrl
        loading = true
        loadError = ""
        network(ctx).requestGet("${baseUrl}/api/plugins", JSONObject()) { data, success, errorMsg, resp ->
            loading = false
            val code = resp.statusCode
            if (!success || (code != null && code !in 200..299)) {
                loadError = "能力清单加载失败: ${if (errorMsg.isNotEmpty()) errorMsg else "HTTP $code"}"
                return@requestGet
            }
            apply(data)
        }
    }

    /** 按上次的基址重新拉取（总览页「刷新」按钮）。 */
    fun refresh(ctx: Pager) = load(ctx, base)

    /** 解析 `capabilities[]` → 两行列表 + 计数摘要。 */
    private fun apply(data: JSONObject) {
        requiredRows.clear()
        optionalRows.clear()
        val caps = data.optJSONArray("capabilities")
        if (caps == null) {
            loadError = "能力清单为空（服务端返回异常）"
            return
        }
        var active = 0
        var degraded = 0
        var failed = 0
        var disabled = 0
        for (i in 0 until caps.length()) {
            val c = caps.optJSONObject(i) ?: continue
            val row = CapabilityRow(
                id = c.optString("id") ?: "",
                display = c.optString("display") ?: "",
                required = c.optBoolean("required", false),
                phase = c.optString("phase", "disabled"),
                enabled = c.optBoolean("enabled", false),
                implDisplay = activeImplLabel(c),
                hotHint = hotHintOf(c),
                reason = c.optString("last_error", ""),
            )
            if (row.required) requiredRows.add(row) else optionalRows.add(row)
            when (row.phase) {
                "active" -> active++
                "degraded" -> degraded++
                "failed" -> failed++
                else -> disabled++
            }
        }
        val total = requiredRows.size + optionalRows.size
        val parts = mutableListOf("共 $total 个能力")
        if (active > 0) parts.add("$active 运行中")
        if (degraded > 0) parts.add("$degraded 降级")
        if (failed > 0) parts.add("$failed 异常")
        if (disabled > 0) parts.add("$disabled 已关闭")
        summary = parts.joinToString(" · ")
        loaded = true
        loadError = ""
    }

    /** 当前选中的实现/服务商展示名（`implementations[]` 里找 `active_impl`）。 */
    private fun activeImplLabel(cap: JSONObject): String {
        val impls = cap.optJSONArray("implementations") ?: return ""
        val active = cap.optString("active_impl", "")
        var firstEnabled: String = ""
        for (i in 0 until impls.length()) {
            val im = impls.optJSONObject(i) ?: continue
            val id = im.optString("id") ?: ""
            val label = im.optString("display") ?: ""
            if (id == active) return label
            if (firstEnabled.isEmpty() && im.optBoolean("enabled", false)) firstEnabled = label
        }
        return firstEnabled
    }

    /**
     * 该能力的生效语义文案：优先用服务端 `implementations[].hot_hint`（注册表是唯一权威），
     * 缺失时才本地兜底——避免前后端各写一份文案而漂移。
     */
    private fun hotHintOf(cap: JSONObject): String {
        val impls = cap.optJSONArray("implementations")
        if (impls != null) {
            val active = cap.optString("active_impl", "")
            for (i in 0 until impls.length()) {
                val im = impls.optJSONObject(i) ?: continue
                if ((im.optString("id") ?: "") == active) {
                    val hint = im.optString("hot_hint", "")
                    if (hint.isNotEmpty()) return hint
                }
            }
        }
        return when (cap.optString("hot", "")) {
            "live" -> "保存即生效"
            "next_session" -> "保存后新会话生效"
            "restart_only" -> "需重启 server 生效"
            else -> ""
        }
    }
}

/**
 * 总览页的一行（一个能力）。字段都是**不可变快照**——刷新时整表重建，
 * 不做增量更新（列表规模为个位数，重建更不容易出现"改了一半"的中间态）。
 */
class CapabilityRow(
    val id: String,
    val display: String,
    val required: Boolean,
    /** `active` / `degraded` / `failed` / `disabled`（服务端 `phase`，勿与 `enabled` 混用）。 */
    val phase: String,
    /** 用户配置里是否选中（== `Config` 里的 `enabled`）。 */
    val enabled: Boolean,
    /** 当前实现/服务商的展示名（如「本地 Kokoro INT8」）。 */
    val implDisplay: String,
    /** 生效语义文案（"保存后新会话生效"等）。 */
    val hotHint: String,
    /** 降级/失败原因（含「怎么修」）；正常时为空串。 */
    val reason: String,
)
