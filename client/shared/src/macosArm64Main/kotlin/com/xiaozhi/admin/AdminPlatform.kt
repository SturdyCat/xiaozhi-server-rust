package com.xiaozhi.admin

import com.xiaozhi.admin.ui.*

import com.tencent.kuikly.core.base.Border
import com.tencent.kuikly.core.base.BorderStyle
import com.tencent.kuikly.core.base.CursorType
import com.tencent.kuikly.core.base.PagerScope
import com.tencent.kuikly.core.base.ViewContainer
import com.tencent.kuikly.core.directives.vfor
import com.tencent.kuikly.core.directives.vif
import com.tencent.kuikly.core.module.Module
import com.tencent.kuikly.core.module.SharedPreferencesModule
import com.tencent.kuikly.core.nvi.serialization.json.JSONArray
import com.tencent.kuikly.core.nvi.serialization.json.JSONObject
import com.tencent.kuikly.core.pager.Pager
import com.tencent.kuikly.core.reactive.handler.observable
import com.tencent.kuikly.core.reactive.handler.observableList
import com.tencent.kuikly.core.views.Input
import com.tencent.kuikly.core.views.TextInputState
import com.tencent.kuikly.core.views.Text
import com.tencent.kuikly.core.views.View
import platform.AppKit.NSPasteboard
import platform.AppKit.NSPasteboardTypeString
import platform.Foundation.NSDate
import platform.Foundation.NSDateFormatter
import platform.Foundation.NSLocale
import platform.Foundation.NSTimeZone
import platform.Foundation.dateWithTimeIntervalSince1970
import platform.Foundation.localeWithLocaleIdentifier
import platform.Foundation.localTimeZone

/** macOS 客户端：测试台、XiaoZhiModule 原生桥与「服务器设置」仅此平台启用。 */
actual val IS_MACOS: Boolean = true

/** 原生渲染器由 textInputState 通道显示；再绑 text() 会打断 IME 组词（见 AdminFormControls）。 */
actual val INPUT_NEEDS_TEXT_PROP: Boolean = false

/** 测试台依赖的原生桥（WS 建连 / 麦克风采集 / 播放 / Opus 编解码，实现在 apps/macosApp）。 */
actual fun Pager.platformExternalModules(): Map<String, Module> =
    XiaoZhiModule()
        .also { registerSharedBridge(it) } // AdminRealtime 的 PlatformWsSocket 无 Pager，经弱引用取桥
        .let { mapOf(XiaoZhiModule.MODULE_NAME to it) }

/**
 * 测试台状态缓存：AdminShell 时代 bench 是壳的字段（跨 section 切换存活）；
 * 统一壳后测试台渲染按需创建，按 Pager 缓存，保证切走再切回来
 * 连接状态 / 录音不丢（与 AdminShell 行为一致）。
 * （Kotlin/Native 无 java.util.WeakHashMap；管理台进程内只有一个 admin Pager，普通 map 即可。）
 */
private val benchStates = mutableMapOf<Pager, TestBenchState>()

actual fun ViewContainer<*, *>.renderTestbenchSection(
    pager: Pager,
    form: ConfigFormState,
    twoColumn: () -> Boolean,
    pageWidth: () -> Float,
    pageHeight: () -> Float,
) {
    val bench = benchStates.getOrPut(pager) { TestBenchState(pager) }
    // WS 端点始终跟随统一服务器基址（与 API 同源，切换服务器后一起生效）。
    bench.serverUrl = unifiedWsEndpoint(pager)
    renderBench(
        bench, form, pager, twoColumn,
        pageWidth = pageWidth,
        pageHeight = pageHeight,
    )
}

// ============================================================
// 「服务器设置」（侧边栏入口 + 内联表单 + 局域网自动发现）
// ============================================================

private const val KEY_SERVER_URL = "xz.server.baseUrl"
private const val DEFAULT_SERVER = "http://127.0.0.1:8000"

// 已保存地址的连通性三态：初始未知，探测失败才允许广播发现的服务器接管。
private const val SAVED_BASE_UNKNOWN = 0
private const val SAVED_BASE_USABLE = 1
private const val SAVED_BASE_UNUSABLE = 2

