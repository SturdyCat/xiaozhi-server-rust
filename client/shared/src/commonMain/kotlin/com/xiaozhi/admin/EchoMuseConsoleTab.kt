package com.xiaozhi.admin

import com.xiaozhi.admin.ui.*

import com.tencent.kuikly.core.base.ViewContainer
import com.tencent.kuikly.core.directives.vif
import com.tencent.kuikly.core.pager.Pager
import com.tencent.kuikly.core.views.Scroller
import com.tencent.kuikly.core.views.Text
import com.tencent.kuikly.core.views.View

/**
 * 控制台 tab：官方 dashboard 的 **Shell 终端**（`dashboard.jsx` 的 `Shell` 组件）。
 *
 * ```
 * ┌ 终端块（TERM_BG / 圆角 / 内边距）────────────────────────┐
 * │ shell — <deviceId>                    [中断] [清屏] [重连] │ ← 等宽弱化
 * │ ┌ 输出（Scroller，等宽，TERM_FG，自动滚底）──────────────┐ │
 * │ │ emos:/ # ls                                         │ │
 * * │ │ ...                                                 │ │
 * │ └─────────────────────────────────────────────────────┘ │
 * │ ❯ [ 无壳输入行 ................ ]            [ 执行 ]      │
 * │ （状态/提示/审计，等宽弱化，仅在有话说时出现）              │
 * └──────────────────────────────────────────────────────────┘
 * ```
 *
 * **与上一版（行模式表单）的区别**：删掉了横幅、状态表格、首次进入说明、「关于行模式」
 * 折叠区、四按钮动作行和「命令」标签输入框——那些把终端变成了「一个能打字的日志查看器」，
 * 也就是反复移植却始终看不出「web shell」的原因。现在整块是一个终端：底色、等宽正文、
 * `❯` 提示符、无边框输入行；设备侧 pty 负责提示符与回显，前端不再自己造提示符。
 *
 * 保留的三个文字动作是**可用性兜底**，不是官方那种纯终端：
 * - `执行`：H5 的 `Input.inputReturn` 官方标注「实现中」，只留回车等于 web 端没有提交入口；
 * - `中断`：等价 Ctrl+C（`ctrl-c` 帧），终端里没键盘焦点时唯一的信号通道；
 * - `清屏 / 重连`：纯本地动作，不重开 shell 会话。
 *
 * ⚠️ Kuikly 铁律：observable 只在 `attr` / `vif` / 按钮 lambda 里读；渲染函数在本容器
 * 闭包内非限定调用；版式只用官方组件（`View`/`Text`/`Scroller`/`Input`），不自造控件。
 */

/** 终端块高度（宽窗）：一屏能看完 `dmesg` 头部，又不把输入行顶出视野。 */
private const val EM_TERM_HEIGHT = 460f

/** 终端块高度（紧凑窗口）：让出键盘弹出后的可视区。 */
private const val EM_TERM_HEIGHT_COMPACT = 320f

/**
 * 「贴底」容差（pt）：`scroll` 事件的 `offsetY` 与理论最大偏移差在它以内就算到底。
 *
 * 不能用 `==`：各平台滚动视图的 `contentInset`/弹性回弹会让两者差零点几像素，
 * 严格相等会让自动滚底**永远**判成「用户上翻了」。
 */
private const val EM_TERM_BOTTOM_SLACK = 12f

private fun echoMuseTerminalHeight(ctx: Pager): Float =
    if (workspaceIsCompact(ctx)) EM_TERM_HEIGHT_COMPACT else EM_TERM_HEIGHT

internal fun ViewContainer<*, *>.echoMuseConsoleTab(
    row: EchoMuseDeviceRow,
    state: EchoMuseState,
    toast: ToastState,
    ctx: Pager,
) {
    val console = state.detail.console
    terminalFrame(row, console, toast, ctx)
    // unsupported（Android/iOS 宿主未接 WS）：整块说明，不给一个永远打不动的空终端
    vif({ console.phase == EM_CONSOLE_UNSUPPORTED }) {
        textNote(ConsoleCopy.UNSUPPORTED, tone = "warn", micro = true)
    }
}

