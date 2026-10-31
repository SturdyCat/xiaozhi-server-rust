package com.xiaozhi.admin

import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.remember
import com.tencent.kuikly.compose.foundation.layout.Arrangement
import com.tencent.kuikly.compose.foundation.layout.Row
import com.tencent.kuikly.compose.foundation.layout.fillMaxWidth
import com.tencent.kuikly.compose.foundation.layout.height
import com.tencent.kuikly.compose.foundation.layout.padding
import com.tencent.kuikly.compose.foundation.lazy.LazyColumn
import com.tencent.kuikly.compose.foundation.lazy.rememberLazyListState
import com.tencent.kuikly.compose.ui.Alignment
import com.tencent.kuikly.compose.ui.Modifier
import com.tencent.kuikly.compose.ui.unit.dp
import com.xiaozhi.admin.kit.GearTerminal
import com.xiaozhi.admin.kit.GearTerminalInput
import com.xiaozhi.admin.kit.GearTerminalKey
import com.xiaozhi.admin.kit.GearTerminalPalette
import com.xiaozhi.admin.kit.GearTerminalText
import com.xiaozhi.admin.kit.GearTerminalTone
import com.xiaozhi.admin.kit.LocalAdminSpacing
import com.xiaozhi.admin.kit.LocalAdminViewport

/**
 * 控制台 tab：官方 dashboard 的 **Shell 终端**（`dashboard.jsx` 的 `Shell` 组件）。
 *
 * ```
 * ┌ 终端块（GearTerminal：深色底/圆角/内边距）────────────────┐
 * │ shell — <deviceId>   [中断] [清屏] [重连] [跟随中]          │ ← 等宽弱化
 * │ ┌ 输出（LazyColumn，等宽，TERM_FG，跟随尾行）────────────┐ │
 * │ │ emos:/ # ls                                           │ │
 * │ └───────────────────────────────────────────────────────┘ │
 * │ ❯ [ 无壳输入行 ................ ]               [ 执行 ]    │
 * │ （状态/提示/审计，等宽弱化，仅在有话说时出现）               │
 * └────────────────────────────────────────────────────────────┘
 * ```
 *
 * ## K2：这块终端必须是深色的
 *
 * 全部颜色取自 `kit/GearTerminal.kt` 的 [GearTerminalPalette]（页面文件里**零色值**）；
 * 背景圆角、等宽 13sp、`❯` 强调色都由 kit 出。用户在浅色管理台里点进控制台，期望看到
 * 深色终端——把正文反色成浅色会直接违背这条视觉约束。
 *
 * ## 与旧 DSL 版的三处差异（均为 A 类，已登记）
 *
 * 1. **输出按行渲染**（`LazyColumn` 逐行 item）：旧版是单个等宽 `Text` + `Scroller`。
 *    `console.display` 已经是 `\r` 覆盖与 ANSI 序列解析后的最终文本（`EchoMuseConsoleText`），
 *    按行拆 item 不改变语义，却换来可用的跟随尾行（旧版靠 `contentSizeChanged` +
 *    `setContentOffset`）；
 * 2. **跟随改为显式开关**：旧版在 `scroll` 事件里算「离底距离」自动开/关 `autoScroll`；
 *    Compose 侧没有等价的滚动回调（本 Kuikly 版本无 `snapshotFlow`），所以顶栏给
 *    「跟随中 / 已暂停」文字键：默认跟随，暂停后新输出不再拽人，再点一下回到底部恢复。
 *    `console.autoScroll` 字段与方法签名零改动；
 * 3. **高度改用视口断点**：旧版按 `workspaceIsCompact(pager)` 取 460/320，
 *    现在读 [LocalAdminViewport]（同一阈值 `WORKSPACE_COMPACT = 700`，值不变）。
 *
 * ⚠️ 运行时行为（WS 会话、回显、自动跟随）本环境无法执行，见交付说明「需本地验证」。
 */
@Composable
internal fun EchoMuseConsoleTab(row: EchoMuseDeviceRow, state: EchoMuseState) {
    val console = state.detail.console
    EmTerminalFrame(row = row, console = console)
    // unsupported（Android/iOS 宿主未接 WS）：整块说明，不给一个永远打不动的空终端
    if (console.phase == EM_CONSOLE_UNSUPPORTED) {
        EmWarn(ConsoleCopy.UNSUPPORTED)
    }
}

/**
 * 终端块本体：顶栏 + 输出 + 输入行 + 状态行。
 *
 * 状态行放在**终端内部最底部**（而不是终端外的说明区）：官方的 `shell — deviceId` 与
 * 断开提示都写在终端里，用户视线一直在这一块；挪到外面就又变成「终端 + 面板」两截。
 */
@Composable
private fun EmTerminalFrame(row: EchoMuseDeviceRow, console: EchoMuseConsoleState) {
    // 紧凑窗口让出键盘弹出后的可视区（值与旧实现一致：460 / 320）
    val viewport = LocalAdminViewport.current
    val terminalHeight = if (viewport.workspaceCompact) 320.dp else 460.dp
    GearTerminal(
        modifier = Modifier
            .fillMaxWidth()
            .padding(top = LocalAdminSpacing.current.sm)
            .height(terminalHeight),
    ) {
        EmTerminalTitleBar(row = row, console = console)
        // weight 由调用处给：输出区吃掉终端块里除顶栏/输入行/状态行之外的全部高度
        EmTerminalOutput(console = console, modifier = Modifier.weight(1f))
        EmTerminalPrompt(console = console)
        EmTerminalStatusLine(console = console)
    }
}

