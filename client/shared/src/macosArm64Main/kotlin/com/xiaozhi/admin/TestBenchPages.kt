package com.xiaozhi.admin

import com.tencent.kuikly.core.base.ViewContainer
import com.tencent.kuikly.core.directives.velse
import com.tencent.kuikly.core.directives.vif
import com.tencent.kuikly.core.pager.Pager
import com.tencent.kuikly.core.views.DivView
import com.tencent.kuikly.core.views.SelectableOption
import com.tencent.kuikly.core.views.SelectionType
import com.tencent.kuikly.core.views.Text
import com.tencent.kuikly.core.views.View

/**
 * 渲染测试台标签页（tabbedPanel：官方 Tabs + PageList）：连接 / ASR 识别 / TTS 合成 / LLM 对话 四个 tab。
 * 每个 tab 宽屏左侧功能卡、右侧对应配置卡（连接→Server+Audio、ASR→ASR+VAD、TTS→TTS、LLM→LLM），
 * 改配置不必切走，配合顶栏「保存配置」一键写回；窄屏纵向堆叠（twoPane）。
 *
 * ⚠️ 响应式铁律（本项目反复踩坑）：凡依赖 observable 的文案/启用态/分支，必须写在
 * vif/velse 条件闭包或 attr 闭包内——构建期裸读（if/when 直接读 observable）只求值一次，
 * 状态变化后 UI 不会更新（实测「按钮三态不出现」的根因）。本文件所有分支均为 vif/velse 链。
 * ⚠️ 必须在目标容器闭包内**非限定**调用 `renderBench(bench, form, ctx, wide)`：
 * Kuikly 的 View{} DSL 静态绑定到词法作用域最近的 ViewContainer 接收者，
 * 以 `ctx.tabbedPanel(...)` 方式调用会把节点挂到 Pager 根容器，导致布局逃逸。
 * ⚠️ wide 须为 lambda（如 `{ pagerData.pageViewWidth >= 900f }`），在 twoPane 的
 * vif 闭包内读取才能随窗口 resize 响应式重排。
 */
fun ViewContainer<*, *>.renderBench(
    bench: TestBenchState,
    form: ConfigFormState,
    ctx: Pager,
    wide: () -> Boolean,
    pageWidth: () -> Float,
    pageHeight: () -> Float,
) {
    tabbedPanel(
        ui = bench.tabUi,
        pageWidth = pageWidth,
        pageHeight = pageHeight,
        pages = listOf(
            TabPage("连接") { benchConnectPage(bench, form, ctx, wide) },
            TabPage("ASR 识别") { benchAsrPage(bench, form, ctx, wide) },
            TabPage("TTS 合成") { benchTtsPage(bench, form, ctx, wide) },
            TabPage("LLM 对话") { benchLlmPage(bench, form, ctx, wide) },
        ),
    )
}

/** 连接 tab：左=连接卡（地址/token/连接按钮+状态徽标），右=Server + Audio 配置卡。 */
private fun ViewContainer<*, *>.benchConnectPage(
    bench: TestBenchState,
    form: ConfigFormState,
    ctx: Pager,
    wide: () -> Boolean,
) {
    twoPane(
        wide = wide,
        left = {
            groupedCard("连接") {
                labeledField("server url", { bench.serverUrl }, { bench.serverUrl = it }, "ws://127.0.0.1:8000/api/ws")
                labeledField("token（可选）", { bench.token }, { bench.token = it })
                actionRow {
                    // 连接/断开合为一个三态按钮：未连=绿「连接」、已连=红「断开」、连接中=灰底「连接中…」+拦截点击。
                    primaryButton(
                        text = "连接",
                        danger = { bench.connected },
                        loading = { bench.connectionState == "connecting" },
                        loadingText = "连接中…",
                        dynamicText = { if (bench.connected) "断开" else "连接" },
                    ) {
                        if (bench.connected) bench.disconnect(ctx) else bench.connect(ctx)
                    }
                    View { attr { width(AdminSpace.md) } }
                    statusBadge({ bench.connectionState }, { connectionLabel(bench.connectionState) })
                }
            }
        },
        right = {
            serverConfigCard(form)
            audioConfigCard(form)
        },
    )
}

