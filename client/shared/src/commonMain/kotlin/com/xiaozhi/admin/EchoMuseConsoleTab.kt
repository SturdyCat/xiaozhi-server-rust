package com.xiaozhi.admin

import com.xiaozhi.admin.ui.*

import com.tencent.kuikly.core.base.ViewContainer
import com.tencent.kuikly.core.directives.vif
import com.tencent.kuikly.core.pager.Pager
import com.tencent.kuikly.core.views.Scroller
import com.tencent.kuikly.core.views.Text

/**
 * 控制台 tab：emOS 受限 shell 的**行模式终端**（设计 §6.4、PRD §5）。
 *
 * ```
 * ┌──────────────────────────────────────────────────────────┐
 * │ 常驻危险横幅（root + 不支持全屏 TUI + 输出上限）            │ ← PRD §5.6 定稿文案
 * │ 审计提示                                                  │
 * │ 会话状态 / 设备 / 本次会话统计 / 一次性提示                 │
 * │ [insecure] 琥珀警告条                                     │
 * │ [首次进入] 一次性 inline 说明 + 「知道了」                  │
 * │ ┌──────────────────────────────────────────────────────┐ │
 * │ │ 输出区：Scroller + 单个等宽 Text（自动滚底）             │ │
 * │ └──────────────────────────────────────────────────────┘ │
 * │ 命令 [____________________]  ← 官方 Input，回车提交       │
 * │            [执行] [上一条] [下一条]                        │
 * │            [中断] [清屏] [复制] [重新连接]                 │
 * │ 关于行模式（折叠）                                         │
 * └──────────────────────────────────────────────────────────┘
 * ```
 *
 * 为什么输出区是 `Scroller` + **单个** `Text` 而不是 `List`/`LazyColumn`（§6.4 对比表）：
 * 2000 个 item 的数据同步成本远高于一次文本设置；`\r` 覆盖当前行的语义按行拆 item 就断了；
 * `List` 还得自己算 item 高度才能 `scrollToPosition` 到底。代价是每次合并刷新（200 ms）
 * 重排一次文本布局，已在 §11 R2 记过并有降级路径。
 *
 * ⚠️ 本文件的 Kuikly 铁律落点：
 * 1. `text(console.display)`、`enabled = { console.canInput }` 等**所有** observable 读取都在
 *    `attr` / `vif` / 按钮 lambda 里，构建期不求值；
 * 2. 渲染函数在本容器闭包内**非限定**调用（`ctx.echoMuseConsoleTab(…)` 会挂到 Pager 根容器）；
 * 3. 本文件不画版式：行用 `infoRow`/`textNote`、按钮用 `actionRow` + 按钮家族、
 *    输入用 `labeledField`（`Input` 的壳由 `fieldBox` 统一），⛔ 不自造控件。
 */

/** 输出区高度（宽窗）：一屏能看完一段 `dmesg` 的头部，又不把输入行顶出视野。 */
private const val EM_CONSOLE_OUTPUT_HEIGHT = 360f

/** 输出区高度（紧凑窗口）：让出键盘弹出后的可视区，避免输入行被键盘盖住。 */
private const val EM_CONSOLE_OUTPUT_HEIGHT_COMPACT = 240f

/**
 * 「贴底」容差（pt）：`scroll` 事件的 `offsetY` 与理论最大偏移差在它以内就算到底。
 *
 * 不能用 `==` 比较：各平台滚动视图的 `contentInset`/弹性回弹会让 offsetY 与
 * `contentHeight - viewHeight` 有零点几像素的差，严格相等会让自动滚底**永远**判成"用户上翻了"。
 */
private const val EM_CONSOLE_BOTTOM_SLACK = 12f

/** 输出区高度：紧凑窗口降一档（不改变信息，只改可视行数）。 */
private fun echoMuseConsoleOutputHeight(ctx: Pager): Float =
    if (workspaceIsCompact(ctx)) EM_CONSOLE_OUTPUT_HEIGHT_COMPACT else EM_CONSOLE_OUTPUT_HEIGHT

