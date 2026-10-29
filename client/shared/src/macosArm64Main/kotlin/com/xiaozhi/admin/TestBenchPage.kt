package com.xiaozhi.admin

import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.MutableState
import androidx.compose.runtime.mutableStateOf
import com.gearui.components.button.Button
import com.gearui.components.button.ButtonTheme
import com.gearui.components.button.ButtonType
import com.gearui.components.tabs.Tab
import com.gearui.components.tabs.Tabs
import com.gearui.components.tabs.TabsSize
import com.gearui.foundation.primitives.ScrollView
import com.gearui.foundation.primitives.Text
import com.gearui.theme.Theme
import com.tencent.kuikly.compose.foundation.layout.Column
import com.tencent.kuikly.compose.foundation.layout.Row
import com.tencent.kuikly.compose.foundation.layout.fillMaxSize
import com.tencent.kuikly.compose.foundation.layout.fillMaxWidth
import com.tencent.kuikly.compose.foundation.layout.padding
import com.tencent.kuikly.compose.ui.Alignment
import com.tencent.kuikly.compose.ui.Modifier
import com.tencent.kuikly.core.pager.Pager
import com.xiaozhi.admin.contracts.connectionLabel
import com.xiaozhi.admin.kit.GearUiSpace
import com.xiaozhi.admin.kit.LocalAdminViewport
import com.xiaozhi.admin.kit.actionRow
import com.xiaozhi.admin.kit.hGap
import com.xiaozhi.admin.kit.labeledField
import com.xiaozhi.admin.kit.statusBadge
import com.xiaozhi.admin.kit.twoPane

/**
 * 「测试台」页（Compose 版，T12 重写；macOS 专属）。
 *
 * ## 结构
 *
 * ```
 * TestbenchSection（actual，AdminPlatform expect 的 mac 实现；按 Pager 缓存状态）
 *   └─ TestBenchPage       本文件：页头（说明 + 状态行）+ GearUI Tabs + tab 分发
 *       ├─ BenchConnectPage  连接（TestBenchAsrTtsPage.kt 之外，本文件内）
 *       ├─ BenchAsrPage      ASR 识别（TestBenchAsrTtsPage.kt）
 *       ├─ BenchTtsPage      TTS 合成（TestBenchAsrTtsPage.kt）
 *       ├─ BenchLlmPage      LLM 对话（TestBenchLlmNluPage.kt）
 *       └─ BenchNluPage      NLU 意图（TestBenchLlmNluPage.kt）
 * ```
 *
 * ## 与旧 DSL 版（TestBenchPages.kt，已删）的三条硬差异
 *
 * 1. **两个 1x1 原生 `Audio{}` 视图删除**：DSL 平台视图挂不进 Compose 树。播放控制下沉
 *    XiaoZhiModule（`playRecording`/`playTts`/`stopPlayback` + 100ms 轮询
 *    `getAudioState`）——那是 AudioView 引入**之前**的原始路径，属回归而非新造；
 * 2. **右栏配置卡恢复**：`benchConfigHandoff` 占位删除，五个 tab 的右栏直接调
 *    T07/T08 的 Compose 配置卡（与配置页同一份实现，不再两套漂移）；
 * 3. **文本选中降级（R-b）**：官方文本选中容器是 DSL 能力，Compose 侧识别/回复文本
 *    只读 + 「复制」按钮（`copyToClipboard` false 时静默；Android 宿主是 stub 恒 false，
 *    按钮静默无操作——既有行为）。状态层 `copyAsr/copyLlm/copyNlu` 原样复用
 *    （ViewRef 为 null 时回退整段复制，经模块 `copyText` 写 NSPasteboard）。
 *
 * ⚠️ 运行时可验证项（五 tab 全流程、原生播放器命令、mDNS 发现）本环境无法执行，
 * 见交付说明的「需本地验证」清单。
 */

// tab id 常量（GearUI Tabs 的 id = 中文文案在旧 DSL 里等价，这里用稳定 id 便于缓存选中态）。
internal const val BENCH_TAB_CONNECT = "connect"
internal const val BENCH_TAB_ASR = "asr"
internal const val BENCH_TAB_TTS = "tts"
internal const val BENCH_TAB_LLM = "llm"
internal const val BENCH_TAB_NLU = "nlu"