/** ASR 识别 tab：左=识别测试卡（录音→试听→发送识别 + 结果），右=ASR + VAD 配置卡。 */
private fun ViewContainer<*, *>.benchAsrPage(
    bench: TestBenchState,
    form: ConfigFormState,
    ctx: Pager,
    wide: () -> Boolean,
) {
    twoPane(
        wide = wide,
        left = {
            groupedCard("ASR 识别测试") {
                Text {
                    attr {
                        fontSize(AdminType.caption)
                        color(AdminColors.textSecondary)
                        marginTop(AdminSpace.xs)
                        text("流程：录音 → 停止后可反复试听 → 发送识别（SenseVoice 自动检测语言）")
                    }
                }
                actionRow {
                    // 三态按钮：一个按钮承载 开始/停止/重新录音/加载中 全部状态（文案随 asrPhase 响应式切换）。
                    // ⚠️ 按钮行只放按钮：状态徽标在下方独立一行（见后）。
                    primaryButton(
                        text = "开始录音",
                        danger = { bench.asrPhase == "recording" },
                        loading = { bench.asrPhase == "starting" || bench.asrPhase == "recognizing" },
                        dynamicText = {
                            when (bench.asrPhase) {
                                "starting" -> "正在启动…"
                                "recording" -> "停止录音"
                                "recognizing" -> "识别中…"
                                "recorded" -> "重新录音"
                                else -> "开始录音"
                            }
                        },
                    ) {
                        when (bench.asrPhase) {
                            "recording" -> bench.stopRecording(ctx)
                            "recorded", "idle" -> bench.startAsr(ctx)
                        }
                    }
                    // 发送识别：仅在已录（且未在录/未在识别）时出现
                    vif({ bench.asrPhase == "recorded" }) {
                        View { attr { width(AdminSpace.md) } }
                        primaryButton("发送识别") { bench.sendAsr(ctx) }
                    }
                }
                // 状态徽标：独立一行（避免与按钮挤同一行导致横向溢出）
                View {
                    attr {
                        flexDirectionRow()
                        marginTop(AdminSpace.sm)
                    }
                    vif({ bench.asrPhase == "recording" }) {
                        // 录音计秒：文本 lambda 在 attr 闭包内求值，recSeconds（200ms 轮询刷新）驱动实时更新
                        statusBadge({ "recording" }, { "录音中 ${formatSeconds(bench.recSeconds)}" })
                    }
                    velse {
                        vif({ bench.asrPhase == "recognizing" }) {
                            statusBadge({ "busy" }, { "识别中" })
                        }
                        velse {
                            vif({ bench.asrPhase == "recorded" }) {
                                statusBadge({ "connected" }, { "已录 ${formatDuration(bench.recDuration)}，可试听/发送识别" })
                            }
                            velse {
                                statusBadge({ "idle" }, { "空闲" })
                            }
                        }
                    }
                }
                // 录音试听播放器（已录且有波形时显示）
                vif({ bench.recWave.isNotEmpty() }) {
                    waveformPlayer(
                        wave = { bench.recWave },
                        progress = { bench.recProgress },
                        playing = { bench.recPlaying },
                        durationText = { formatDuration(bench.recDuration) },
                        enabled = { bench.asrPhase != "recording" && !bench.ttsBusy },
                    ) { bench.togglePlayback(ctx, "recording") }
                }
                View { attr { height(AdminSpace.md) } }
                groupedCard("识别结果", withDivider = false) {
                    vif({ bench.asrText.isEmpty() }) {
                        Text {
                            attr {
                                fontSize(AdminType.body)
                                color(AdminColors.textTertiary)
                                text("识别结果将显示在此")
                            }
                        }
                    }
                    velse {
                        // 官方文本选中容器：长按进入选中态（渲染层画选区），配「复制」按钮
                        View {
                            ref { bench.asrResultRef = it }
                            attr {
                                selectable(SelectableOption.ENABLE)
                                selectionColor(AdminColors.accent)
                            }
                            event {
                                longPress { p ->
                                    if (p.state == "start") {
                                        bench.startSelection(bench.asrResultRef, p.x, p.y)
                                    }
                                }
                                selectEnd { bench.asrCopyHint = "已选中，点「复制」拷贝选中内容" }
                            }
                            Text {
                                attr {
                                    fontSize(AdminType.body)
                                    color(AdminColors.textPrimary)
                                    text(bench.asrText)
                                }
                            }
                            // 识别耗时：sendAsr 发出 → 收到 stt 的服务端往返（stt 回调携带）
                            vif({ bench.asrElapsedMs > 0 }) {
                                Text {
                                    attr {
                                        fontSize(AdminType.micro)
                                        color(AdminColors.textTertiary)
                                        marginTop(AdminSpace.xs)
                                        text("识别耗时 ${bench.asrElapsedMs} ms（发送→收到结果）")
                                    }
                                }
                            }
                        }
                        View { attr { height(AdminSpace.sm) } }
                        actionRow {
                            secondaryButton("复制") { bench.copyAsr(ctx) }
                            vif({ bench.asrCopyHint.isNotEmpty() }) {
                                View { attr { width(AdminSpace.sm) } }
                                Text {
                                    attr {
                                        fontSize(AdminType.micro)
                                        color(AdminColors.textTertiary)
                                        text(bench.asrCopyHint)
                                    }
                                }
                            }
                        }
                    }
                }
            }
        },
        right = {
            asrConfigCard(form)
            vadConfigCard(form)
        },
    )
}

