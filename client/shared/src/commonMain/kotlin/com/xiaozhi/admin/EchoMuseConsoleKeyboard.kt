package com.xiaozhi.admin

import androidx.compose.runtime.*
import com.tencent.kuikly.compose.extension.MakeKuiklyComposeNode
import com.tencent.kuikly.compose.ui.Modifier
import com.tencent.kuikly.core.base.Attr
import com.tencent.kuikly.core.base.DeclarativeBaseView
import com.tencent.kuikly.core.base.event.Event
import com.tencent.kuikly.core.nvi.serialization.json.JSONObject

/** Native keyboard capture only. The terminal is rendered entirely by Kuikly, not a WebView. */
@Composable
internal fun EchoMuseConsoleKeyboard(
    view: EchoMuseConsoleKeyboardView, enabled: Boolean, modifier: Modifier,
    onEvent: (JSONObject) -> Unit,
) {
    val currentEvent = rememberUpdatedState(onEvent)
    MakeKuiklyComposeNode(factory = { view }, modifier = modifier,
        viewInit = {
            getViewEvent().input { currentEvent.value(it) }
            getViewAttr().enabled(enabled)
        },
        viewUpdate = { it.getViewAttr().enabled(enabled) })
    DisposableEffect(view) { onDispose { view.dispose() } }
}

internal class EchoMuseConsoleKeyboardView : DeclarativeBaseView<EchoMuseConsoleKeyboardAttr, EchoMuseConsoleKeyboardEvent>() {
    override fun viewName() = "EchoMuseConsoleKeyboardHost"
    override fun createAttr() = EchoMuseConsoleKeyboardAttr()
    override fun createEvent() = EchoMuseConsoleKeyboardEvent()
    fun focus() { performTaskWhenRenderViewDidLoad { renderView?.callMethod("focus", null) } }
    fun dispose() { renderView?.callMethod("dispose", null) }
    override fun removeRenderView() { dispose(); super.removeRenderView() }
}
internal class EchoMuseConsoleKeyboardAttr : Attr() {
    fun enabled(value: Boolean) { "enabled" with value }
}
internal class EchoMuseConsoleKeyboardEvent : Event() {
    fun input(callback: (JSONObject) -> Unit) {
        register("onInput") { (it as? JSONObject)?.let(callback) }
    }
}
