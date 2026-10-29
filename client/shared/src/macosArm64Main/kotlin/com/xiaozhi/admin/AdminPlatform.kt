package com.xiaozhi.admin

import com.tencent.kuikly.core.module.Module
import com.tencent.kuikly.core.nvi.serialization.json.JSONObject
import com.tencent.kuikly.core.pager.Pager
import platform.AppKit.NSPasteboard
import platform.AppKit.NSPasteboardTypeString

/** macOS 客户端：测试台、XiaoZhiModule 原生桥与「服务器设置」仅此平台启用。 */
actual val IS_MACOS: Boolean = true

/** 测试台依赖的原生桥（WS 建连 / 麦克风采集 / 播放 / Opus 编解码，实现在 apps/macosApp）。 */
actual fun Pager.platformExternalModules(): Map<String, Module> =
    XiaoZhiModule()
        .also { registerSharedBridge(it) } // AdminRealtime 的 PlatformWsSocket 无 Pager，经弱引用取桥
        .let { mapOf(XiaoZhiModule.MODULE_NAME to it) }

/** 局域网 mDNS 发现：转发 XiaoZhiModule（T12 起「服务器设置」渲染层在 commonMain MacServerSwitch.kt）。 */
actual fun Pager.startAdminServerDiscovery(onResult: (JSONObject?) -> Unit) {
    acquireModule<XiaoZhiModule>(XiaoZhiModule.MODULE_NAME).startServerDiscovery(onResult)
}

actual fun Pager.stopAdminServerDiscovery() {
    acquireModule<XiaoZhiModule>(XiaoZhiModule.MODULE_NAME).stopServerDiscovery()
}

/** 服务端 API 基址：读记忆地址（ConnectState 时代同一 SP key），没存过回退本地默认。 */
actual fun pagerBaseUrl(pager: Pager): String =
    adminPrefs(pager).getItem(KEY_SERVER_URL).ifEmpty { DEFAULT_SERVER }

/**
 * 测试台 WS 端点：由**同一**服务器基址推导（http→ws、https→wss + /api/ws）。
 * WS 与 API 只有一个地址源，切换服务器时两者一起变。
 */
fun unifiedWsEndpoint(pager: Pager): String {
    val base = pagerBaseUrl(pager)
    val scheme = if (base.startsWith("https://", true)) "wss" else "ws"
    return "$scheme://${base.substringAfter("://")}/api/ws"
}

actual fun copyToClipboard(text: String): Boolean = try {
    val pb = NSPasteboard.generalPasteboard
    pb.clearContents()
    pb.setString(text, forType = NSPasteboardTypeString)
    true
} catch (_: Exception) {
    false
}