/** TTS 合成 tab：左=合成测试卡（文字 + 随「合成方式」切换的 语言/音色（本地）或 服务商/发音人（远程）+ 语速 + 波形回放），右=TTS 配置卡。 */
private fun ViewContainer<*, *>.benchTtsPage(
    bench: TestBenchState,
    form: ConfigFormState,
    ctx: Pager,
    wide: () -> Boolean,
) {
    twoPane(
        wide = wide,
        left = {
            groupedCard("TTS 合成测试") {
                // 散文输入：官方 TextArea 多行（可换行）
                labeledTextArea("合成文字", { bench.ttsText }, { bench.ttsText = it }, "输入要合成的文字", height = 100f)
                // ⚠️ 测试卡随配置卡的「合成方式」切换显示对应配置（同一状态源 form）：
                //   本地 → Kokoro 语言/性别/sid 音色；远程 → 服务商 + 讯飞发音人。
                //   远程模式服务端音色由 [tts.xfyun].voice 决定（Kokoro sid 与 zh/en lang 均不适用，
                //   引擎直接忽略），故必须切换显示，否则选了也不生效（实测误导）。
                vif({ form.ttsMode == "local" }) {
                    // 语言：下拉（auto 走服务器默认；zh/en 显式指定）
                    dropdownField(
                        label = "语言",
                        currentLabel = { bench.langOptions.firstOrNull { it.first == bench.ttsLang }?.second ?: bench.ttsLang },
                        options = { bench.langOptions },
                        selectedId = { bench.ttsLang },
                        isOpen = { bench.openDropdown == "lang" },
                        onToggle = { bench.openDropdown = if (bench.openDropdown == "lang") "" else "lang" },
                        onSelect = {
                            bench.ttsLang = it
                            bench.openDropdown = ""
                        },
                    )
                    // 音色性别：下拉（切换时自动落到该组第一个音色，并联动语言 zh/en）
                    dropdownField(
                        label = "音色性别",
                        currentLabel = { bench.genderOptions.firstOrNull { it.first == bench.voiceGender }?.second ?: bench.voiceGender },
                        options = { bench.genderOptions },
                        selectedId = { bench.voiceGender },
                        isOpen = { bench.openDropdown == "gender" },
                        onToggle = { bench.openDropdown = if (bench.openDropdown == "gender") "" else "gender" },
                        onSelect = { g ->
                            bench.selectVoice(g, VoiceCatalog.options(g).firstOrNull()?.id ?: bench.voiceId)
                            // 英文音色配中文 lang 会产生错配音；测试台按性别联动语言，减少无效组合
                            bench.ttsLang = if (g == "en") "en" else "zh"
                            bench.openDropdown = ""
                        },
                    )
                    // 音色：下拉（Kokoro voices.bin 索引，55/45/3 项；选项列表随性别切换增删）
                    dropdownField(
                        label = "音色",
                        currentLabel = { "${bench.voiceId}（sid ${bench.ttsSpeaker} · 共 ${bench.voiceOptions.size} 项）" },
                        options = { bench.voiceOptions },
                        selectedId = { bench.voiceId },
                        isOpen = { bench.openDropdown == "voice" },
                        onToggle = { bench.openDropdown = if (bench.openDropdown == "voice") "" else "voice" },
                        onSelect = { id ->
                            bench.selectVoice(bench.voiceGender, id)
                            bench.openDropdown = ""
                        },
                    )
                }
                velse {
                    // 远程模式：显示远程配置（与右侧 TTS 配置卡绑定同一 form 字段，改后保存即生效）。
                    // 下拉开合用 bench.openDropdown 独立键位——右栏配置卡同屏可见，与 form.openDropdown 互不干扰。
                    dropdownField(
                        label = "服务商",
                        currentLabel = {
                            form.ttsRemoteEngines.firstOrNull { it.first == form.ttsBackend }?.second
                                ?: form.ttsBackend
                        },
                        options = { form.ttsRemoteEngines },
                        selectedId = { form.ttsBackend },
                        isOpen = { bench.openDropdown == "remote_provider" },
                        onToggle = { bench.openDropdown = if (bench.openDropdown == "remote_provider") "" else "remote_provider" },
                        onSelect = {
                            form.ttsBackend = it
                            form.lastRemoteEngine = it
                            form.dirty = true
                            bench.openDropdown = ""
                        },
                    )
                    // 讯飞发音人（backend=xfyun）：分组 + vcn，与配置卡同一状态。
                    // ⚠️ 音色列表依赖凭据：三要素未填齐时隐藏（避免选出必然失败的音色）
                    vif({ form.ttsBackend == "xfyun" && form.xfyunCredsReady() }) {
                        dropdownField(
                            label = "音色分组",
                            currentLabel = {
                                when (form.xfyunVoiceGroup) {
                                    "male" -> "男声"
                                    "custom" -> "自定义（手填 vcn）"
                                    else -> "女声"
                                }
                            },
                            options = { form.xfyunVoiceGroupOptions },
                            selectedId = { form.xfyunVoiceGroup },
                            isOpen = { bench.openDropdown == "xfyun_group" },
                            onToggle = { bench.openDropdown = if (bench.openDropdown == "xfyun_group") "" else "xfyun_group" },
                            onSelect = {
                                form.selectXfyunVoiceGroup(it)
                                bench.openDropdown = ""
                            },
                        )
                        vif({ form.xfyunVoiceGroup != "custom" }) {
                            dropdownField(
                                label = "音色类型",
                                currentLabel = {
                                    when (form.xfyunVoiceType) {
                                        "classic" -> "普通发音人"
                                        "x6" -> "极速拟人"
                                        else -> "全部发音人"
                                    }
                                },
                                options = { form.xfyunVoiceTypeOptions },
                                selectedId = { form.xfyunVoiceType },
                                isOpen = { bench.openDropdown == "xfyun_voice_type" },
                                onToggle = { bench.openDropdown = if (bench.openDropdown == "xfyun_voice_type") "" else "xfyun_voice_type" },
                                onSelect = {
                                    form.selectXfyunVoiceType(it)
                                    bench.openDropdown = ""
                                },
                            )
                            dropdownField(
                                label = "发音人（vcn）",
                                currentLabel = {
                                    form.xfyunVoiceOptions.firstOrNull { it.first == form.xfyunVoice }?.second
                                        ?: form.xfyunVoice
                                },
                                options = { form.xfyunVoiceOptions },
                                selectedId = { form.xfyunVoice },
                                isOpen = { bench.openDropdown == "xfyun_voice" },
                                onToggle = { bench.openDropdown = if (bench.openDropdown == "xfyun_voice") "" else "xfyun_voice" },
                                onSelect = {
                                    form.xfyunVoice = it
                                    form.dirty = true
                                    bench.openDropdown = ""
                                },
                            )
                        }
                        vif({ form.xfyunVoiceGroup == "custom" }) {
                            labeledField("voice（手填 vcn）", { form.xfyunVoice }, { form.xfyunVoice = it; form.dirty = true })
                        }
                    }
                    vif({ form.ttsBackend == "xfyun" && !form.xfyunCredsReady() }) {
                        Text {
                            attr {
                                fontSize(AdminType.caption)
                                color(AdminColors.textTertiary)
                                text("先在右侧 TTS 配置卡填写 app_id / api_key / api_secret 并保存，再点「探测音色目录」生成可用音色列表。")
                            }
                        }
                    }
                    Text {
                        attr {
                            fontSize(AdminType.caption)
                            color(AdminColors.textSecondary)
                            marginTop(AdminSpace.xs)
                            text("密钥在右侧 TTS 配置卡维护；测试台合成会直接使用上方选中的音色（无需先保存配置）")
                        }
                    }
                }
                labeledField("语速", { bench.ttsSpeed }, { bench.ttsSpeed = it })
                actionRow {
                    primaryButton(
                        "合成并播放",
                        loading = { bench.ttsBusy },
                        loadingText = "合成中…",
                        enabled = { bench.asrPhase != "recording" && bench.asrPhase != "starting" && bench.asrPhase != "recognizing" },
                    ) {
                        // 远程模式把测试卡当前选中音色随请求下发（无需先保存配置）
                        bench.speak(ctx, if (form.ttsMode == "remote") form.xfyunVoice else "")
                    }
                    // 服务端回报的引擎名（sherpa/xfyun）；成功合成后可见
                    vif({ bench.ttsEngine.isNotEmpty() }) {
                        View { attr { width(AdminSpace.md) } }
                        statusBadge(
                            { if (bench.ttsError.isNotEmpty()) "error" else "connected" },
                            { "${bench.ttsEngine}${if (bench.ttsError.isNotEmpty()) " · 失败" else " · 就绪"}" },
                        )
                    }
                }
                // 合成失败原因（引擎切换失败/凭据错误等）——服务端 tts_test 结果帧携带
                vif({ bench.ttsError.isNotEmpty() }) {
                    Text {
                        attr {
                            fontSize(AdminType.caption)
                            color(AdminColors.dangerTintText)
                            marginTop(AdminSpace.sm)
                            text("合成失败：${bench.ttsError}")
                        }
                    }
                }
                // TTS 波形回放（合成完成后显示，可反复回听；播放进度流过点亮）
                vif({ bench.ttsReady && bench.ttsWave.isNotEmpty() }) {
                    waveformPlayer(
                        wave = { bench.ttsWave },
                        progress = { bench.ttsProgress },
                        playing = { bench.ttsPlaying },
                        durationText = { formatDuration(bench.ttsDuration) },
                        enabled = { !bench.ttsBusy },
                    ) { bench.togglePlayback(ctx, "tts") }
                }
            }
        },
        right = {
            ttsConfigCard(form, ctx)
        },
    )
}

