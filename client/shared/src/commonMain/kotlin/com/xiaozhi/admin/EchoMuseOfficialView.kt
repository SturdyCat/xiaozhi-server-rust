package com.xiaozhi.admin

import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.key
import androidx.compose.runtime.remember
import com.gearui.components.empty.EmptyState
import com.tencent.kuikly.compose.extension.MakeKuiklyComposeNode
import com.tencent.kuikly.compose.ui.Modifier
import com.tencent.kuikly.compose.ui.platform.LocalActivity
import com.tencent.kuikly.core.base.Attr
import com.tencent.kuikly.core.base.DeclarativeBaseView
import com.tencent.kuikly.core.base.event.Event
import com.tencent.kuikly.core.pager.Pager

/**
 * Inline official dashboard; [url] is already encoded, including embed/device/token parameters.
 * The caller supplies bounded layout constraints and composes this only for the selected tab.
 * Leaving composition destroys its browsing context (including dashboard WS/console leases).
 * Never launches a browser, a Kuikly page, or an external window; URLs are never logged.
 */
@Composable
internal fun EchoMuseOfficialView(url: String, modifier: Modifier = Modifier) {
    val pager = LocalActivity.current.getPager() as Pager
    if (!IS_MACOS && !pager.pageData.isWeb) {
        EmptyState(
            message = "当前平台不支持内嵌官方管理页",
            description = "请使用 Web 管理台或 macOS 客户端。",
            modifier = modifier,
            icon = {},
        )
        return
    }
    if (url.isBlank()) {
        EmptyState(message = "尚未提供官方管理页地址", modifier = modifier, icon = {})
        return
    }
    // Credentials/device changes must not retain the old document or its sockets.
    key(url) {
        val view = remember { EchoMuseOfficialNativeView() }
        MakeKuiklyComposeNode(
            factory = { view },
            modifier = modifier,
            viewInit = { getViewAttr().url(url) },
            viewUpdate = { it.getViewAttr().url(url) },
        )
        DisposableEffect(view) {
            onDispose { view.disposeDocument() }
        }
    }
}

/** Atomic interop view: ComposeView would flatten away the native view's frame. */
private class EchoMuseOfficialNativeView : DeclarativeBaseView<EchoMuseOfficialViewAttr, Event>() {
    override fun viewName() = "EchoMuseOfficialViewHost"
    override fun createAttr() = EchoMuseOfficialViewAttr()
    override fun createEvent() = Event()

    fun disposeDocument() {
        // Do not queue a lazy task after disposal: no render view means it is already gone.
        renderView?.callMethod("dispose", null)
    }

    override fun removeRenderView() {
        disposeDocument()
        super.removeRenderView()
    }
}

private class EchoMuseOfficialViewAttr : Attr() {
    fun url(value: String) {
        "url" with value
    }
}
