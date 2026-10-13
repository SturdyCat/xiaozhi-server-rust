package com.xiaozhi.admin

import com.tencent.kuikly.core.base.ViewBuilder
import com.tencent.kuikly.core.base.ViewContainer
import com.tencent.kuikly.core.directives.vfor
import com.tencent.kuikly.core.directives.vif
import com.tencent.kuikly.core.pager.Pager
import com.tencent.kuikly.core.views.Scroller
import com.tencent.kuikly.core.views.Text
import com.tencent.kuikly.core.views.View

/**
 * 管理台「概览 / 设备 / 会话 / 对话记录 / 用量 / 延迟」六个页面的渲染。
 *
 * 与 `OpsState.kt`（数据）分开：这里是**纯展示**，不含任何取数逻辑，便于把"数据对不对"
 * 与"显示得对不对"分开排查。文件拆分的直接原因是 `AGENTS.md` §5.10（单文件 ≤ 600 行）。
 *
 * ## 贯穿全页的呈现规则（与「总览 / MCP 工具」页同一口径）
 *
 * 1. **只在偏离时打标**：正常/在线不打标，只有异常、降级、断开、被截断才占视觉；
 * 2. **失败必须显示原因**，并给出"怎么修"（`textNote(tone = "error")` + 「刷新」）；
 * 3. **空列表给可行动的解释**：解释"怎么样才会有数据"，而不是让人以为坏了；
 * 4. **口径必须写在页面上**：数据是"自本次服务启动以来"的内存态，页面直接把服务端下发的
 *    `note`/`scope_note` 显示出来——不写清楚的话，"在线 0 台"会被误读成"设备都掉线了"。
 *
 * ⚠️ 渲染函数必须是 `ViewContainer` 扩展，并在目标容器闭包内**非限定**调用
 * （见 `ConfigFormState` 文件头的 Kuikly DSL 绑定说明）。
 */

/** 运维页通用滚动容器：统一左右留白与上下边距（各页只关心卡片本身）。 */
fun ViewContainer<*, *>.opsScroller(content: ViewBuilder) {
    Scroller {
        attr {
            flex(1f)
            paddingLeft(AdminSpace.xl)
            paddingRight(AdminSpace.xl)
            paddingTop(AdminSpace.xxl)
            paddingBottom(AdminSpace.xxxl)
        }
        // ⚠️ ViewBuilder 值须显式传接收者调用（同 tabbedPanel 内 page.content(this)）
        content(this)
    }
}

/** 页内统一的「摘要 + 刷新」头行。 */
private fun ViewContainer<*, *>.opsHeader(summary: () -> String, loading: () -> Boolean, onRefresh: () -> Unit) {
    View {
        attr {
            flexDirectionRow()
            alignItemsCenter()
            marginBottom(AdminSpace.sm)
        }
        Text {
            attr {
                flex(1f)
                fontSize(AdminType.body)
                color(AdminColors.textSecondary)
                text(summary())
            }
        }
        secondaryButton("刷新", loading = loading, loadingText = "刷新中…") { onRefresh() }
    }
}

/** 首次加载失败的就地错误（带"怎么修"）。 */
private fun ViewContainer<*, *>.opsError(message: String) {
    vif({ message.isNotEmpty() }) {
        textNote("$message（确认服务端可达后点「刷新」）", tone = "error", micro = true)
    }
}

/**
 * 首次加载中的骨架占位（P1-4）：两张骨架卡模拟"指标卡"的节奏。
 * 只在 `loading && !loaded` 时显示——已有数据的刷新**不**回到骨架（避免整页闪烁），
 * 刷新反馈由头部「刷新中…」按钮态承担。
 */
private fun ViewContainer<*, *>.opsSkeleton() {
    skeletonCard(rows = 3)
    skeletonCard(rows = 4)
}

// ============================================================
// 概览
// ============================================================

/**
 * 概览页：一眼看到"服务现在什么状态"。
 *
 * 这是本项目此前**完全没有**的一页——管理页打开就是一堆配置字段，无法回答"有几台设备在线、
 * 刚才有没有人在说话、上游是不是在报错"。参考项目 `manager-web` 的 `home.vue` 就是这个位置。
 *
 * 能力清单**不在这里重复投影**：直接复用「配置 → 总览」那张 `overviewCard`（数据源
 * [PluginMetaState]），两处永远一致。本页只加"实时运行数字"这一层。
 *
 * P1-6：顶部增插 [healthBanner]（引擎健康横幅，仅偏离时渲染，可点击跳配置 section）——
 * 部署者不用翻日志就知道"记忆/灵魂/指令/唤醒这些可选能力坏了哪个、怎么修"。
 */