/** 服务端 API 基址：读记忆地址（ConnectState 时代同一 SP key），没存过回退本地默认。 */
actual fun pagerBaseUrl(pager: Pager): String =
    prefs(pager).getItem(KEY_SERVER_URL).ifEmpty { DEFAULT_SERVER }

/**
 * 测试台 WS 端点：由**同一**服务器基址推导（http→ws、https→wss + /api/ws）。
 * WS 与 API 只有一个地址源，切换服务器时两者一起变。
 */
fun unifiedWsEndpoint(pager: Pager): String {
    val base = pagerBaseUrl(pager)
    val scheme = if (base.startsWith("https://", true)) "wss" else "ws"
    return "$scheme://${base.substringAfter("://")}/api/ws"
}

/** 局域网自动发现的一个服务器（服务端常驻广播 `_xiaozhi-admin._tcp`，原生桥解析回传）。 */
private data class DiscoveredServer(val name: String, val base: String)

/**
 * 表单/发现态（按 Pager 缓存，重开表单保留上次输入）。
 * ⚠️ observable 只允许在 Kuikly 线程写：发现回调经框架模块回调通道已切回 context 线程。
 */
private class ServerSwitchState(scope: PagerScope) {
    var input by scope.observable(DEFAULT_SERVER)
    var busy by scope.observable(false)

    // Masked connection-form draft only. Applied credential lives in this Pager's params,
    // never SP/keychain or ConfigFormState.expectedToken. Clear the draft on Apply/Cancel.
    var tokenInput by scope.observable("")
    var tokenEdited = false

    /** 当前生效基址（侧边栏入口小字实时展示；observable 在 attr 闭包内读取才被跟踪）。 */
    var current by scope.observable("")

    /** 自动接管只发生一次：首次运行（从未保存过地址）发现到服务器即自动连接。 */
    var autoApplied = false

    /**
     * 已保存地址的连通性探测结果（[SAVED_BASE_UNKNOWN] / [SAVED_BASE_USABLE] /
     * [SAVED_BASE_UNUSABLE]）。服务器换机或换 IP/端口后旧地址会永久失效，
     * 只有确认旧地址连不通，才允许被局域网广播发现的服务器接管——
     * 用户手填的可用地址（含远程服务器）永远优先，不被自动改掉。
     */
    var savedBaseState by scope.observable(SAVED_BASE_UNKNOWN)

    /** 探测只发一次（入口每次渲染都会走到这里）。 */
    var savedBaseProbing = false

    /** 原生 mDNS 浏览是否在跑（observable：面板提示文案随它响应式切换；防重复 start）。 */
    var discoveryOn by scope.observable(false)

    /** vfor 数据源：当前局域网内发现的服务器（每次发现变化整体替换）。 */
    val discovered by scope.observableList<DiscoveredServer>()
}

private val switchStates = mutableMapOf<Pager, ServerSwitchState>()

/**
 * 侧边栏「服务器设置」：入口项（下方小字实时显示当前地址）+ vif 展开的内联表单。
 * 表单 = 自动发现列表（点击即接入）+ 手填地址（保存归一化写回 SP → 收起表单 →
 * [onApplied]（壳层用新基址重跑全部数据加载））。不做浮层，避免跨端点击命中问题。
 */