/**
 * 终端块本体：顶栏 + 输出 + 输入行 + 状态行。
 *
 * 状态行放在**终端内部最底部**（而不是终端外的说明区）：官方的 `shell — deviceId` 与
 * 断开提示都写在终端里，用户视线一直在这一块；挪到外面就又变成「终端 + 面板」两截。
 */
private fun ViewContainer<*, *>.terminalFrame(
    row: EchoMuseDeviceRow,
    console: EchoMuseConsoleState,
    toast: ToastState,
    ctx: Pager,
) {
    View {
        attr {
            height(echoMuseTerminalHeight(ctx))
            marginTop(AdminSpace.sm)
            padding(AdminSpace.sm)
            borderRadius(AdminShape.radiusSm)
            backgroundColor(AdminColors.termBg)
            flexDirectionColumn()
        }
        terminalTitleBar(row, console)
        terminalOutput(console)
        terminalPrompt(console)
        terminalStatusLine(console)
    }
}

/**
 * 顶栏：`shell — <deviceId>` + 右侧文字动作。
 *
 * 用等宽弱化色，与官方 `term.write('\x1b[2mshell — ${deviceId}\x1b[0m')` 同一语义：
 * 这一行是终端自己的开场白，不是面板标题，所以不设卡片标题样式、不加分隔线。
 */
private fun ViewContainer<*, *>.terminalTitleBar(
    row: EchoMuseDeviceRow,
    console: EchoMuseConsoleState,
) {
    View {
        attr {
            flexDirectionRow()
            alignItemsCenter()
            height(24f)
        }
        Text {
            attr {
                flex(1f)
                fontFamily(ADMIN_MONO_FONT)
                fontSize(AdminType.micro)
                color(AdminColors.termDim)
                text("shell — ${row.deviceId}")
            }
        }
        // 中断 / 清屏 / 重连：终端里的「功能键」，弱化色不抢提示符
        terminalKey("中断", enabled = { console.canInput }) { console.interrupt() }
        terminalKey("清屏", enabled = { console.display.isNotEmpty() }) { console.clearBuffer() }
        vif({ console.canReconnect }) {
            terminalKey("重连", enabled = { true }) { console.reconnect() }
        }
    }
}

/**
 * 终端里的文字动作键：等宽、弱化色，按下无边框（终端里没有按钮）。
 *
 * `enabled` 是 lambda：Kuikly 只跟踪 `attr`/事件闭包里的 observable 读取，
 * 构建期求值会让置灰状态永远停在上一次构建。
 */
private fun ViewContainer<*, *>.terminalKey(
    label: String,
    enabled: () -> Boolean,
    onClick: () -> Unit,
) {
    Text {
        attr {
            marginLeft(AdminSpace.sm)
            fontFamily(ADMIN_MONO_FONT)
            fontSize(AdminType.micro)
            color(if (enabled()) AdminColors.termAccent else AdminColors.termDim)
            text(label)
        }
        event {
            click { if (enabled()) onClick() }
        }
    }
}

/**
 * 输出区：官方 [Scroller] + **单个**等宽 [Text]。
 *
 * 为什么不是 `List`/`LazyColumn`：`\r` 覆盖当前行、ANSI 序列跨行、2000 行同步成本——
 * 按行拆 item 会把终端的行语义拆碎（详见原实现注释）。
 *
 * 自动滚底靠官方 `contentSizeChanged` + `setContentOffset`，不用 `setTimeout` 轮询：
 * 轮询要么抢在布局前滚（滚不到位），要么多滚一次。
 *
 * 上翻/回底：`scroll` 事件里算 `离底距离`，超过容差就关掉 `autoScroll`，回到底自动打开；
 * 只在值真的变化时才写 observable（一帧能给好几次 `scroll`，无脑赋值会触发多轮重算）。
 */
