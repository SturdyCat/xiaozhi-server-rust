package com.xiaozhi.admin

import androidx.compose.runtime.*
import com.gearui.components.empty.EmptyState
import com.gearui.foundation.primitives.GearLazyColumn
import com.gearui.overlay.LocalInputBlockedByOverlay
import com.tencent.kuikly.compose.foundation.clickable
import com.tencent.kuikly.compose.foundation.layout.*
import com.tencent.kuikly.compose.foundation.lazy.rememberLazyListState
import com.tencent.kuikly.compose.ui.Alignment
import com.tencent.kuikly.compose.ui.Modifier
import com.tencent.kuikly.compose.ui.zIndex
import com.tencent.kuikly.compose.ui.layout.onSizeChanged
import com.tencent.kuikly.compose.ui.platform.LocalDensity
import com.tencent.kuikly.compose.ui.unit.IntSize
import com.tencent.kuikly.compose.ui.unit.dp
import com.xiaozhi.admin.kit.*
import kotlinx.coroutines.delay

/** Native Kuikly terminal: upstream's dark monospace body, without a second title or input form. */
@Composable
internal fun EchoMuseConsoleTab(row: EchoMuseDeviceRow, state: EchoMuseState, toast: ToastState,
    apiBase: String, token: String, inputBlocked: Boolean = false, modifier: Modifier = Modifier) {
    val pager = LocalAdminPagerOrThrow()
    if (!IS_MACOS && !pager.pageData.isWeb) {
        EmptyState(message = "当前平台暂不支持设备控制台", description = "请使用 Web 管理台或 macOS 客户端。", modifier = modifier)
        return
    }
    val console = state.detail.console
    val keyboard = remember(row.deviceId, apiBase, token) { EchoMuseConsoleKeyboardView() }
    val blocked = inputBlocked || LocalInputBlockedByOverlay.current
    val enabled = console.canInput && !blocked
    val density = LocalDensity.current
    var size by remember { mutableStateOf(IntSize.Zero) }
    var blink by remember { mutableStateOf(true) }
    var focused by remember(keyboard) { mutableStateOf(false) }
    val list = rememberLazyListState()
    val frame = console.frame
    val lines = frame.history + frame.lines
    DisposableEffect(console, row.deviceId, apiBase, token) {
        console.open(row.deviceId, row.online, apiBase, token)
        onDispose { console.close() }
    }
    SideEffect { console.updateDevice(row.online, row.sessionId) }
    LaunchedEffect(size, density) {
        if (size.width > 0 && size.height > 0) with(density) {
            console.resize((size.width.toDp().value / GearTerminalMetrics.cellWidth).toInt(), (size.height.toDp().value / GearTerminalMetrics.lineHeight).toInt())
        }
    }
    LaunchedEffect(frame, console.autoScroll) {
        if (console.autoScroll && !list.isScrollInProgress && lines.isNotEmpty()) list.scrollToItem(frame.history.size)
    }
    LaunchedEffect(enabled, focused, frame.cursorRow, frame.cursorCol) {
        blink = true
        while (enabled && focused) { delay(500); blink = !blink }
    }
    // Card's decorative border/shadow canvas is appended after its children on H5.
    // Keep this interactive surface above it, inside the modal's own stacking context.
    GearTerminal(modifier = modifier.zIndex(1f).padding(top = 8.dp)) {
        Row(modifier = Modifier.fillMaxWidth().padding(bottom = 4.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(12.dp)) {
            GearTerminalText(console.phaseText, modifier = Modifier.weight(1f), tone = GearTerminalTone.Dim, fill = false)
            if (console.canReconnect) GearTerminalKey("重新连接", !blocked) { console.reconnect() }
            GearTerminalKey("中断", enabled) { console.interrupt(); keyboard.focus() }
            GearTerminalKey("清屏", !blocked) { console.clearBuffer(); keyboard.focus() }
            GearTerminalKey("复制", !blocked) {
                toast.show(if (copyToClipboard(console.copyAll())) "终端输出已复制" else "当前平台无法复制", "info")
            }
        }
        if (console.insecure) GearTerminalText(ConsoleCopy.INSECURE_WARN, tone = GearTerminalTone.Dim)
        if (console.notice.isNotEmpty()) GearTerminalText(console.notice, tone = GearTerminalTone.Dim)
        if (console.hint.isNotEmpty() && console.canInput) GearTerminalText(console.hint, tone = GearTerminalTone.Accent)
        Box(modifier = Modifier.weight(1f).fillMaxWidth().onSizeChanged { size = it }
            .clickable(enabled = enabled) { keyboard.focus() }) {
            GearLazyColumn(modifier = Modifier.fillMaxSize(), state = list, avoidKeyboard = false) {
                items(lines.size) { index ->
                    GearTerminalScreenLine(lines[index],
                        cursorCol = if (enabled && focused && blink && frame.cursorVisible && index == frame.history.size + frame.cursorRow)
                            frame.cursorCol else null)
                }
            }
            EchoMuseConsoleKeyboard(keyboard, enabled, Modifier.size(1.dp)) { event ->
                if (event.text("kind") == "focus") focused = event.optBoolean("focused", false)
                if (enabled && focused) when (event.text("kind")) {
                    "key" -> console.key(event.text("key"), event.optBoolean("ctrl", false),
                        event.optBoolean("alt", false), event.optBoolean("shift", false), event.optBoolean("meta", false))
                    "paste" -> console.type(event.text("data"), paste = true)
                    "text" -> console.type(event.text("data"))
                }
            }
        }
        Row(modifier = Modifier.fillMaxWidth().padding(top = 4.dp), horizontalArrangement = Arrangement.spacedBy(12.dp)) {
            GearTerminalText("${frame.cols} × ${frame.rows} · ${if (console.interactive) "Tab 补全 · ↑↓ 历史 · Ctrl+C 中断" else "回车执行 · ↑↓ 历史"}",
                modifier = Modifier.weight(1f), tone = GearTerminalTone.Dim, fill = false)
            GearTerminalKey(if (console.autoScroll) "跟随输出" else "查看历史", !blocked) { console.autoScroll = !console.autoScroll }
        }
    }
}