internal fun ViewContainer<*, *>.echoMuseConsoleTab(
    row: EchoMuseDeviceRow,
    state: EchoMuseState,
    toast: ToastState,
    ctx: Pager,
) {
    val console = state.detail.console
    echoMuseConsoleBanners()
    echoMuseConsolePhase(row, console)
    vif({ !console.introSeen }) { echoMuseConsoleIntro(console) }
    echoMuseConsoleOutput(console, ctx)
    // unsupported（Android 未接 WS）不建连、不重试，输入行与动作行整块不给：
    // 给了就是一排永远置灰的死控件，比"没有"更难解释（§6.4 表：按钮列写「—」）
    vif({ console.phase != EM_CONSOLE_UNSUPPORTED }) {
        echoMuseConsoleInput(console)
        echoMuseConsoleActions(console, toast)
    }
    echoMuseConsoleAbout(console)
}

// ============================================================
// 常驻提示与状态
// ============================================================

/**
 * 常驻危险横幅（PRD §5.6 定稿文案，**逐字**）+ 审计提示（PRD §7.2）。
 *
 * 两条都常驻、都不进折叠：这是"root 通道"的唯一长期可见提示。不做命令黑名单
 * （PRD §5.6 明确裁决：黑名单不可靠、误报高），所以提示必须一直在。
 *
 * 用**弱化色**（tertiary）而不是琥珀：PRD §5.6 明确要求常驻条"弱化色"——常驻的
 * 警示一旦用了告警色，就再没有"更严重"可以升级了，用户两天后就会当它不存在；
 * 真正需要抓眼球的两次是「首次进入」和 `insecure`（未设设备 token），那两处用琥珀。
 */
private fun ViewContainer<*, *>.echoMuseConsoleBanners() {
    textNote(ConsoleCopy.ROOT_BANNER, micro = true)
    textNote(ConsoleCopy.IN_USE_HINT, micro = true)
}

/**
 * 状态行：会话状态 + 设备 + 本次会话统计 + 服务端/连接的即时提示。
 *
 * 状态文案全部来自 [EchoMuseConsoleState.phaseText]（§8.3 唯一出处）：服务端 `closed.reason`
 * 与 `error.code` 的翻译只在那里做一次，这里只负责贴出来。
 */
private fun ViewContainer<*, *>.echoMuseConsolePhase(row: EchoMuseDeviceRow, console: EchoMuseConsoleState) {
    infoRow({ "会话状态" }, { console.phaseText }, topGap = AdminSpace.sm)
    infoRow({ "设备" }, { row.deviceId })
    // 断开统计只在会话真的结束过之后出现（`closed.inputs/bytes_out/secs`），排查用
    vif({ console.closingStats.isNotEmpty() }) { infoRow({ "本次会话" }, { console.closingStats }) }
    // `ready.notice` / 缓冲截断 / 输出封顶：服务端与客户端的常驻提示，会话继续、输入仍可用
    vif({ console.notice.isNotEmpty() }) { textNote(text = { console.notice }, micro = true) }
    // 一次性提示：busy / 限流 / 单行超长 / 断开原因（下一次成功操作后清空）
    vif({ console.hint.isNotEmpty() }) {
        textNote(text = { console.hint }, tone = { if (console.canInput) "warn" else "error" }, micro = true)
    }
    // 未设设备 token = 同网段任何人可连入 root 通道（服务端 `ready.insecure`）
    vif({ console.insecure }) { textNote(ConsoleCopy.INSECURE_WARN, tone = "warn", micro = true) }
}

/** 首次进入的一次性 inline 说明（非模态，PRD §5.6 定稿文案）。 */
private fun ViewContainer<*, *>.echoMuseConsoleIntro(console: EchoMuseConsoleState) {
    textNote(ConsoleCopy.FIRST_ENTER, tone = "warn", micro = true)
    actionRow {
        secondaryButton("我知道了，继续", icon = IconToken.Check) { console.dismissIntro() }
    }
}

// ============================================================
// 输出区
// ============================================================

