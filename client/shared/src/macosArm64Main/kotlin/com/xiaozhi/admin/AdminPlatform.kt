package com.xiaozhi.admin

import com.tencent.kuikly.core.base.Border
import com.tencent.kuikly.core.base.BorderStyle
import com.tencent.kuikly.core.base.PagerScope
import com.tencent.kuikly.core.base.ViewContainer
import com.tencent.kuikly.core.directives.vif
import com.tencent.kuikly.core.module.Module
import com.tencent.kuikly.core.module.SharedPreferencesModule
import com.tencent.kuikly.core.pager.Pager
import com.tencent.kuikly.core.reactive.handler.observable
import com.tencent.kuikly.core.views.Text
import com.tencent.kuikly.core.views.View
import platform.AppKit.NSPasteboard
import platform.AppKit.NSPasteboardTypeString
import platform.Foundation.NSDate
import platform.Foundation.NSDateFormatter
import platform.Foundation.NSLocale
import platform.Foundation.dateWithTimeIntervalSince1970
import platform.Foundation.localeWithLocaleIdentifier

/** macOS 客户端：测试台、XiaoZhiModule 原生桥与「切换服务器」仅此平台启用。 */
actual val IS_MACOS: Boolean = true

/** 原生渲染器由 textInputState 通道显示；再绑 text() 会打断 IME 组词（见 AdminFormControls）。 */
actual val INPUT_NEEDS_TEXT_PROP: Boolean = false

/** 测试台依赖的原生桥（WS 建连 / 麦克风采集 / 播放 / Opus 编解码，实现在 apps/macosApp）。 */
actual fun Pager.platformExternalModules(): Map<String, Module> =
    mapOf(XiaoZhiModule.MODULE_NAME to XiaoZhiModule())

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
// 「切换服务器」（原 AdminShell 启动连接页的收编版）
// ============================================================

private const val KEY_SERVER_URL = "xz.server.baseUrl"
private const val DEFAULT_SERVER = "http://127.0.0.1:8000"

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

/** 表单态（输入框内容 / 保存中），按 Pager 缓存，重开表单保留上次输入。 */
private class ServerSwitchState(scope: PagerScope) {
    var input by scope.observable(DEFAULT_SERVER)
    var busy by scope.observable(false)
}

private val switchStates = mutableMapOf<Pager, ServerSwitchState>()

/**
 * 侧边栏内联「切换服务器」：入口项 + vif 展开的地址表单（不做浮层，避免跨端点击命中问题）。
 * 保存 = 归一化地址写回 SP → 收起表单 → [onApplied]（壳层用新基址重跑全部数据加载）。
 */
actual fun ViewContainer<*, *>.macServerSwitchEntry(
    pager: Pager,
    open: () -> Boolean,
    onClosed: () -> Unit,
    onApplied: () -> Unit,
) {
    val state = switchStates.getOrPut(pager) { ServerSwitchState(pager) }
    sidebarItem("切换服务器", { false }) {
        state.input = prefs(pager).getItem(KEY_SERVER_URL).ifEmpty { DEFAULT_SERVER }
        state.busy = false
        onClosed()
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
                    text("切换服务器")
                }
            }
            View { attr { height(AdminSpace.xs) } }
            labeledField(
                "服务器地址",
                { state.input },
                { state.input = it },
                DEFAULT_SERVER,
            )
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
                    prefs(pager).setItem(KEY_SERVER_URL, base)
                    state.input = base
                    onClosed()
                    onApplied()
                }
                View { attr { width(AdminSpace.sm) } }
                secondaryButton("取消") { onClosed() }
            }
        }
    }
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

actual fun copyToClipboard(text: String): Boolean = try {
    val pb = NSPasteboard.generalPasteboard
    pb.clearContents()
    pb.setString(text, forType = NSPasteboardTypeString)
    true
} catch (_: Exception) {
    false
}
