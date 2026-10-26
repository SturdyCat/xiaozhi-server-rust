package com.xiaozhi.admin

import com.xiaozhi.admin.ui.*

import com.tencent.kuikly.core.base.PagerScope
import com.tencent.kuikly.core.reactive.handler.observable
import com.tencent.kuikly.core.reactive.handler.observableList

/** 日志来源白名单 id ↔ 下拉文案。id 必须与服务端 `log_command` 的 match 分支逐一对应。 */
private val EM_LOG_SOURCES = listOf(
    "server" to "设备服务日志（/tmp/server.log）",
    "messages" to "当前系统日志（messages）",
    "messages_last" to "上次系统日志（messages.last）",
    "last_kmsg_prev" to "上次内核日志（last_kmsg.prev）",
    "supervisor" to "启动监督日志（supervisor.log）",
    "net" to "网络日志（net.log）",
)

/**
 * 日志 tab 的状态：**固定来源白名单**下拉 + 读取 + 复制。
 *
 * 从 `EchoMuseManagementState` 里整块搬出来（设计 §2.2：日志是唯一"会反复盯着看"的
 * 分区，和音量/重启这类一次性操作混在一个对象里，进 tab 就会连带抓运行状态）。
 * 白名单 6 项与文案**逐字保留**：服务端 `system/logs?source=` 只认这 6 个 id，
 * 前端多一项会被 400 拒绝、少一项用户就看不到那份日志。
 *
 * 设备事实（已批准/在线/emOS）与**单操作闸门**不在这里：同一台设备只有一把槽
 * （`/system/…`、控制台、资产下发共用，见 §8.4），所以读取走
 * [EchoMuseDetailState.run]——两个对象抢同一把槽时，闸门只能有一个。
 *
 * 不做任意路径：`source` 是白名单 id，不是文件名；设备端的路径拼接在服务端
 * `log_command` 里固定，前端拿不到"读任意文件"的能力。
 */
class EchoMuseLogsState(scope: PagerScope, private val detail: EchoMuseDetailState) {

    /** 当前选中的来源 id（默认第一项 = 设备自身的 server.log）。 */
    var logSource by scope.observable(EM_LOG_SOURCES.first().first)

    /** 下拉展开态（`dropdownField` 需要）。 */
    var sourceOpen by scope.observable(false)

    /** 下拉选项（`observableList<Pair<id, 文案>>`，与其它下拉同型）。 */
    val logSources by scope.observableList<Pair<String, String>>()

    /** 最近一次读取到的日志原文。 */
    var output by scope.observable("")
        private set

    /** 是否读到过（决定"复制日志"可用性与输出区是否渲染）。 */
    var loaded by scope.observable(false)
        private set

    /** 本次输出对应的来源 id（服务端回包里的 `source`，可能与会话里选的不同）。 */
    var loadedSource by scope.observable("")
        private set

    /** 输出所属设备号：与当前 `target` 不一致时属于串台，视图应提示重新读取。 */
    var loadedDevice by scope.observable("")
        private set

    /** 读取时的 control 会话标识：会话变了，这份输出就是"历史快照"。 */
    private var loadedSession = ""

    init {
        logSources.addAll(EM_LOG_SOURCES)
    }

    /** 输出是否为历史快照（设备重连过，或读的不是当前设备）。 */
    val historical: Boolean
        get() = loaded && (loadedSession != detail.sessionId || crossDevice)

    /** 输出是否属于另一台设备（切设备后必须重新读取，不能把上一台的日志当成当前设备的）。 */
    val crossDevice: Boolean
        get() = loadedDevice.isNotEmpty() && loadedDevice != detail.target

    /** 能否读取（已批准 + emOS + 在线 + 无并发操作）。 */
    val canRead: Boolean get() = detail.canSystem && detail.idle

    /** 设备切换时清空上一台的输出（由 `EchoMuseState` 在列表刷新后调用）。 */
    fun syncDevice(deviceId: String) {
        if (loadedDevice.isEmpty() || loadedDevice == deviceId) return
        output = ""
        loaded = false
        loadedSource = ""
        loadedSession = ""
        loadedDevice = ""
        sourceOpen = false
    }

    /** 读一次选定来源的日志（设备上那份文件的尾部 48 KB，服务端截断）。 */
    fun read() {
        if (!canRead) return
        if (logSources.none { it.first == logSource }) return
        val source = logSource
        val deviceId = detail.target
        detail.run("system/logs?source=$source", system = true) { data ->
            output = data.text("output")
            loadedSource = data.text("source").ifEmpty { source }
            loadedSession = detail.sessionId
            loadedDevice = deviceId
            loaded = true
        }
    }

    /** 「复制日志」取它（本类不碰平台剪贴板）。 */
    fun copyAll(): String = output

    /** 诊断汇总里的日志段落（未读取时为空串，调用方直接拼接即可）。 */
    val diagnosticSection: String
        get() = if (!loaded || crossDevice) {
            ""
        } else {
            "日志 $loadedSource${if (historical) "（历史）" else "（手动快照）"}：\n$output"
        }
}