private val BENCH_TABS = listOf(BENCH_TAB_CONNECT, BENCH_TAB_ASR, BENCH_TAB_TTS, BENCH_TAB_LLM, BENCH_TAB_NLU)

private fun benchTabLabel(id: String): String = when (id) {
    BENCH_TAB_CONNECT -> "连接"
    BENCH_TAB_ASR -> "ASR 识别"
    BENCH_TAB_TTS -> "TTS 合成"
    BENCH_TAB_LLM -> "LLM 对话"
    else -> "NLU 意图"
}

/**
 * 测试台状态缓存：切走再切回来连接状态 / 录音不丢（旧 `renderTestbenchSection` 同一口径；
 * Kotlin/Native 无 WeakHashMap，管理台进程内只有一个 admin Pager，普通 map 即可）。
 * tab 选中态同理按 Pager 缓存（快照状态，Compose 读它自动订阅重组）。
 */
private val benchStates = mutableMapOf<Pager, TestBenchState>()
private val benchTabs = mutableMapOf<Pager, MutableState<String>>()

/** [TestbenchSection] 的 macOS actual：状态缓存 + WS 端点跟随统一服务器基址 + 页面挂载。 */
@Composable
actual fun TestbenchSection(page: AdminPage) {
    val pager: Pager = page
    val bench = benchStates.getOrPut(pager) { TestBenchState() }
    // WS 端点始终跟随统一服务器基址（与 API 同源）；key 在 apiBase（refreshAll 时刷新，
    // 「服务器设置」保存后即走这里），切换服务器后 bench.serverUrl 一起生效。
    LaunchedEffect(page.apiBase) { bench.serverUrl = unifiedWsEndpoint(pager) }
    val tab = benchTabs.getOrPut(pager) { mutableStateOf(BENCH_TAB_CONNECT) }
    TestBenchPage(bench = bench, form = page.form, pager = pager, tab = tab)
}

@Composable
private fun TestBenchPage(
    bench: TestBenchState,
    form: ConfigFormState,
    pager: Pager,
    tab: MutableState<String>,
) {
    Column(modifier = Modifier.fillMaxSize()) {
        BenchHeader(bench)
        // 五个 tab 一个都不能藏：GearUI Tabs(scrollable) 窗口再窄只做横向滚动
        Tabs(
            items = BENCH_TABS.map { Tab(id = it, label = benchTabLabel(it)) },
            selectedId = tab.value,
            onSelect = { tab.value = it },
            modifier = Modifier
                .fillMaxWidth()
                .padding(start = GearUiSpace.pagePadding, end = GearUiSpace.pagePadding, top = GearUiSpace.sm),
            scrollable = true,
            size = TabsSize.SMALL,
        )
        ScrollView(modifier = Modifier.weight(1f).fillMaxWidth()) {
            Column(
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(horizontal = GearUiSpace.pagePadding, vertical = GearUiSpace.md),
            ) {
                // 双列阈值与旧版一致（>=900 左右并排，窄屏纵向堆叠）
                val viewport = LocalAdminViewport.current
                when (tab.value) {
                    BENCH_TAB_CONNECT -> BenchConnectPage(bench, form, pager, wide = { viewport.twoColumn })
                    BENCH_TAB_ASR -> BenchAsrPage(bench, form, pager, wide = { viewport.twoColumn })
                    BENCH_TAB_TTS -> BenchTtsPage(bench, form, pager, wide = { viewport.twoColumn })
                    BENCH_TAB_LLM -> BenchLlmPage(bench, form, pager, wide = { viewport.twoColumn })
                    BENCH_TAB_NLU -> BenchNluPage(bench, form, pager, wide = { viewport.twoColumn })
                    else -> HintLine("未知测试台 tab「${tab.value}」，请重启管理台。")
                }
            }
        }
    }
}

