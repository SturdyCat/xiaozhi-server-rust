package com.xiaozhi.admin

import com.tencent.kuikly.core.base.Border
import com.tencent.kuikly.core.base.BorderStyle
import com.tencent.kuikly.core.base.ViewContainer
import com.tencent.kuikly.core.directives.velse
import com.tencent.kuikly.core.directives.vif
import com.tencent.kuikly.core.module.NetworkModule
import com.tencent.kuikly.core.module.SharedPreferencesModule
import com.tencent.kuikly.core.nvi.serialization.json.JSONObject
import com.tencent.kuikly.core.reactive.handler.observable
import com.tencent.kuikly.core.views.Text
import com.tencent.kuikly.core.views.View

/**
 * 启动连接流程状态（macOS 测试台专用，macosArm64Main）。
 *
 * 流程（本地/远程调试均走这一条路）：
 * 1. 启动先显示「连接服务器」页：输入服务器地址（记忆上次地址，有记忆则自动发起连接）；
 * 2. 地址归一化后 GET {base}/api/config 读配置 → 填充配置表单（配置功能在 Mac 上由此可用）；
 * 3. 从配置提取 server.expected_token，拼出 ws(s)://{host}/api/ws?token=… 自动建立 WebSocket
 *    （测试功能：ASR/TTS）；
 * 4. 成功进入主壳（概览/测试台/配置），失败留在连接页并给出可操作提示。
 *
 * web（h5App）不受影响：web 由 server 同域托管、只提供配置功能，走 ConfigPage 的相对路径，
 * 不注册本壳（本文件仅 macOS 编译）。
 *
 * 响应式约定：状态依赖 UI 一律在 attr/vif 闭包内读取 observable（按钮文案随 busy 变化用
 * vif/velse 分支重建——构建期裸读不触发更新，见 kuikly-ui-framework 技能文档）。
 */
class ConnectState {

    /** 连接页输入框内容（原样保留用户输入，归一化在 connect 时进行） */
    var serverInput by observable("")

    /** 归一化解析出的 WS 地址（实时展示，便于确认远程地址拼对没有） */
    var resolvedWs by observable("")

    /** 流程忙（读配置 / WS 握手中），按钮禁用防重复点击 */
    var busy by observable(false)

    /** 状态行文案 + 级别（info/error/ok 决定颜色） */
    var statusMsg by observable(HINT_INIT)
    var statusLevel by observable("info")

    /** connect=连接页 / main=主壳 */
    var stage by observable("connect")

    /** 已连接服务器的 http 基址（无尾斜杠），配置页 load/save 经此跨机访问 */
    var baseUrl by observable("")

    private var launched = false

    /** 流程序号：connect 每次发起 +1，backToConnect 再 +1；迟到的回调按序号丢弃（防「切换服务器」后被拽回主壳） */
    private var flowSeq = 0

    // ============================================================
    // 生命周期入口（AdminShell.pageDidAppear 调一次）
    // ============================================================

    fun onLaunch(shell: AdminShell) {
        if (launched) return
        launched = true
        val saved = prefs(shell).getItem(KEY_SERVER_URL)
        serverInput = saved.ifEmpty { DEFAULT_SERVER }
        onInputChanged(serverInput)
        if (saved.isNotEmpty()) {
            // 记住过地址 → 启动即自动连接（连接页可见进度/错误，改地址重连即可）
            connect(shell)
        }
    }

    // ============================================================
    // 动作
    // ============================================================

    /** 输入变化：更新归一化预览 */
    fun onInputChanged(value: String) {
        serverInput = value
        resolvedWs = normalize(value)?.second ?: ""
    }

    /**
     * 主流程：读配置 → 填表单 → 取 token → 自动连 WS → 进主壳。
     * 任何一步失败都停在连接页并给出可操作的错误提示。
     */
    fun connect(shell: AdminShell) {
        if (busy) return
        val (base, ws) = normalize(serverInput) ?: run {
            statusLevel = "error"
            statusMsg = "地址无效：填 127.0.0.1:8000 或 http://192.168.1.10:8000"
            return
        }
        busy = true
        statusLevel = "info"
        statusMsg = "正在连接 $base 并读取配置…"
        val seq = ++flowSeq
        shell.acquireModule<NetworkModule>(NetworkModule.MODULE_NAME)
            .requestGet("$base/api/config", JSONObject()) { data, success, errorMsg, resp ->
                if (seq != flowSeq) return@requestGet // 已被「切换服务器」打断，丢弃迟到回调
                val code = resp.statusCode
                if (!success || (code != null && code != 200)) {
                    busy = false
                    statusLevel = "error"
                    statusMsg = "读取配置失败: ${errorMsg.ifEmpty { "网络错误" }}（检查地址与网络后重试）"
                    return@requestGet
                }
                if (!data.has("server")) {
                    busy = false
                    statusLevel = "error"
                    statusMsg = "$base 不是 xiaozhi-server-rust（/api/config 响应异常）"
                    return@requestGet
                }
                baseUrl = base
                // 记住地址：下次启动自动连接
                prefs(shell).setItem(KEY_SERVER_URL, base)
                // 填充配置表单（dirty 不受影响：程序填充不算用户改动）
                shell.form.fill(data)
                // 从配置提取 WS 鉴权 token，派生 WS 地址并自动连接
                val token = data.optJSONObject("server")?.optString("expected_token", "") ?: ""
                shell.bench.serverUrl = ws
                shell.bench.token = token
                statusMsg = "配置已读取，正在建立 WebSocket：$ws …"
                shell.bench.connect(shell) { ok ->
                    if (seq != flowSeq) return@connect
                    busy = false
                    if (ok) {
                        statusLevel = "ok"
                        statusMsg = "已连接 $ws"
                        stage = "main"
                        shell.toast = "已连接 $base（配置已同步，token 取自 server.expected_token）"
                    } else {
                        statusLevel = "error"
                        statusMsg = "配置已读取，但 WebSocket 连接失败: ${shell.bench.statusMsg}（可在测试台改 token 重试，或直接进入）"
                    }
                }
            }
    }