actual fun ViewContainer<*, *>.macServerSwitchEntry(
    pager: Pager,
    open: () -> Boolean,
    onOpened: () -> Unit,
    onClosed: () -> Unit,
    onApplied: () -> Unit,
) {
    val state = switchStates.getOrPut(pager) { ServerSwitchState(pager) }
    if (state.current.isEmpty()) state.current = pagerBaseUrl(pager)
    // 启动即尝试自动连接：从未配置过地址 → 直接开始广播发现（开箱自动连接）；
    // 已有保存地址 → 先探一次 /api/health，确认旧地址连不通才让发现的服务器接管
    // （服务器换机/换 IP 或端口后旧地址会永久失效，不探测则自动发现永远不会生效）。
    if (!state.autoApplied) {
        if (prefs(pager).getItem(KEY_SERVER_URL).isEmpty()) {
            startDiscovery(pager, state, onApplied)
        } else {
            probeSavedBase(pager, state, onApplied)
        }
    }
    View {
        attr { flexDirectionColumn() }
        sidebarItem("服务器设置", { false }) {
            state.input = prefs(pager).getItem(KEY_SERVER_URL).ifEmpty { DEFAULT_SERVER }
            state.tokenInput = pager.adminRequestToken()
            state.tokenEdited = false
            state.busy = false
            onOpened()
            // 面板展开期间保持发现（列表实时刷新）；已在跑则复用。
            startDiscovery(pager, state, onApplied)
        }
        Text {
            attr {
                fontSize(AdminType.micro)
                color(AdminColors.textTertiary)
                marginLeft(AdminSpace.md + AdminSpace.sm)
                marginBottom(AdminSpace.xs)
                text("当前：${state.current}")
            }
        }
        // 旧地址连不通时明确说出来：服务器换机/改端口后用户只看「连不上」是猜不到原因的。
        vif({ state.savedBaseState == SAVED_BASE_UNUSABLE && state.discovered.isEmpty() }) {
            Text {
                attr {
                    fontSize(AdminType.micro)
                    color(AdminColors.danger)
                    marginLeft(AdminSpace.md + AdminSpace.sm)
                    marginBottom(AdminSpace.xs)
                    text("当前地址连不通，正在搜索局域网内的服务器…")
                }
            }
        }
    }
    vif({ open() }) {
        View {
            attr {
                flexDirectionColumn()
                marginLeft(AdminSpace.sm)
                marginRight(AdminSpace.sm)
                marginBottom(AdminSpace.sm)
                borderRadius(AdminShape.radiusMd)
                backgroundColor(AdminColors.cardBg)
                border(Border(1f, BorderStyle.SOLID, AdminColors.divider))
                padding(AdminSpace.md)
            }
            Text {
                attr {
                    fontSize(AdminType.caption)
                    fontWeightMedium()
                    color(AdminColors.textPrimary)
                    text("局域网自动发现")
                }
            }
            Text {
                attr {
                    fontSize(AdminType.micro)
                    color(AdminColors.textTertiary)
                    marginTop(2f)
                    text(
                        if (state.discovered.isEmpty()) {
                            if (state.discoveryOn) "正在搜索同网段的小智服务端…" else "未发现（服务端需已启动）"
                        } else {
                            "点击即连接"
                        },
                    )
                }
            }
            // ⚠️ Kuikly 2.29.1：vfor 项闭包必须有且仅有一个孩子节点（本行 View 即唯一孩子）。
            vfor({ state.discovered }) { server ->
                View {
                    attr {
                        flexDirectionRow()
                        alignItemsCenter()
                        marginTop(AdminSpace.xs)
                        paddingTop(AdminSpace.xs)
                        paddingBottom(AdminSpace.xs)
                        paddingLeft(AdminSpace.sm)
                        paddingRight(AdminSpace.sm)
                        borderRadius(AdminShape.radiusSm)
                        backgroundColor(AdminColors.insetBg)
                        cursor(CursorType.POINTER)
                    }
                    event {
                        click {
                            applyBase(pager, state, server.base, onClosed, onApplied,
                                requestToken = tokenForTarget(state, server.base))
                        }
                    }
                    Text {
                        attr {
                            flex(1f)
                            fontSize(AdminType.caption)
                            color(AdminColors.textPrimary)
                            text(server.name)
                        }
                    }
                    Text {
                        attr {
                            fontSize(AdminType.micro)
                            color(AdminColors.textSecondary)
                            text(server.base)
                        }
                    }
                }
            }
            View { attr { height(AdminSpace.sm) } }
            fieldDivider()
            labeledField(
                "服务器地址",
                { state.input },
                {
                    state.input = it
                    if (normalizeBase(it) != state.current && !state.tokenEdited) state.tokenInput = ""
                },
                DEFAULT_SERVER,
            )
            sessionTokenField(state)
            actionRow {
                primaryButton(
                    "保存并重新加载",
                    loading = { state.busy },
                    loadingText = "加载中…",
                ) {
                    val base = normalizeBase(state.input)
                    if (base == null) {
                        state.input = "无效地址"
                        return@primaryButton
                    }
                    state.busy = true
                    // 手动选择是最高优先级：写回 SP 后自动接管永久退场（尊重用户选择）。
                    state.autoApplied = true
                    stopDiscovery(pager, state)
                    applyBase(pager, state, base, onClosed, onApplied,
                        requestToken = tokenForTarget(state, base))
                }
                View { attr { width(AdminSpace.sm) } }
                secondaryButton("取消") {
                    state.tokenInput = ""
                    state.tokenEdited = false
                    onClosed()
                    // 自动接管不再可能（有手填记忆）或用户主动收起且已接管 → 停止常驻浏览。
                    if (!autoDiscoveryNeeded(pager, state)) stopDiscovery(pager, state)
                }
            }
        }
    }
}