/** 页头：标题 + 状态行（statusMsg 由各测试动作回写；空时给一句固定说明）。 */
@Composable
private fun BenchHeader(bench: TestBenchState) {
    Column(
        modifier = Modifier
            .fillMaxWidth()
            .padding(start = GearUiSpace.pagePadding, end = GearUiSpace.pagePadding, top = GearUiSpace.md),
    ) {
        Text(
            text = "测试台",
            style = Theme.typography.titleMedium,
            color = Theme.colors.foreground,
        )
        HintLine(
            if (bench.statusMsg.isNotEmpty()) bench.statusMsg
            else "连接服务端后测试录音识别 / TTS 合成 / LLM 对话 / NLU 意图；右侧配置卡与配置页同源，改后保存即生效。",
        )
    }
}

/**
 * 连接 tab：左=连接卡（服务器地址只读 + token + 三态连接按钮 + 状态徽标），
 * 右=Server + Audio 配置卡（恢复 benchConfigHandoff 占位）。
 */
@Composable
internal fun BenchConnectPage(
    bench: TestBenchState,
    form: ConfigFormState,
    pager: Pager,
    wide: () -> Boolean,
) {
    twoPane(
        wide = wide,
        left = {
            LabeledCard(title = "连接") {
                // server 地址只读展示：与 API 同一来源（侧边栏「服务器设置」），改一处 WS/API 一起变。
                HintLine("服务器：${bench.serverUrl}（与 API 同一地址，经侧边栏「服务器设置」修改）")
                labeledField(
                    label = "token（可选）",
                    value = bench.token,
                    onValueChange = { bench.token = it },
                )
                actionRow {
                    // 连接/断开合为一个三态按钮：未连=主色「连接」、已连=红「断开」、连接中=菊花拦截。
                    Button(
                        text = when {
                            bench.connectionState == "connecting" -> "连接中…"
                            bench.connected -> "断开"
                            else -> "连接"
                        },
                        onClick = { if (bench.connected) bench.disconnect(pager) else bench.connect(pager) },
                        theme = if (bench.connected) ButtonTheme.DANGER else ButtonTheme.PRIMARY,
                        type = ButtonType.FILL,
                        loading = bench.connectionState == "connecting",
                        disabled = bench.connectionState == "connecting",
                    )
                    hGap(GearUiSpace.md)
                    statusBadge(bench.connectionState, connectionLabel(bench.connectionState))
                }
            }
        },
        right = {
            Column(modifier = Modifier.fillMaxWidth()) {
                ServerSection(form)
                AudioConfigCard(form)
            }
        },
    )
}

// ============================================================
// 共享零件
// ============================================================

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

/**
 * 结果块（ASR 识别结果 / LLM 回复 / NLU 详情共用）：只读文本 + 耗时明细行 + 「复制」按钮。
 *
 * R-b 降级口径：旧版是「长按进选中态 → 复制选区」，官方选中容器是 DSL 能力；
 * Compose 侧不做选区，复制固定走状态层的整段回退路径（ViewRef 为 null → `copyText` 全文），
 * 失败静默（Android 宿主 copyToClipboard 为 stub，但本页复制经 macOS 模块 `copyText`，
 * 与 ConsolePage 的 expect 复制不是一条通道）。
 */
@Composable
internal fun BenchResultBlock(
    placeholder: String,
    body: String,
    bodyError: Boolean,
    detail: String,
    copyHint: String,
    onCopy: () -> Unit,
) {
    if (body.isEmpty()) {
        HintLine(placeholder)
        return
    }
    Text(
        text = body,
        modifier = Modifier.padding(top = GearUiSpace.xs),
        style = Theme.typography.bodyMedium,
        color = if (bodyError) Theme.colors.destructive else Theme.colors.foreground,
    )
    if (detail.isNotEmpty()) {
        HintLine(detail)
    }
    Row(
        modifier = Modifier.padding(top = GearUiSpace.sm),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Button(text = "复制", onClick = onCopy, type = ButtonType.OUTLINE)
        if (copyHint.isNotEmpty()) {
            hGap(GearUiSpace.sm)
            Text(
                text = copyHint,
                style = Theme.typography.caption,
                color = Theme.colors.mutedForeground,
            )
        }
    }
}