fun ViewContainer<*, *>.renderOpsOverview(
    stats: StatsState,
    meta: PluginMetaState,
    health: EngineHealthState,
    ctx: Pager,
    onJumpSection: (section: String) -> Unit,
) {
    // 就绪度横幅（P1-6）：页面最顶部、实时卡之上；正常时不占任何视觉
    healthBanner(health, onJumpSection)

    opsHeader({ stats.summary.ifEmpty { if (stats.loading) "加载中…" else "—" } }, { stats.loading }) {
        stats.refresh(ctx)
    }
    opsError(stats.loadError)
    vif({ stats.loading && !stats.loaded }) { opsSkeleton() }

    groupedCard("实时") {
        metricRow("在线设备", "${stats.devicesOnline} / ${stats.devicesTotal}", "见过的设备总数在括号内")
        metricRow("活会话", "${stats.sessionsLive}", if (stats.sessionsTest > 0) "含 ${stats.sessionsTest} 条测试台连接" else "设备每次唤醒通常新建一条连接")
        metricRow("缓冲内轮次", "${stats.turnsUser + stats.turnsAssistant}", "用户 ${stats.turnsUser} · 助手 ${stats.turnsAssistant}（受内存环形上界影响，是下限）")
        metricRow("已计量轮次", "${stats.usageTurns}", "上游返回 usage 的轮次数（令牌计量口径）")
        metricRow("进程运行", stats.uptimeText.ifEmpty { "—" }, "注册表是内存态：重启后上面所有累计值清零")
        vif({ stats.usageSummary.isNotEmpty() }) {
            Text {
                attr {
                    fontSize(AdminType.micro)
                    color(AdminColors.textTertiary)
                    marginTop(AdminSpace.xs)
                    text("令牌：${stats.usageSummary}")
                }
            }
        }
    }

    // 能力状态：复用既有卡片（数据源 /api/plugins），不在本页第二次实现
    overviewCard(meta, ctx)

    vif({ stats.scopeNote.isNotEmpty() }) {
        textNote(stats.scopeNote, micro = true)
    }
    if (stats.version.isNotEmpty()) {
        textNote("服务端版本 ${stats.version}", micro = true)
    }
}

/** 概览卡片里的一行指标：名称 + 值 + 一行解释。 */
private fun ViewContainer<*, *>.metricRow(name: String, value: String, hint: String) {
    View {
        attr {
            flexDirectionColumn()
            paddingTop(AdminSpace.xs)
            paddingBottom(AdminSpace.xs)
        }
        View {
            attr {
                flexDirectionRow()
                alignItemsCenter()
            }
            Text {
                attr {
                    flex(1f)
                    fontSize(AdminType.body)
                    color(AdminColors.textSecondary)
                    text(name)
                }
            }
            Text {
                attr {
                    fontSize(AdminType.title)
                    fontWeightMedium()
                    color(AdminColors.textPrimary)
                    text(value)
                }
            }
        }
        textNote(hint, micro = true)
    }
}

// ============================================================
// 设备
// ============================================================

/**
 * 设备页：本进程见过的设备 + 在线状态 + 协商参数 + 上次异常。
 *
 * 参考项目对应物是 `DeviceManagement.vue`。差别：那边有"绑定智能体/解绑/工具调用"等写操作，
 * 本页除「个性化配置」（P1-5，跳配置页以该设备为编辑目标）外**刻意只读**——
 * 其余写操作要动设备侧状态，本期不做（见最终报告的范围说明）。
 */
