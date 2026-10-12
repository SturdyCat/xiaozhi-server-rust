package com.xiaozhi.admin

import com.tencent.kuikly.core.base.PagerScope
import com.tencent.kuikly.core.nvi.serialization.json.JSONArray
import com.tencent.kuikly.core.nvi.serialization.json.JSONObject
import com.tencent.kuikly.core.reactive.collection.ObservableList
import com.tencent.kuikly.core.reactive.handler.observable
import com.tencent.kuikly.core.reactive.handler.observableList

/**
 * 「MCP 工具」页的数据源：`GET /api/mcp/tools`。
 *
 * 回答一个此前**只能翻设备串口日志或读固件源码**的问题：这台 ESP 支持哪些 MCP 工具、
 * 每个工具能干什么、要什么参数。
 *
 * 数据是**会话开始时**服务器通过 MCP `initialize` + `tools/list` 从设备拉到的（见服务端
 * `app/session/mcp.rs`），因此：
 * - 设备空闲时不建连 → 刚部署本功能、或设备长期没唤醒过，这里会是空的（用服务端的 `note` 说明）；
 * - 每台设备显示的是**最近一次**探测结果，带 `age_ms`（"3 分钟前"）与在线状态——
 *   不把"上次探测"伪装成"现在"。
 *
 * 呈现规则（与「总览」页一致）：只在偏离时打标（在线/正常不打标）、失败必须显示原因、
 * 空列表要给可行动的解释。拆成独立文件的两条理由同 [PluginMetaState]：`AGENTS.md` §5.10
 * 文件规模约定；它是**只读投影**，与可写表单混在一起会让"哪些字段会被回传"难以判断。
 */
class McpToolsState(scope: PagerScope) : LoadableState(scope) {

    /** 已登记的设备（最近探测的在前，服务端已排序）。 */
    val devices: ObservableList<DeviceToolsRow> by scope.observableList()

    /** 计数摘要（只含非零项）；未加载时为空串。 */
    var summary by scope.observable("")

    /** 服务端给的说明（空列表时显示：为什么这里什么都没有）。 */
    var note by scope.observable("")

    override fun endpoint(baseUrl: String): String = "$baseUrl/api/mcp/tools"

    override val errorLabel: String = "设备工具清单加载失败"

    /** 解析 `devices[]` → 行列表 + 计数摘要。 */
    override fun apply(data: JSONObject) {
        devices.clear()
        note = data.optString("note", "")
        val arr = data.optJSONArray("devices")
        if (arr == null) {
            loadError = "设备工具清单为空（服务端返回异常）"
            return
        }
        var online = 0
        var tools = 0
        for (i in 0 until arr.length()) {
            val d = arr.optJSONObject(i) ?: continue
            val row = DeviceToolsRow(
                deviceId = d.optString("device_id", ""),
                board = d.optString("board", ""),
                firmwareVersion = d.optString("firmware_version", ""),
                protocolVersion = d.optString("protocol_version", ""),
                online = d.optBoolean("online", false),
                ageText = ageText(d.optLong("age_ms") ?: 0L),
                toolCount = d.optInt("tool_count", 0),
                aiToolCount = d.optInt("ai_tool_count", 0),
                userToolCount = d.optInt("user_tool_count", 0),
                commandGate = d.optString("command_gate", ""),
                error = d.optString("error", ""),
                tools = toolRows(d.optJSONArray("tools")),
            )
            devices.add(row)
            if (row.online) online++
            tools += row.toolCount
        }
        val parts = mutableListOf("共 ${devices.size} 台设备上报过工具")
        if (tools > 0) parts.add("工具 $tools 个")
        if (online > 0) parts.add("$online 台在线")
        summary = parts.joinToString(" · ")
        loaded = true
        loadError = ""
    }