/**
 * 输出区：官方 [Scroller] + **单个**等宽 [Text]。
 *
 * 自动滚底靠官方 `contentSizeChanged`（内容尺寸变化事件）+ `setContentOffset`，
 * **不用 `setTimeout` 轮询**：轮询要么抢在布局前滚（滚不到位），要么多滚一次。
 *
 * 上翻/回底的手感（终端基本行为，漏了就是"看历史时被一直拽回底部"）：
 * - `scroll` 事件里算 `离底距离 = contentHeight - viewHeight - offsetY`，超过
 *   [EM_CONSOLE_BOTTOM_SLACK] 就关掉 `autoScroll`，回到贴底就自动打开；
 * - 只在**值真的变化**时才写 observable——`scroll` 一帧能来好几次，无脑赋值会让
 *   每次滚动都触发一轮属性重算。
 *
 * `viewHeight` 从 `scroll` 事件回填（`contentSizeChanged` 的入参只给内容高，减掉视口高
 * 才是真正的"到底"偏移）。首帧视口高还是 0 时算出的偏移偏大，但滚动视图会自行夹到
 * 最大值，所以不会露白。
 *
 * ⚠️ 不用空格凑列对齐：`ADMIN_MONO_FONT` 在某端可能没命中（回退默认字体），
 * 字符宽度就不再一致；需要列的时候用固定像素宽的 `View`（§6.4）。
 */
private fun ViewContainer<*, *>.echoMuseConsoleOutput(console: EchoMuseConsoleState, ctx: Pager) {
    Scroller {
        // 事件闭包里要回调 scroller 自己的 setContentOffset，先抓住它的 receiver
        val host = this
        // 视口高：`scroll` 事件回填；0 表示还没量过（首帧）
        var viewHeight = 0f
        attr {
            height(echoMuseConsoleOutputHeight(ctx))
            marginTop(AdminSpace.sm)
            padding(AdminSpace.sm)
            borderRadius(AdminShape.radiusSm)
            backgroundColor(AdminColors.insetBg)
        }
        event {
            contentSizeChanged { _, contentHeight ->
                if (!console.autoScroll) return@contentSizeChanged
                host.setContentOffset(0f, (contentHeight - viewHeight).coerceAtLeast(0f), false)
            }
            scroll { params ->
                viewHeight = params.viewHeight
                val maxOffset = (params.contentHeight - params.viewHeight).coerceAtLeast(0f)
                val atBottom = params.offsetY >= maxOffset - EM_CONSOLE_BOTTOM_SLACK
                if (console.autoScroll != atBottom) console.autoScroll = atBottom
            }
        }
        Text {
            attr {
                alignSelfStretch()
                fontFamily(ADMIN_MONO_FONT)
                fontSize(AdminType.micro)
                color(AdminColors.textSecondary)
                text(console.display.ifEmpty { console.emptyText })
            }
        }
    }
    textNote(
        text = {
            if (console.autoScroll) {
                "自动滚底：已开启（往下翻看历史会自动暂停，滚回底部自动恢复）"
            } else {
                "自动滚底：已暂停——你正在看历史输出，滚回底部即恢复"
            }
        },
        micro = true,
    )
    // 非 ready 态还留着字，就是上一次会话的缓冲：必须标明，不能让人当成"现在的状态"
    vif({ console.display.isNotEmpty() && !console.canInput }) {
        textNote("以上是断开前保留的历史输出（重连是一条新会话，不恢复上一个 shell 的进程状态）。", micro = true)
    }
}

// ============================================================
// 输入与动作
// ============================================================

/**
 * 输入行：官方 `Input`（`labeledField` 的壳）+ 回车提交 + 可见「执行」按钮。
 *
 * 为什么必须有「执行」按钮：官方 `Input.inputReturn` 在 **H5 上标注"实现中"**，
 * 浏览器里可能根本不触发；只留回车等于 web 端没有提交入口。
 *
 * 为什么 ↑/↓ 只回填历史、不改草稿：选中历史项才覆盖输入框，未选中时 `historyPrev/Next`
 * 不动 `input`（见 [EchoMuseConsoleState.historyPrev]），否则"打了半条命令误按方向键"就丢了。
 */