fun ViewContainer<*, *>.renderOpsDevices(
    devices: DevicesState,
    form: ConfigFormState,
    onEditDevice: (String) -> Unit,
    ctx: Pager,
) {
    opsHeader({ devices.summary.ifEmpty { if (devices.loading) "加载中…" else "—" } }, { devices.loading }) {
        devices.refresh(ctx)
    }
    opsError(devices.loadError)
    vif({ devices.loading && !devices.loaded }) { opsSkeleton() }

    vif({ devices.loaded && devices.devices.size == 0 }) {
        textNote(
            devices.note.ifEmpty {
                "还没有任何设备连过。设备只在**唤醒/按键触发对话**时才建立连接（开机只走 OTA），" +
                    "所以刚部署、或设备一直没被唤醒时这里是空的。"
            },
            micro = true,
        )
    }

    vfor({ devices.devices }) { row ->
        groupedCard(row.deviceId.ifEmpty { "(设备未报 Device-Id)" }) {
            View {
                attr {
                    flexDirectionRow()
                    alignItemsCenter()
                    paddingBottom(AdminSpace.xs)
                }
                Text {
                    attr {
                        flex(1f)
                        fontSize(AdminType.caption)
                        color(AdminColors.textSecondary)
                        text(row.subtitle)
                    }
                }
                // 只在偏离时打标：在线是正常态，不显示徽标；离线/测试台才提示
                if (!row.online) {
                    statusBadge(state = { "idle" }, text = { "已断开" })
                } else if (row.isTest) {
                    statusBadge(state = { "busy" }, text = { "测试台" })
                }
            }
            Text {
                attr {
                    fontSize(AdminType.caption)
                    color(AdminColors.textTertiary)
                    text(row.activity)
                }
            }
            // 上次会话异常：这是"设备为什么突然不出声"的第一现场，必须显示
            if (row.lastError.isNotEmpty()) {
                textNote("上次会话异常：${row.lastError}", tone = "error", micro = true)
            }
            if (row.clientId.isNotEmpty() && row.clientId != row.deviceId) {
                textNote("Client-Id ${row.clientId}", micro = true)
            }
            // 个性化配置入口（P1-5）：按设备覆盖会话内参数（音色/语速/提示词/人格等白名单字段）。
            // 测试台连接不显示（is_test 不是真实设备，个性化无意义）。
            if (!row.isTest && row.deviceId.isNotEmpty()) {
                actionRow {
                    secondaryButton(
                        if (form.editDeviceId == row.deviceId) "正在编辑该设备" else "个性化配置",
                    ) { onEditDevice(row.deviceId) }
                }
            }
            // 设备消息注入（P1-2）：在线的设备会话才能注入（文本作为用户话语走完整管线）；
            // 测试台连接不显示入口（服务端也会拒绝——它走独立请求-响应，语义不通）。
            if (row.online && !row.isTest) {
                actionRow {
                    secondaryButton(
                        if (devices.injectTarget == row.deviceId) "收起" else "注入消息",
                    ) { devices.toggleInject(row.deviceId) }
                }
                vif({ devices.injectTarget == row.deviceId }) {
                    labeledField(
                        "注入文本（作为用户话语走完整 LLM→TTS 管线）",
                        { devices.injectDraft },
                        { devices.injectDraft = it },
                        "例如：现在几点了",
                    )
                    actionRow {
                        primaryButton(
                            "发送",
                            loading = { devices.injectBusy },
                            loadingText = "注入中…",
                        ) { devices.inject(ctx, row.deviceId) }
                    }
                    vif({ devices.injectMsg.isNotEmpty() }) {
                        textNote(
                            devices.injectMsg,
                            tone = if (devices.injectMsg.startsWith("✓")) "tertiary" else "error",
                            micro = true,
                        )
                    }
                }
            }
        }
    }

    vif({ devices.note.isNotEmpty() }) {
        textNote(devices.note, micro = true)
    }
}

// ============================================================
// 会话
// ============================================================

/** 会话页：**此刻**活着的连接与它们所处的阶段（聆听/思考/说话）。 */
fun ViewContainer<*, *>.renderOpsSessions(sessions: SessionsState, ctx: Pager) {
    opsHeader({ sessions.summary.ifEmpty { if (sessions.loading) "加载中…" else "—" } }, { sessions.loading }) {
        sessions.refresh(ctx)
    }
    opsError(sessions.loadError)
    vif({ sessions.loading && !sessions.loaded }) { opsSkeleton() }

    vif({ sessions.loaded && sessions.sessions.size == 0 }) {
        textNote(
            sessions.note.ifEmpty {
                "当前没有活跃连接。设备空闲时会断开 WebSocket（这是固件的省电设计，不是故障），" +
                    "唤醒后会重新连上。"
            },
            micro = true,
        )
    }

    vfor({ sessions.sessions }) { row ->
        groupedCard(row.deviceId.ifEmpty { "(未知设备)" }) {
            View {
                attr {
                    flexDirectionRow()
                    alignItemsCenter()
                    paddingBottom(AdminSpace.xs)
                }
                Text {
                    attr {
                        flex(1f)
                        fontSize(AdminType.caption)
                        color(AdminColors.textSecondary)
                        text(row.subtitle)
                    }
                }
                statusBadge(state = { row.badgeState }, text = { row.phaseText })
            }
            // 最近一次识别到的原话：无需等对话记录刷新就能看到"它刚听到什么"
            if (row.lastUserText.isNotEmpty()) {
                Text {
                    attr {
                        fontSize(AdminType.caption)
                        color(AdminColors.textPrimary)
                        text("最近听到：${row.lastUserText}")
                    }
                }
            }
            if (row.lastError.isNotEmpty()) {
                textNote(row.lastError, tone = "error", micro = true)
            }
            textNote("会话 ${row.sessionId}", micro = true)
        }
    }
}