private fun ViewContainer<*, *>.terminalOutput(console: EchoMuseConsoleState) {
    Scroller {
        // 事件闭包里要回调 scroller 自己的 setContentOffset，先抓住它的 receiver
        val host = this
        // 视口高：`scroll` 事件回填；0 表示还没量过（首帧）
        var viewHeight = 0f
        attr {
            flex(1f)
            marginTop(AdminSpace.xs)
        }
        event {
            contentSizeChanged { _, contentHeight ->
                if (!console.autoScroll) return@contentSizeChanged
                host.setContentOffset(0f, (contentHeight - viewHeight).coerceAtLeast(0f), false)
            }
            scroll { params ->
                viewHeight = params.viewHeight
                val maxOffset = (params.contentHeight - params.viewHeight).coerceAtLeast(0f)
                val atBottom = params.offsetY >= maxOffset - EM_TERM_BOTTOM_SLACK
                if (console.autoScroll != atBottom) console.autoScroll = atBottom
            }
        }
        Text {
            attr {
                alignSelfStretch()
                fontFamily(ADMIN_MONO_FONT)
                fontSize(AdminType.micro)
                color(AdminColors.termFg)
                text(console.display.ifEmpty { console.emptyText })
            }
        }
    }
}

/**
 * 输入行：`❯` 提示符 + 无壳 [terminalInput] + `执行`。
 *
 * 这里的 `❯` **只在还没连上/设备不给提示符时**才是唯一的提示符来源；pty 连上之后，
 * 提示符由设备自己的 mksh 打印在输出流里，`❯` 只标「输入从这里开始」，不模拟 shell。
 */
private fun ViewContainer<*, *>.terminalPrompt(console: EchoMuseConsoleState) {
    View {
        attr {
            flexDirectionRow()
            alignItemsCenter()
            height(32f)
            marginTop(AdminSpace.xs)
        }
        Text {
            attr {
                marginRight(AdminSpace.xs)
                fontFamily(ADMIN_MONO_FONT)
                fontSize(AdminType.body)
                color(if (console.canInput) AdminColors.termAccent else AdminColors.termDim)
                text("❯")
            }
        }
        terminalInput(
            getValue = { console.input },
            onChange = { console.input = it },
            placeholder = "输入命令，回车执行",
            enabled = { console.canInput },
            onSubmit = { console.submit() },
        )
        terminalKey("执行", enabled = { console.canSubmit }) { console.submit() }
    }
}

/**
 * 状态行：只在该有话说的时候出现（连接态异常、服务端 notice、一次性提示、未设 token）。
 *
 * 正常 `ready` 且无 notice/hint 时整行不渲染——终端里常驻一行「已连接」会把输出区
 * 挤小一圈，而设备提示符本身已经说明了「连上了」。
 */
private fun ViewContainer<*, *>.terminalStatusLine(console: EchoMuseConsoleState) {
    vif({
        // 表达式必须写在 lambda 里：构建期求值只会读一次 observable，
        // 之后连接态变了这行也不会出现/消失。
        console.phase != EM_CONSOLE_READY ||
            console.notice.isNotEmpty() || console.hint.isNotEmpty() || console.insecure
    }) {
        Text {
            attr {
                marginTop(AdminSpace.xs)
                fontFamily(ADMIN_MONO_FONT)
                fontSize(AdminType.micro)
                color(if (console.canInput) AdminColors.termDim else AdminColors.warningTintText)
                text(
                    buildString {
                        append(console.phaseText)
                        if (console.notice.isNotEmpty()) append("\n").append(console.notice)
                        if (console.hint.isNotEmpty()) append("\n").append(console.hint)
                        // 未设设备 token = 同网段任何人可连入 root 通道（服务端 ready.insecure）
                        if (console.insecure) append("\n").append(ConsoleCopy.INSECURE_WARN)
                    },
                )
            }
        }
    }
}