    /** 跳过连接直接进主壳（配置可看；未连接时读写配置/测试不可用） */
    fun skip(shell: AdminShell) {
        if (busy) return
        stage = "main"
        shell.toast = if (baseUrl.isEmpty()) "未连接服务器：请稍后在侧边栏「切换服务器」连接" else "已进入主界面（WebSocket 未连接）"
    }

    /** 主壳 → 连接页（侧边栏「切换服务器」）：断开当前 WS，保留输入便于改地址 */
    fun backToConnect(shell: AdminShell) {
        flowSeq++ // 使进行中的连接流程回调全部失效
        if (shell.bench.connected) shell.bench.disconnect(shell)
        busy = false
        statusLevel = "info"
        statusMsg = HINT_INIT
        stage = "connect"
    }

    // ============================================================
    // 地址归一化：容忍 ws:// wss:// 无 scheme、路径后缀等输入
    // 127.0.0.1:8000            → http://127.0.0.1:8000 + ws://127.0.0.1:8000/api/ws
    // http://192.168.1.10:8000  → 原样 + ws://192.168.1.10:8000/api/ws
    // wss://demo.notip.com.cn   → https://demo.notip.com.cn + wss://demo.notip.com.cn/api/ws
    // ============================================================

    private fun normalize(input: String): Pair<String, String>? {
        var s = input.trim()
        if (s.isEmpty()) return null
        s = when {
            s.startsWith("ws://", true) -> "http://" + s.substring(5)
            s.startsWith("wss://", true) -> "https://" + s.substring(6)
            else -> s
        }
        if (!s.contains("://")) s = "http://$s"
        val schemeEnd = s.indexOf("://")
        val scheme = s.substring(0, schemeEnd).lowercase()
        if (scheme != "http" && scheme != "https") return null
        // 只保留 authority：服务端在根路径挂 /api/*，用户误带 /api/ws 也能纠正
        val authority = s.substring(schemeEnd + 3).split('/', '?', '#').firstOrNull() ?: ""
        if (authority.isEmpty() || authority.startsWith(":")) return null
        val base = "$scheme://$authority"
        val ws = "${if (scheme == "https") "wss" else "ws"}://$authority/api/ws"
        return base to ws
    }

    private fun prefs(shell: AdminShell) =
        shell.acquireModule<SharedPreferencesModule>(SharedPreferencesModule.MODULE_NAME)

    companion object {
        private const val KEY_SERVER_URL = "xz.server.baseUrl"
        private const val DEFAULT_SERVER = "http://127.0.0.1:8000"
        private const val HINT_INIT =
            "将读取服务器配置（/api/config）并自动建立 WebSocket 连接"
    }
}

/**
 * 启动连接页渲染：暗色居中卡片——地址输入 + WS 解析预览 + 状态行 + 连接/直接进入。
 * ⚠️ 必须在目标容器闭包内**非限定**调用（接收者=该容器），经 ctx. 调用会布局逃逸（见 AdminShell 注释）。
 */
fun ViewContainer<*, *>.renderConnect(conn: ConnectState, shell: AdminShell) {
    View {
        attr {
            flex(1f)
            backgroundColor(AdminColors.windowBg)
            allCenter()
        }
        View {
            attr {
                width(480f)
                flexDirectionColumn()
                backgroundColor(AdminColors.cardBg)
                borderRadius(AdminShape.radiusLg)
                border(Border(1f, BorderStyle.SOLID, AdminColors.divider))
                padding(AdminSpace.cardPadding)
            }
            Text {
                attr {
                    fontSize(AdminType.title)
                    fontWeightMedium()
                    color(AdminColors.textPrimary)
                    text("连接服务器")
                }
            }
            Text {
                attr {
                    fontSize(AdminType.caption)
                    color(AdminColors.textSecondary)
                    marginTop(AdminSpace.xs)
                    text("本地调试填 127.0.0.1:8000，远程填服务器 IP/域名。读取配置后自动连接 WebSocket。")
                }
            }
            labeledField("服务器地址", { conn.serverInput }, { conn.onInputChanged(it) }, "http://127.0.0.1:8000")
            vif({ conn.resolvedWs.isNotEmpty() }) {
                Text {
                    attr {
                        fontSize(AdminType.micro)
                        color(AdminColors.textTertiary)
                        marginTop(AdminSpace.xs)
                        text("WS: ${conn.resolvedWs}")
                    }
                }
            }
            Text {
                attr {
                    fontSize(AdminType.caption)
                    // 级别颜色在 attr 闭包内按 observable 求值 → 状态变化即时变色
                    color(
                        when (conn.statusLevel) {
                            "error" -> AdminColors.dangerTintText
                            "ok" -> AdminColors.accentTintText
                            else -> AdminColors.textSecondary
                        },
                    )
                    marginTop(AdminSpace.md)
                    text(conn.statusMsg)
                }
            }
            actionRow {
                // 忙碌态用 vif/velse 分支重建（构建期裸读 busy 不会响应式更新按钮文案）
                vif({ conn.busy }) {
                    primaryButton("连接中…", enabled = false) { }
                }
                velse {
                    primaryButton("连接") { conn.connect(shell) }
                }
                View { attr { width(AdminSpace.md) } }
                secondaryButton("直接进入") { conn.skip(shell) }
            }
        }
    }
}