    private fun toolRows(arr: JSONArray?): List<McpToolRow> {
        if (arr == null) return emptyList()
        val out = mutableListOf<McpToolRow>()
        for (i in 0 until arr.length()) {
            val t = arr.optJSONObject(i) ?: continue
            out.add(
                McpToolRow(
                    name = t.optString("name", ""),
                    description = t.optString("description", ""),
                    userOnly = t.optBoolean("user_only", false),
                    // 参数摘要由服务端给（服务端是字段唯一权威，客户端不解析 JSON Schema）
                    signature = t.optString("signature", ""),
                ),
            )
        }
        return out
    }

    /**
     * `age_ms` → 人话。
     *
     * 用服务端给的**相对毫秒**而不是本地时钟：客户端不必知道"现在几点"，
     * 也不会因设备/浏览器时区或时钟漂移把"3 分钟前"显示成"3 小时前"。
     */
    private fun ageText(ms: Long): String = when {
        ms < 60_000L -> "刚刚"
        ms < 3_600_000L -> "${ms / 60_000L} 分钟前"
        ms < 86_400_000L -> "${ms / 3_600_000L} 小时前"
        else -> "${ms / 86_400_000L} 天前"
    }
}

/**
 * 一台设备的工具快照。字段都是**不可变快照**——刷新时整表重建，不做增量更新
 * （列表规模为个位数，重建更不容易出现"改了一半"的中间态）。
 */
class DeviceToolsRow(
    val deviceId: String,
    /** 板型名（固件 `serverInfo.name`，如 `bread-compact-wifi`）。 */
    val board: String,
    /** 固件版本（固件 `serverInfo.version`）。 */
    val firmwareVersion: String,
    /** 设备声明的 MCP 协议版本。 */
    val protocolVersion: String,
    /** 最近一次探测时设备是否连着（会话结束会翻成 false）。 */
    val online: Boolean,
    /** 距最近一次探测多久（"3 分钟前"）。 */
    val ageText: String,
    val toolCount: Int,
    /** 其中**对大模型可见**的工具数。 */
    val aiToolCount: Int,
    /** 其中 `user_only`（不暴露给大模型，只给管理端/服务器按名字调用）的数量。 */
    val userToolCount: Int,
    /**
     * 指令闸门兼容性三态（P1-10，服务端给）：
     * `compatible` = 清单含 stop 工具；`will_degrade` = 清单在手但不含（说「退下」将断开）；
     * `unknown`/空 = 清单未知（探测失败）。派生属性 [commandGateLabel] 供展示。
     */
    val commandGate: String,
    /** 探测未完成的原因（空串 = 完整成功）。 */
    val error: String,
    val tools: List<McpToolRow>,
) {
    /** 副标题：板型 · 固件 · 协议 · 探测时间。缺失项自动省略（老固件可能不报版本）。 */
    val subtitle: String
        get() = listOfNotNull(
            board.ifEmpty { null },
            firmwareVersion.takeIf { it.isNotEmpty() }?.let { "固件 $it" },
            protocolVersion.takeIf { it.isNotEmpty() }?.let { "MCP $it" },
            "探测于 $ageText",
        ).joinToString(" · ")

    /** 工具构成一句话（只显示非零项）。 */
    val toolSummary: String
        get() = buildString {
            append("$toolCount 个工具")
            if (userToolCount > 0) append("（$aiToolCount 个可被 AI 调用 · $userToolCount 个仅管理员）")
        }

    /** 指令闸门兼容标记文案（空串 = 无需展示）。 */
    val commandGateLabel: String
        get() = when (commandGate) {
            "compatible" -> "指令闸门兼容"
            "will_degrade" -> "说「退下」将断开"
            else -> ""
        }
}

/** 一个设备工具（`GET /api/mcp/tools` 的投影）。 */
class McpToolRow(
    val name: String,
    /** 工具描述——**它同时是喂给大模型的提示词**，所以显示原文。 */
    val description: String,
    /** `true` = 固件 `AddUserOnlyTool` 注册：默认不出现在 `tools/list`、大模型看不到。 */
    val userOnly: Boolean,
    /** 参数摘要（服务端生成，如 `volume: integer（必填，0~100）`；无参数为 `（无参数）`）。 */
    val signature: String,
)