private fun ViewContainer<*, *>.echoMuseConsoleInput(console: EchoMuseConsoleState) {
    labeledField(
        label = "命令（回车提交；↑ ↓ 取历史）",
        getValue = { console.input },
        onChange = { console.input = it },
        placeholder = "如 uname -a",
        enabled = { console.canInput },
        onSubmit = { console.submit() },
    )
    // 超长前置提示（`LINE_MAX` 是 4096 **字节**，中文 3 字节/字）：不等服务端拒
    vif({ console.inputTooLong }) {
        textNote(
            "单行命令过长（上限 4096 字节，中文按 3 字节算）：这一行不会发送，请拆成几条。",
            tone = "error",
            micro = true,
        )
    }
    actionRow {
        primaryButton(
            "执行",
            enabled = { console.canSubmit },
            icon = IconToken.Play,
        ) { console.submit() }
        hGap(AdminSpace.sm)
        secondaryButton(
            "上一条",
            enabled = { console.canInput },
            icon = IconToken.ChevronUp,
        ) { console.historyPrev() }
        hGap(AdminSpace.sm)
        secondaryButton(
            "下一条",
            enabled = { console.canInput },
            icon = IconToken.ChevronDown,
        ) { console.historyNext() }
    }
}

/**
 * 动作行：中断 / 清屏 / 复制 / 重新连接。
 *
 * 按钮可见性与 §6.4 表的对应关系（偏差见交付报告）：`中断` 只在 `ready` 可用；
 * `清屏`/`复制` 只要**有输出**就可用——包括 `closed`/`offline` 保留的历史输出，
 * 否则"输出已保留"就没有意义（保留的目的就是让人看完、复制走）；
 * `重新连接` 只在 `offline`/`busy`/`closed` 出现，且**只手工**（自动重连等于悄悄开
 * 一条新的 root shell，服务端还会把它记成一次 `superseded`）。
 */
private fun ViewContainer<*, *>.echoMuseConsoleActions(console: EchoMuseConsoleState, toast: ToastState) {
    actionRow {
        secondaryButton(
            "中断",
            enabled = { console.canInput },
            icon = IconToken.Stop,
        ) { console.interrupt() }
        hGap(AdminSpace.sm)
        // 清屏是**纯本地**：不断会话、不动设备端 sh 与工作目录
        secondaryButton("清屏", enabled = { console.display.isNotEmpty() }) { console.clearBuffer() }
        hGap(AdminSpace.sm)
        secondaryButton(
            "复制",
            enabled = { console.display.isNotEmpty() },
            icon = IconToken.Copy,
        ) {
            if (copyToClipboard(console.copyAll())) toast.show("控制台输出已复制到剪贴板", "ok")
            else toast.show("复制失败：当前平台剪贴板不可用", "error")
        }
        vif({ console.canReconnect }) {
            hGap(AdminSpace.sm)
            primaryButton("重新连接", icon = IconToken.Refresh) { console.reconnect() }
        }
    }
    vif({ console.phase == EM_CONSOLE_OFFLINE }) {
        textNote("设备离线时不会建连。等设备上线并刷新列表后，再点「重新连接」。", micro = true)
    }
    vif({ console.phase == EM_CONSOLE_BUSY }) {
        textNote(
            "同一台设备只有一把操作槽：控制台 / 系统信息 / 日志 / 资产下发互斥。等占用结束后点「重新连接」。",
            micro = true,
        )
    }
}

/**
 * 「关于行模式」折叠说明（PRD §7.1 硬要求：必须把能力差异给用户看）。
 *
 * 默认收起：常驻会挤掉终端本身的高度，而这条信息只在"为什么 vim 打不开"时才需要。
 * 展开按钮用 `appButton` 的 `dynamicText`——`textButton` 的文案是构建期 String，
 * 展开/收起切换不会刷新（Kuikly 只跟踪 lambda 里的 observable 读取）。
 */
private fun ViewContainer<*, *>.echoMuseConsoleAbout(console: EchoMuseConsoleState) {
    actionRow {
        appButton(
            text = "关于行模式",
            variant = ButtonVariant.TEXT,
            dynamicText = { if (console.aboutOpen) "收起行模式说明" else "关于行模式（能做什么 / 不能做什么）" },
            onClick = { console.aboutOpen = !console.aboutOpen },
        )
    }
    vif({ console.aboutOpen }) {
        infoRow({ "能做什么" }, { ConsoleCopy.ABOUT_CAN }, topGap = AdminSpace.sm)
        infoRow({ "不能做什么" }, { ConsoleCopy.ABOUT_CANNOT })
        textNote(ConsoleCopy.ABOUT_NOTE, micro = true)
    }
}
