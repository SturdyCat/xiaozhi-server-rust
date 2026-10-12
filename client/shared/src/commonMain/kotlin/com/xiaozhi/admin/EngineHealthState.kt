package com.xiaozhi.admin

import com.tencent.kuikly.core.base.PagerScope
import com.tencent.kuikly.core.nvi.serialization.json.JSONObject
import com.tencent.kuikly.core.reactive.collection.ObservableList
import com.tencent.kuikly.core.reactive.handler.observable
import com.tencent.kuikly.core.reactive.handler.observableList

/**
 * 引擎健康自检的数据源：`GET /api/health/engines`（P1-6，设计见 `docs/p1-round1-design.md` §2.1）。
 *
 * ## 与 [PluginMetaState]（`GET /api/plugins`）的分工
 *
 * - `/api/plugins`：**实现级**清单 + 配置级校验状态（plan 的纯投影，无 I/O）；
 * - `/api/health/engines`：**能力级**健康聚合 + 运行期本地探测（模型文件存在 / 记忆目录可写）。
 *
 * 服务端两处的能力聚合同源（共 `PluginHost::capability_summary`），
 * 因此本页的 phase 与总览页不会分叉——客户端不再自算任何状态。
 *
 * 呈现规则与总览页同口径：
 * 1. **只在偏离时打标**（`deviation` 行才占视觉，正常行弱化，避免一片绿噪音）；
 * 2. **降级/异常必须显示原因**（`last_error` 含「怎么修」，行内展开）；
 * 3. 概览页横幅（[EngineHealthCard.healthBanner]）只渲染 deviation 行，可点击跳配置 section。
 *
 * ⚠️ 只读投影：绝不参与配置表单，不随 `POST /api/config` 回传。
 */
class EngineHealthState(scope: PagerScope) : LoadableState(scope) {

    /** 健康行（11 个 capability 各一行；刷新时整表重建，行是不可变快照）。 */
    val rows: ObservableList<HealthRow> by scope.observableList()

    /** 偏离行（degraded/failed）——横幅只渲染这些，正常时列表为空、横幅不占视觉。 */
    val deviations: ObservableList<HealthRow> by scope.observableList()

    /** 计数摘要（只含非零项）；未加载时为空串。 */
    var summary by scope.observable("")

    override fun endpoint(baseUrl: String): String = "$baseUrl/api/health/engines"

    override val errorLabel: String = "引擎健康数据加载失败"

    /** 解析 `capabilities[]` → 行列表 + 偏离行 + 计数摘要。 */
    override fun apply(data: JSONObject) {
        rows.clear()
        deviations.clear()
        val caps = data.optJSONArray("capabilities")
        if (caps == null) {
            loadError = "健康清单为空（服务端返回异常）"
            return
        }
        var active = 0
        var degraded = 0
        var failed = 0
        var disabled = 0
        for (i in 0 until caps.length()) {
            val c = caps.optJSONObject(i) ?: continue
            val row = HealthRow(
                capability = c.text("capability"),
                display = c.text("display"),
                required = c.optBoolean("required", false),
                enabled = c.optBoolean("enabled", false),
                phase = c.text("phase"),
                activeImpl = c.text("active_impl"),
                lastError = c.text("last_error"),
                probeState = c.optJSONObject("probe")?.text("state") ?: "",
                probeDetail = c.optJSONObject("probe")?.text("detail") ?: "",
                // config_section 为 null（voices 无对应配置页）时收成空串，
                // 横幅据此不渲染点击行为
                configSection = c.text("config_section"),
            )
            rows.add(row)
            when (row.phase) {
                "active" -> active++
                "degraded" -> degraded++
                "failed" -> failed++
                else -> disabled++
            }
            if (row.phase == "degraded" || row.phase == "failed") deviations.add(row)
        }
        val parts = mutableListOf("共 ${rows.size} 项")
        if (active > 0) parts.add("$active 正常")
        if (degraded > 0) parts.add("$degraded 降级")
        if (failed > 0) parts.add("$failed 异常")
        if (disabled > 0) parts.add("$disabled 已关闭")
        summary = parts.joinToString(" · ")
        loaded = true
        loadError = ""
    }
}

/**
 * 健康行（一个 capability）。字段是不可变快照——刷新时整表重建，
 * 与 [CapabilityRow] 同模式（列表规模个位数，重建不易出"改了一半"的中间态）。
 */
class HealthRow(
    /** capability id（服务端 `Capability::id()`，如 "memory"）。 */
    val capability: String,
    val display: String,
    val required: Boolean,
    /** 合并后终态：`active` / `degraded` / `failed` / `disabled`。 */
    val phase: String,
    val enabled: Boolean,
    /** 当前选中实现 id（如 "memory.graph"）；未启用为空串。 */
    val activeImpl: String,
    /** 降级/失败原因（含「怎么修」）；正常时为空串。 */
    val lastError: String,
    /** 探测结论：`ok` / `fail` / 空串（无探测项或未启用）。 */
    val probeState: String,
    /** 探测原始文案（与 last_error 同文案时也保留，便于区分来源）。 */
    val probeDetail: String,
    /** 跳转锚点（配置页 section id）；voices 无对应 section 为空串。 */
    val configSection: String,
) {
    /** 是否偏离（横幅与健康卡据此打标）。 */
    val isDeviation: Boolean get() = phase == "degraded" || phase == "failed"

    /** 展示用原因：优先 last_error（合并后终态原因），空时回退探测文案。 */
    val reason: String get() = if (lastError.isNotEmpty()) lastError else probeDetail

    /** 行是否可点击跳转（有偏离且有对应配置 section）。 */
    val jumpable: Boolean get() = isDeviation && configSection.isNotEmpty()
}