// ============================================================
// 对话记录
// ============================================================

/**
 * 对话记录页：内存里保留的最近几轮对话。
 *
 * ⚠️ 服务端**不持久化**对话（本期范围明确不做）：这里是环形缓冲，每会话最多 40 条、
 * 最多 32 个会话，进程重启即清空。要长期留存需开启 `[memory]`（走 `gm_messages`）。
 * 页面必须把这个口径说清楚，否则用户会以为丢了数据。
 */
fun ViewContainer<*, *>.renderOpsConversations(conversations: ConversationsState, ctx: Pager) {
    opsHeader({ conversations.summary.ifEmpty { if (conversations.loading) "加载中…" else "—" } }, { conversations.loading }) {
        conversations.refresh(ctx)
    }
    opsError(conversations.loadError)
    vif({ conversations.loading && !conversations.loaded }) { opsSkeleton() }

    groupedCard("按设备筛选") {
        labeledField(
            "设备号（留空 = 全部）",
            { conversations.filterDraft },
            { conversations.filterDraft = it },
            "aa:bb:cc:dd:ee:ff",
        )
        actionRow {
            primaryButton("筛选") { conversations.filterBy(ctx, conversations.filterDraft.trim()) }
            View { attr { width(AdminSpace.sm) } }
            secondaryButton("全部") { conversations.filterBy(ctx, "") }
        }
        if (conversations.deviceFilter.isNotEmpty()) {
            textNote("当前只看：${conversations.deviceFilter}", micro = true)
        }
    }

    vif({ conversations.loaded && conversations.conversations.size == 0 }) {
        textNote(
            conversations.note.ifEmpty {
                "还没有对话记录。设备唤醒后说一句话，这里就会出现一轮问答——" +
                    "注意服务端只保留**最近**的若干轮（内存环形缓冲）。"
            },
            micro = true,
        )
    }

    vfor({ conversations.conversations }) { row ->
        groupedCard(row.sessionId) {
            Text {
                attr {
                    fontSize(AdminType.caption)
                    color(AdminColors.textSecondary)
                    text(row.subtitle)
                }
            }
            for (turn in row.turns) {
                conversationTurn(turn)
            }
            if (row.turns.isEmpty()) {
                textNote("这个会话建立了但还没有对话（设备连上了却一直没说话）。", micro = true)
            }
        }
    }

    vif({ conversations.note.isNotEmpty() }) {
        textNote(conversations.note, micro = true)
    }
}

/** 一轮里的一条：用户/助手用不同色与缩进区分。 */
private fun ViewContainer<*, *>.conversationTurn(turn: TurnRow) {
    View {
        attr {
            flexDirectionColumn()
            marginTop(AdminSpace.sm)
            paddingLeft(AdminSpace.sm)
        }
        View {
            attr {
                flexDirectionRow()
                alignItemsCenter()
            }
            Text {
                attr {
                    flex(1f)
                    fontSize(AdminType.micro)
                    color(if (turn.isUser) AdminColors.textAccent else AdminColors.textTertiary)
                    text(if (turn.isUser) "用户" else "助手")
                }
            }
            if (turn.elapsedText.isNotEmpty()) {
                Text {
                    attr {
                        fontSize(AdminType.micro)
                        color(AdminColors.textTertiary)
                        text(turn.elapsedText)
                    }
                }
            }
        }
        Text {
            attr {
                fontSize(AdminType.body)
                color(AdminColors.textPrimary)
                text(turn.text.ifEmpty { "(空)" })
            }
        }
    }
}

// ============================================================
// 用量
// ============================================================

/**
 * 用量页：把 `GET /api/usage` 接出来。
 *
 * 这个接口**早就实现了**，但管理页一直没有入口（客户端全目录 grep `usage` 零命中），
 * 等于死接口。本页是四项改动里后端零成本的一项。
 */