/** Official password input; native textInputState keeps the editor IME-safe (no text() echo). */
private fun ViewContainer<*, *>.sessionTokenField(state: ServerSwitchState) {
    fieldLabel("管理令牌（仅本次会话）")
    View {
        attr {
            flexDirectionRow()
            alignItemsCenter()
            height(48f)
            backgroundColor(AdminColors.fieldBg)
            border(Border(1f, BorderStyle.SOLID, AdminColors.divider))
            borderRadius(AdminShape.radiusSm)
            paddingLeft(AdminSpace.sm)
            paddingRight(AdminSpace.sm)
        }
        Input {
            attr {
                flex(1f)
                height(48f)
                backgroundColor(AdminColors.transparent)
                fontSize(AdminType.body)
                color(AdminColors.textPrimary)
                keyboardTypePassword()
                returnKeyTypeDone()
                placeholder("留空不携带管理令牌")
                accessibility("管理令牌，仅本次会话")
                textInputState { TextInputState(text = state.tokenInput) }
            }
            event {
                textInputStateChange { input ->
                    if (input.compositionStart == TextInputState.NO_COMPOSITION && input.text != state.tokenInput) {
                        state.tokenInput = input.text
                        state.tokenEdited = true
                    }
                }
            }
        }
    }
    Text {
        attr {
            fontSize(AdminType.micro)
            color(AdminColors.textTertiary)
            marginTop(AdminSpace.xs)
            text("仅用于当前管理请求，不写入服务端配置或本地存储。切换服务器请重新输入令牌。")
        }
    }
}

/** Never forward a previous server's credential implicitly to another authority. */
private fun tokenForTarget(state: ServerSwitchState, base: String): String =
    if (base == state.current || state.tokenEdited) state.tokenInput else ""

/** 表单内分组细分割线（发现列表与手填区之间）。 */
private fun ViewContainer<*, *>.fieldDivider() {
    View {
        attr {
            height(1f)
            backgroundColor(AdminColors.divider)
            marginBottom(AdminSpace.sm)
        }
    }
}

// ------------------------------------------------------------
// 发现与地址应用
// ------------------------------------------------------------

/** 是否还需要常驻发现：仅在「未自动接管且（从未保存地址 或 保存的地址已连不通）」时为真。 */
private fun autoDiscoveryNeeded(pager: Pager, state: ServerSwitchState): Boolean =
    !state.autoApplied &&
        (prefs(pager).getItem(KEY_SERVER_URL).isEmpty() || state.savedBaseState == SAVED_BASE_UNUSABLE)

/**
 * 探测已保存地址是否还活着（GET /api/health）。连不通 = 服务器换机/换 IP/改端口，
 * 此时才启用广播发现接管，让客户端自己恢复，不用用户去猜新地址。
 */
private fun probeSavedBase(pager: Pager, state: ServerSwitchState, onApplied: () -> Unit) {
    if (state.savedBaseProbing) return
    state.savedBaseProbing = true
    val base = prefs(pager).getItem(KEY_SERVER_URL)
    network(pager).httpRequest("$base/api/health", false, JSONObject(), JSONObject(), timeout = 5) { _, success, _, resp ->
        // 只有明确的 HTTP 2xx 才算可用；超时/连接拒绝/5xx 一律视为失效。
        val usable = success && (resp.statusCode == null || resp.statusCode in 200..299)
        state.savedBaseState = if (usable) SAVED_BASE_USABLE else SAVED_BASE_UNUSABLE
        if (!usable && !state.autoApplied) {
            // 旧地址已死：启动广播发现，命中即接管（saveBase 写回新地址并重载全部数据）。
            startDiscovery(pager, state, onApplied)
        }
    }
}