/** LLM 对话 tab：左=对话测试卡（提示词/发送 + 回复），右=LLM 配置卡。 */
private fun ViewContainer<*, *>.benchLlmPage(
    bench: TestBenchState,
    form: ConfigFormState,
    ctx: Pager,
    wide: () -> Boolean,
) {
    twoPane(
        wide = wide,
        left = {
            groupedCard("LLM 对话测试") {
                Text {
                    attr {
                        fontSize(AdminType.caption)
                        color(AdminColors.textSecondary)
                        marginTop(AdminSpace.xs)
                        text("流程：发送提示词 → 服务器按最新保存的 [llm] 配置直调 LLM → 返回回复（改配置后无需重启即可验证）")
                    }
                }
                // 散文输入：官方 TextArea 多行（可换行）
                labeledTextArea("测试提示词", { bench.llmPrompt }, { bench.llmPrompt = it }, height = 80f)
                actionRow {
                    // 三态按钮：测试中显示菊花 + 禁用，点击立刻有反馈（不像此前「点了没反应」）。
                    primaryButton(
                        "发送测试",
                        loading = { bench.llmBusy },
                        loadingText = "测试中…",
                    ) { bench.testLlm(ctx) }
                }
                // 状态徽标：独立一行（每态一个 vif 分支——裸读 when 不随状态更新）
                View {
                    attr {
                        flexDirectionRow()
                        marginTop(AdminSpace.sm)
                    }
                    vif({ bench.llmBusy }) {
                        statusBadge({ "busy" }, { "测试中" })
                    }
                    velse {
                        vif({ bench.llmReply.isNotEmpty() && bench.llmOk }) {
                            statusBadge({ "connected" }, { "正常 · ${bench.llmElapsedMs}ms" })
                        }
                        velse {
                            vif({ bench.llmReply.isNotEmpty() }) {
                                statusBadge({ "error" }, { "失败" })
                            }
                            velse {
                                statusBadge({ "idle" }, { "空闲" })
                            }
                        }
                    }
                }
                View { attr { height(AdminSpace.md) } }
                groupedCard("LLM 回复", withDivider = false) {
                    vif({ bench.llmReply.isEmpty() }) {
                        Text {
                            attr {
                                fontSize(AdminType.body)
                                color(AdminColors.textTertiary)
                                text("回复将显示在此（右侧配置卡可修改 backend/api_base/api_key/model）")
                            }
                        }
                    }
                    velse {
                        // 官方文本选中容器：长按进入选中态（渲染层画选区），配「复制」按钮
                        View {
                            ref { bench.llmReplyRef = it }
                            attr {
                                selectable(SelectableOption.ENABLE)
                                selectionColor(AdminColors.accent)
                            }
                            event {
                                longPress { p ->
                                    if (p.state == "start") {
                                        bench.startSelection(bench.llmReplyRef, p.x, p.y)
                                    }
                                }
                                selectEnd { bench.llmCopyHint = "已选中，点「复制」拷贝选中内容" }
                            }
                            Text {
                                attr {
                                    fontSize(AdminType.body)
                                    color(if (bench.llmOk) AdminColors.textPrimary else AdminColors.danger)
                                    text(bench.llmReply)
                                }
                            }
                            // 服务端直调耗时：llm_test 回包携带（含到 LLM API 的网络往返）
                            vif({ bench.llmOk && bench.llmElapsedMs > 0 }) {
                                Text {
                                    attr {
                                        fontSize(AdminType.micro)
                                        color(AdminColors.textTertiary)
                                        marginTop(AdminSpace.xs)
                                        text("服务端耗时 ${bench.llmElapsedMs} ms")
                                    }
                                }
                            }
                        }
                        View { attr { height(AdminSpace.sm) } }
                        actionRow {
                            secondaryButton("复制") { bench.copyLlm(ctx) }
                            vif({ bench.llmCopyHint.isNotEmpty() }) {
                                View { attr { width(AdminSpace.sm) } }
                                Text {
                                    attr {
                                        fontSize(AdminType.micro)
                                        color(AdminColors.textTertiary)
                                        text(bench.llmCopyHint)
                                    }
                                }
                            }
                        }
                    }
                }
            }
        },
        right = {
            llmConfigCard(form)
        },
    )
}

/** 语言下拉选项：auto（服务器默认）/ 中文 / 英文（Kokoro 模型仅支持中文与英文）。 */
internal val LANG_OPTIONS: List<Pair<String, String>> = listOf(
    "auto" to "自动（服务器默认）",
    "zh" to "中文（zh）",
    "en" to "英文（en）",
)

/** 秒 → "m:ss" 播放器时间文本。 */
internal fun formatDuration(seconds: Double): String {
    val s = seconds.toInt().coerceAtLeast(0)
    return "${s / 60}:${(s % 60).toString().padStart(2, '0')}"
}

/** 秒 → "12.3s"（录音计秒/短时长的十进制显示）。 */
internal fun formatSeconds(seconds: Double): String {
    val tenth = (seconds * 10).toInt().coerceAtLeast(0)
    return "${tenth / 10}.${tenth % 10}s"
}
