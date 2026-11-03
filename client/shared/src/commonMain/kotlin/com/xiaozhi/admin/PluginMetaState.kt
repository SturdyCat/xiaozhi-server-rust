package com.xiaozhi.admin


import com.tencent.kuikly.core.base.PagerScope
import com.tencent.kuikly.core.nvi.serialization.json.JSONObject
import com.xiaozhi.admin.state.stateVar
import com.xiaozhi.admin.state.stateListOf

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
class PluginMetaState(scope: PagerScope) : LoadableState(scope) {

    /** 必需能力（asr/vad/tts/llm）：坏掉即启动失败，单列一组。 */
    val requiredRows: MutableList<CapabilityRow> = stateListOf()

    /** 可选能力：坏掉只降级（记忆 / 灵魂 / 发音人目录 / 固件托管 / AIUI）。 */
    val optionalRows: MutableList<CapabilityRow> = stateListOf()

    /**
     * 设备插件（echomuse/tuya）注册投影（`device_plugins[]`）——它们不是语音引擎
     * （无 implementations），服务端单独成数组随同一端点下发。
     * 注册语义 = 配置段启用：管理台据此决定「设备管理」菜单与各插件分区是否出现。
     */
    val devicePluginRows: MutableList<DevicePluginRow> = stateListOf()

    /** 计数摘要（只含非零项）；未加载时为空串。 */
    var summary by stateVar("")

    override fun endpoint(baseUrl: String): String = "$baseUrl/api/plugins"

    override val errorLabel: String = "能力清单加载失败"

    fun devicePlugin(id: String): DevicePluginRow? = devicePluginRows.firstOrNull { it.id == id }

    /**
     * 是否至少注册了一个设备插件（「设备管理」菜单的门控）。
     * 未加载或旧服务端未上报 `device_plugins`（空数组）时按 true 处理：
     * 菜单不因加载间隙闪隐，旧服务端维持合并前的常显行为。
     */
    fun anyDevicePluginRegistered(): Boolean {
        if (!loaded || devicePluginRows.isEmpty()) return true
        return devicePluginRows.any { it.registered }
    }

    /** 解析 `capabilities[]` → 两行列表 + 计数摘要。 */
    override fun apply(data: JSONObject) {
        requiredRows.clear()
        optionalRows.clear()
        devicePluginRows.clear()
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
        // 设备插件注册投影（旧服务端无此键 → 保持空数组，门控按"已注册"兜底）
        val dps = data.optJSONArray("device_plugins")
        if (dps != null) {
            for (i in 0 until dps.length()) {
                val p = dps.optJSONObject(i) ?: continue
                devicePluginRows.add(
                    DevicePluginRow(
                        id = p.optString("id") ?: "",
                        label = p.optString("label") ?: "",
                        registered = p.optBoolean("registered", false),
                        phase = p.optString("phase", "disabled"),
                        configSection = p.optString("config_section", ""),
                        note = p.optString("note", ""),
                        reason = p.optString("last_error", ""),
                    ),
                )
            }
        }
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

/**
 * 设备插件一行（`GET /api/plugins` 的 `device_plugins[]` 投影）。
 * `registered` = 配置段启用（注册门控）；`configSection` 指向唯一配置表单的 section id。
 */
class DevicePluginRow(
    val id: String,
    val label: String,
    val registered: Boolean,
    /** `active` / `disabled`（纯配置投影，不做运行探测）。 */
    val phase: String,
    val configSection: String,
    /** 热生效语义说明（服务端下发，如「RestartOnly：开关保存后需重启生效」）。 */
    val note: String,
    /** 异常原因；正常时为空串。 */
    val reason: String,
)