/**
 * 顶栏：`shell — <deviceId>` + 右侧文字动作。
 *
 * 用等宽弱化色，与官方 `term.write('\x1b[2mshell — ${deviceId}\x1b[0m')` 同一语义：
 * 这一行是终端自己的开场白，不是面板标题，所以不设卡片标题样式、不加分隔线。
 */
@Composable
private fun EmTerminalTitleBar(row: EchoMuseDeviceRow, console: EchoMuseConsoleState) {
    Row(
        modifier = Modifier.fillMaxWidth(),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(LocalAdminSpacing.current.sm),
    ) {
        GearTerminalText(
            text = "shell — ${row.deviceId}",
            modifier = Modifier.weight(1f),
            tone = GearTerminalTone.Dim,
            fill = false,
        )
        // 中断 / 清屏 / 重连 / 跟随：终端里的「功能键」，弱化色不抢提示符
        GearTerminalKey(label = "中断", enabled = console.canInput) { console.interrupt() }
        GearTerminalKey(label = "清屏", enabled = console.display.isNotEmpty()) { console.clearBuffer() }
        if (console.canReconnect) {
            GearTerminalKey(label = "重连", enabled = true) { console.reconnect() }
        }
        GearTerminalKey(
            label = if (console.autoScroll) "跟随中" else "已暂停",
            enabled = true,
        ) { console.autoScroll = !console.autoScroll }
    }
}

/**
 * 输出区：`LazyColumn` 逐行 + 等宽 [GearTerminalText]。
 *
 * 跟随尾行：`LaunchedEffect` 只在「行数变化」或「跟随开关变化」时滚动——
 * 不逐帧滚动，也不抢用户的滚动；暂停时新输出照常追加，只是不拽视口。
 */
@Composable
private fun EmTerminalOutput(console: EchoMuseConsoleState, modifier: Modifier = Modifier) {
    val text = console.display.ifEmpty { console.emptyText }
    val lines = remember(text) { text.split('\n') }
    val listState = rememberLazyListState()
    LaunchedEffect(lines.size, console.autoScroll) {
        if (console.autoScroll && lines.isNotEmpty()) {
            listState.scrollToItem(lines.lastIndex)
        }
    }
    LazyColumn(
        modifier = modifier
            .fillMaxWidth()
            .padding(top = LocalAdminSpacing.current.xs),
        state = listState,
    ) {
        items(lines.size) { index ->
            GearTerminalText(text = lines[index])
        }
    }
}

/**
 * 输入行：`❯` 提示符 + 无壳输入行 + `执行`。
 *
 * 这里的 `❯` **只在还没连上/设备不给提示符时**才是唯一的提示符来源；pty 连上之后，
 * 提示符由设备自己的 mksh 打印在输出流里，`❯` 只标「输入从这里开始」，不模拟 shell。
 * 「执行」键是 H5 的兜底：`Input.inputReturn` 官方标注「实现中」，只留回车等于
 * Web 端没有提交入口。
 */
@Composable
private fun EmTerminalPrompt(console: EchoMuseConsoleState) {
    Row(
        modifier = Modifier.fillMaxWidth().padding(top = LocalAdminSpacing.current.xs),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(LocalAdminSpacing.current.xs),
    ) {
        GearTerminalText(
            text = "❯",
            tone = if (console.canInput) GearTerminalTone.Accent else GearTerminalTone.Dim,
            fill = false,
        )
        GearTerminalInput(
            value = console.input,
            onValueChange = { console.input = it },
            modifier = Modifier.weight(1f),
            placeholder = "输入命令，回车执行",
            enabled = console.canInput,
            onSubmit = { console.submit() },
        )
        GearTerminalKey(label = "执行", enabled = console.canSubmit) { console.submit() }
    }
}

/**
 * 状态行：只在该有话说的时候出现（连接态异常、服务端 notice、一次性提示、未设 token）。
 *
 * 正常 `ready` 且无 notice/hint 时整行不渲染——终端里常驻一行「已连接」会把输出区
 * 挤小一圈，而设备提示符本身已经说明了「连上了」。
 */
@Composable
private fun EmTerminalStatusLine(console: EchoMuseConsoleState) {
    if (console.phase == EM_CONSOLE_READY &&
        console.notice.isEmpty() && console.hint.isEmpty() && !console.insecure
    ) {
        return
    }
    GearTerminalText(
        text = buildString {
            append(console.phaseText)
            if (console.notice.isNotEmpty()) append("\n").append(console.notice)
            if (console.hint.isNotEmpty()) append("\n").append(console.hint)
            // 未设设备 token = 同网段任何人可连入 root 通道（服务端 ready.insecure）
            if (console.insecure) append("\n").append(ConsoleCopy.INSECURE_WARN)
        },
        modifier = Modifier.padding(top = LocalAdminSpacing.current.xs),
        tone = if (console.canInput) GearTerminalTone.Dim else GearTerminalTone.Fg,
    )
}