fun ViewContainer<*, *>.renderOpsUsage(usage: UsageState, ctx: Pager) {
    opsHeader({ usage.summary.ifEmpty { if (usage.loading) "加载中…" else "—" } }, { usage.loading }) {
        usage.refresh(ctx)
    }
    opsError(usage.loadError)
    vif({ usage.loading && !usage.loaded }) { opsSkeleton() }

    groupedCard("累计（自本次启动）") {
        metricRow("已计量轮次", "${usage.globalTurns}", "只有上游返回 usage 的轮次才计入")
        metricRow("上下文压力", "${usage.globalPressure}", "输入侧总量（含缓存读写），**不含输出**：它才是推给模型的上下文规模")
        vif({ usage.globalSummary.isNotEmpty() }) {
            Text {
                attr {
                    fontSize(AdminType.caption)
                    color(AdminColors.textSecondary)
                    marginTop(AdminSpace.xs)
                    text(usage.globalSummary)
                }
            }
        }
        textNote("只记 token，不做金额换算——单价随模型/渠道/时段变化，内置价目表必然过期。", micro = true)
    }

    vif({ usage.sessions.size > 0 }) {
        groupedCard("最近会话") {
            // ObservableList：用 vfor 才会在刷新后重新渲染
            vfor({ usage.sessions }) { row ->
                View {
                    attr {
                        flexDirectionColumn()
                        paddingTop(AdminSpace.sm)
                    }
                    View {
                        attr {
                            flexDirectionRow()
                            alignItemsCenter()
                        }
                        Text {
                            attr {
                                flex(1f)
                                fontSize(AdminType.body)
                                color(AdminColors.textPrimary)
                                text(row.sessionId)
                            }
                        }
                        Text {
                            attr {
                                fontSize(AdminType.caption)
                                color(AdminColors.textSecondary)
                                text("${row.turns} 轮 · 合计 ${row.total}")
                            }
                        }
                    }
                    textNote("输入 ${row.input} · 输出 ${row.output} · 上下文压力 ${row.pressure}", micro = true)
                    dividerH()
                }
            }
        }
    }

    vif({ usage.note.isNotEmpty() }) {
        textNote(usage.note, micro = true)
    }
}

// ============================================================
// 延迟
// ============================================================

/**
 * 延迟页：`GET /api/metrics` 的投影（P0-5 事件总线订阅者写入的聚合）。
 *
 * 回答"整轮对话到底多快"：从用户说完到助手回复完成的 p50/p95/max——这是用户体感最直接的
 * 数字。分模块首包延迟（VAD/ASR/LLM/TTS 各自的第一响应）目前只在测试台与内联测试里测，
 * 正常会话的被动采集未纳入（见服务端 `app/metrics.rs` 模块注释）。
 */
fun ViewContainer<*, *>.renderOpsMetrics(metrics: MetricsState, ctx: Pager) {
    opsHeader({ metrics.summary.ifEmpty { if (metrics.loading) "加载中…" else "—" } }, { metrics.loading }) {
        metrics.refresh(ctx)
    }
    opsError(metrics.loadError)
    vif({ metrics.loading && !metrics.loaded }) { opsSkeleton() }

    groupedCard("整轮延迟（最近 200 轮）") {
        val hasSamples = metrics.turnSamples > 0L
        metricRow("p50", if (hasSamples) "${metrics.turnP50Ms} ms" else "—", "一半的轮次快于这个值")
        metricRow("p95", if (hasSamples) "${metrics.turnP95Ms} ms" else "—", "95% 的轮次快于这个值；与 p50 差距大说明偶发慢")
        metricRow("最慢一轮", if (hasSamples) "${metrics.turnMaxMs} ms" else "—", "窗口内最大值")
        metricRow("样本数", "${metrics.turnSamples}", "窗口覆盖 ${metrics.turnsTotal} 轮（早于窗口的轮次已滚出）")
    }

    groupedCard("会话（自本次启动）") {
        metricRow("建立", "${metrics.sessionsStarted}", "每次唤醒/按键通常新建一条连接")
        metricRow("结束", "${metrics.sessionsEnded}", "两者之差 ≈ 此刻活着的连接数")
    }

    groupedCard("TTS 下行（自本次启动）") {
        metricRow("轮次", "${metrics.ttsRounds}", if (metrics.ttsInterrupted > 0L) "其中 ${metrics.ttsInterrupted} 轮被打断（barge-in）" else "0 表示还没有语音回复")
        metricRow("音频帧", "${metrics.ttsFrames}", "下发的 Opus 帧累计")
        metricRow("音频时长", humanDuration(metrics.ttsAudioMs), "合成并下发的语音总时长")
    }

    groupedCard("异常（自本次启动）") {
        metricRow("次数", "${metrics.errorCount}", if (metrics.errorCount == 0L) "正常" else "详见下方最近一条")
        vif({ metrics.lastError.isNotEmpty() }) {
            textNote("最近异常：${metrics.lastError}", tone = "error", micro = true)
        }
    }

    vif({ metrics.note.isNotEmpty() }) {
        textNote(metrics.note, micro = true)
    }
}