/** 启动 mDNS 发现（幂等）；回调（已在 Kuikly 线程）整体替换列表并尝试自动接管。 */
private fun startDiscovery(pager: Pager, state: ServerSwitchState, onApplied: () -> Unit) {
    if (state.discoveryOn) return
    state.discoveryOn = true
    pager.acquireModule<XiaoZhiModule>(XiaoZhiModule.MODULE_NAME).startServerDiscovery { result ->
        val servers = parseServers(result)
        state.discovered.clear()
        state.discovered.addAll(servers)
        if (autoDiscoveryNeeded(pager, state)) {
            servers.firstOrNull()?.let { first ->
                state.autoApplied = true
                stopDiscovery(pager, state)
                // Automatic discovery may select an address, never an uncommitted token draft.
                applyBase(pager, state, first.base, onClosed = {}, onApplied = onApplied,
                    requestToken = if (first.base == state.current) pager.adminRequestToken() else "")
            }
        }
    }
}

private fun stopDiscovery(pager: Pager, state: ServerSwitchState) {
    if (!state.discoveryOn) return
    state.discoveryOn = false
    pager.acquireModule<XiaoZhiModule>(XiaoZhiModule.MODULE_NAME).stopServerDiscovery()
}

/** 解析原生回包 {servers:[{name, base}]}；结构异常返回已解析部分（不抛）。 */
private fun parseServers(result: JSONObject?): List<DiscoveredServer> {
    val arr: JSONArray = result?.optJSONArray("servers") ?: return emptyList()
    val out = mutableListOf<DiscoveredServer>()
    for (i in 0 until arr.length()) {
        val o = arr.optJSONObject(i) ?: continue
        val base = o.optString("base", "")
        if (base.isNotEmpty()) out.add(DiscoveredServer(o.optString("name", "小智服务端"), base))
    }
    return out
}

/** 写回地址 → 收起表单 → 通知壳层用新基址重载全部数据（自动接管与手动保存共用）。 */
private fun applyBase(
    pager: Pager,
    state: ServerSwitchState,
    base: String,
    onClosed: () -> Unit,
    onApplied: () -> Unit,
    requestToken: String,
) {
    prefs(pager).setItem(KEY_SERVER_URL, base)
    pager.applyAdminRequestToken(requestToken)
    state.current = base
    state.input = base
    state.tokenInput = ""
    state.tokenEdited = false
    onClosed()
    onApplied()
}

/** 地址归一化（收编自原 ConnectState）：容忍无 scheme / ws(s):// 前缀 / 误带路径，只保留 authority。 */
private fun normalizeBase(raw: String): String? {
    var s = raw.trim()
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
    val authority = s.substring(schemeEnd + 3).split('/', '?', '#').firstOrNull() ?: ""
    if (authority.isEmpty() || authority.startsWith(":")) return null
    return "$scheme://$authority"
}

private fun prefs(pager: Pager) =
    pager.acquireModule<SharedPreferencesModule>(SharedPreferencesModule.MODULE_NAME)

/** 本地时区（NSDateFormatter 默认系统时区）；en_US_POSIX 锁死数字格式防区域变体。 */
actual fun formatEpochSeconds(secs: Long): String {
    if (secs <= 0) return ""
    val fmt = NSDateFormatter().apply {
        dateFormat = "yyyy-MM-dd HH:mm:ss"
        locale = NSLocale.localeWithLocaleIdentifier("en_US_POSIX")
    }
    return fmt.stringFromDate(NSDate.dateWithTimeIntervalSince1970(secs.toDouble()))
}

/** 本地时区当前 UTC 偏移（秒）。 */
actual fun localZoneOffsetSeconds(): Int =
    NSTimeZone.localTimeZone.secondsFromGMTForDate(NSDate()).toInt()

actual fun copyToClipboard(text: String): Boolean = try {
    val pb = NSPasteboard.generalPasteboard
    pb.clearContents()
    pb.setString(text, forType = NSPasteboardTypeString)
    true
} catch (_: Exception) {
    false
}
