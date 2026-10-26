package com.xiaozhi.admin

import com.tencent.kuikly.core.nvi.serialization.json.JSONObject

/**
 * 一台 EchoMuse 设备（`GET /api/echomuse/devices` 的一行）。字段不可变——刷新时整表重建。
 *
 * 单独成文件的原因：`EchoMuseState.kt` 已到 600 行上限（设计 §7-T04 拆分的硬约束），
 * 而本类是**纯数据 + 展示文本**（无请求、无 observable），跟"列表/审批/下发"那套状态
 * 逻辑没有耦合。T03/T04 两次拆分后这里只剩"这台设备是什么、怎么显示"。
 */
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
