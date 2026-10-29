package com.xiaozhi.admin

import androidx.compose.runtime.Composable
import com.gearui.components.button.Button
import com.gearui.components.button.ButtonTheme
import com.gearui.components.button.ButtonType
import com.gearui.foundation.primitives.Text
import com.gearui.theme.Theme
import com.tencent.kuikly.compose.foundation.layout.Column
import com.tencent.kuikly.compose.foundation.layout.Row
import com.tencent.kuikly.compose.foundation.layout.fillMaxWidth
import com.tencent.kuikly.compose.foundation.layout.padding
import com.tencent.kuikly.compose.ui.Alignment
import com.tencent.kuikly.compose.ui.Modifier
import com.tencent.kuikly.core.pager.Pager
import com.xiaozhi.admin.kit.GearWaveform
import com.xiaozhi.admin.kit.GearUiSpace
import com.xiaozhi.admin.kit.actionRow
import com.xiaozhi.admin.kit.dividerH
import com.xiaozhi.admin.kit.hGap
import com.xiaozhi.admin.kit.labeledDecimalField
import com.xiaozhi.admin.kit.labeledField
import com.xiaozhi.admin.kit.labeledSelect
import com.xiaozhi.admin.kit.labeledTextArea
import com.xiaozhi.admin.kit.statusBadge
import com.xiaozhi.admin.kit.twoPane

/**
 * 测试台 ASR 识别 / TTS 合成两个 tab（macOS）。
 *
 * ## ASR（流程一条不改）
 *
 * 三态按钮承载 开始/停止/重新录音/加载中 全部状态（文案随 `asrPhase` 组合期切换）；
 * 录音中显示计秒徽标（200ms 轮询）；已录后出现 GearWaveform 试听器（进度经模块层
 * 100ms 轮询驱动，见 [TestBenchState.togglePlayback]）；「发送识别」只在已录时出现。
 *
 * ## TTS（合成方式随右栏配置卡切换）
 *
 * 测试卡随 `form.ttsMode` 显示对应配置：本地 → Kokoro 语言/性别/sid 音色；
 * 远程 → 服务商 + 讯飞发音人。远程模式服务端音色由 `[tts.xfyun].voice` 决定
 * （Kokoro sid 与 zh/en lang 均不适用，引擎直接忽略），必须切换显示，否则选了也不生效。
 * 下拉开合归 GearUI `Select` 自身，不再占用 `bench.openDropdown`（该字段零改动保留）。
 */
@Composable
internal fun BenchAsrPage(
    bench: TestBenchState,
    form: ConfigFormState,
    pager: Pager,
    wide: () -> Boolean,
) {
    twoPane(
        wide = wide,
        left = {
            LabeledCard(title = "ASR 识别测试") {
                HintLine("流程：录音 → 停止后可反复试听 → 发送识别（SenseVoice 自动检测语言）")
                actionRow {
                    // 三态按钮：一个按钮承载 开始/停止/重新录音/加载中 全部状态（文案随 asrPhase 切换）。
                    // ⚠️ 按钮行只放按钮：状态徽标在下方独立一行（避免与按钮挤同一行横向溢出）。
                    Button(
                        text = when (bench.asrPhase) {
                            "starting" -> "正在启动…"
                            "recording" -> "停止录音"
                            "recognizing" -> "识别中…"
                            "recorded" -> "重新录音"
                            else -> "开始录音"
                        },
                        onClick = {
                            when (bench.asrPhase) {
                                "recording" -> bench.stopRecording(pager)
                                "recorded", "idle" -> bench.startAsr(pager)
                            }
                        },
                        theme = if (bench.asrPhase == "recording") ButtonTheme.DANGER else ButtonTheme.PRIMARY,
                        type = ButtonType.FILL,
                        loading = bench.asrPhase == "starting" || bench.asrPhase == "recognizing",
                        disabled = bench.asrPhase == "starting" || bench.asrPhase == "recognizing",
                    )
                    // 发送识别：仅在已录（且未在录/未在识别）时出现
                    if (bench.asrPhase == "recorded") {
                        hGap(GearUiSpace.md)
                        Button(
                            text = "发送识别",
                            onClick = { bench.sendAsr(pager) },
                            theme = ButtonTheme.PRIMARY,
                            type = ButtonType.FILL,
                        )
                    }
                }
                // 状态徽标：独立一行
                Row(
                    modifier = Modifier.fillMaxWidth().padding(top = GearUiSpace.sm),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    when {
                        // 录音计秒：recSeconds 由原生 200ms 轮询刷新，读快照即随写重组
                        bench.asrPhase == "recording" ->
                            statusBadge("recording", "录音中 ${formatSeconds(bench.recSeconds)}")
                        bench.asrPhase == "recognizing" -> statusBadge("busy", "识别中")
                        bench.asrPhase == "recorded" ->
                            statusBadge("connected", "已录 ${formatDuration(bench.recDuration)}，可试听/发送识别")
                        else -> statusBadge("idle", "空闲")
                    }
                }
                // 录音试听播放器（已录且有波形时显示）
                if (bench.recWave.isNotEmpty()) {
                    GearWaveform(
                        wave = bench.recWave,
                        progress = bench.recProgress,
                        playing = bench.recPlaying,
                        durationText = formatDuration(bench.recDuration),
                        enabled = bench.asrPhase != "recording" && !bench.ttsBusy,
                        onToggle = { bench.togglePlayback(pager, "recording") },
                    )
                }
                dividerH()
                BenchResultBlock(
                    placeholder = "识别结果将显示在此",
                    body = bench.asrText,
                    bodyError = false,
                    detail = if (bench.asrElapsedMs > 0) "识别耗时 ${bench.asrElapsedMs} ms（发送→收到结果）" else "",
                    copyHint = bench.asrCopyHint,
                    onCopy = { bench.copyAsr(pager) },
                )
            }
        },
        right = {
            Column(modifier = Modifier.fillMaxWidth()) {
                AsrConfigCard(form)
                VadConfigCard(form)
            }
        },
    )
}

@Composable
internal fun BenchTtsPage(
    bench: TestBenchState,
    form: ConfigFormState,
    pager: Pager,
    wide: () -> Boolean,
) {
    twoPane(
        wide = wide,
        left = {
            LabeledCard(title = "TTS 合成测试") {
                labeledTextArea(
                    label = "合成文字",
                    value = bench.ttsText,
                    onValueChange = { bench.ttsText = it },
                    placeholder = "输入要合成的文字",
                    minLines = 3,
                )
                if (form.ttsMode == "local") {
                    BenchTtsLocalPickers(bench)
                } else {
                    BenchTtsRemotePickers(bench, form)
                }
                labeledDecimalField(
                    label = "语速",
                    value = bench.ttsSpeed,
                    onValueChange = { bench.ttsSpeed = it },
                )
                actionRow {
                    Button(
                        text = if (bench.ttsBusy) "合成中…" else "合成并播放",
                        // 远程模式把测试卡当前选中音色随请求下发（无需先保存配置）
                        onClick = { bench.speak(pager, if (form.ttsMode == "remote") form.xfyunVoice else "") },
                        theme = ButtonTheme.PRIMARY,
                        type = ButtonType.FILL,
                        loading = bench.ttsBusy,
                        disabled = bench.ttsBusy ||
                            bench.asrPhase == "recording" ||
                            bench.asrPhase == "starting" ||
                            bench.asrPhase == "recognizing",
                    )
                    // 服务端回报的引擎名（sherpa/xfyun）；成功合成后可见
                    if (bench.ttsEngine.isNotEmpty()) {
                        hGap(GearUiSpace.md)
                        statusBadge(
                            if (bench.ttsError.isNotEmpty()) "error" else "connected",
                            "${bench.ttsEngine}${if (bench.ttsError.isNotEmpty()) " · 失败" else " · 就绪"}",
                        )
                    }
                }
                // 合成失败原因（引擎切换失败/凭据错误等）——服务端 tts_test 结果帧携带
                if (bench.ttsError.isNotEmpty()) {
                    WarnLine("合成失败：${bench.ttsError}", error = true)
                }
                // TTS 波形回放（合成完成后显示，可反复回听；播放进度流过点亮）
                if (bench.ttsReady && bench.ttsWave.isNotEmpty()) {
                    GearWaveform(
                        wave = bench.ttsWave,
                        progress = bench.ttsProgress,
                        playing = bench.ttsPlaying,
                        durationText = formatDuration(bench.ttsDuration),
                        enabled = !bench.ttsBusy,
                        onToggle = { bench.togglePlayback(pager, "tts") },
                    )
                }
            }
        },
        right = {
            Column(modifier = Modifier.fillMaxWidth()) {
                TtsConfigCard(form)
            }
        },
    )
}

/** 本地（Kokoro）模式的三个下拉：语言 / 音色性别 / 音色（sid）。 */
@Composable
private fun BenchTtsLocalPickers(bench: TestBenchState) {
    // 语言：auto 走服务器默认；zh/en 显式指定
    labeledSelect(
        label = "语言",
        selectedId = bench.ttsLang,
        options = bench.langOptions,
        onSelect = { bench.ttsLang = it },
    )
    // 音色性别：切换时自动落到该组第一个音色，并联动语言 zh/en
    labeledSelect(
        label = "音色性别",
        selectedId = bench.voiceGender,
        options = bench.genderOptions,
        onSelect = { g ->
            bench.selectVoice(g, VoiceCatalog.options(g).firstOrNull()?.id ?: bench.voiceId)
            // 英文音色配中文 lang 会产生错配音；测试台按性别联动语言，减少无效组合
            bench.ttsLang = if (g == "en") "en" else "zh"
        },
    )
    // 音色：Kokoro voices.bin 索引（55/45/3 项；选项列表随性别切换增删）
    labeledSelect(
        label = "音色",
        selectedId = bench.voiceId,
        options = bench.voiceOptions,
        onSelect = { id -> bench.selectVoice(bench.voiceGender, id) },
    )
}

/** 远程模式的下拉：服务商 + 讯飞发音人（分组/vcn），与右侧 TTS 配置卡绑定同一 form 字段。 */
@Composable
private fun BenchTtsRemotePickers(bench: TestBenchState, form: ConfigFormState) {
    labeledSelect(
        label = "服务商",
        selectedId = form.ttsBackend,
        options = form.ttsRemoteEngines,
        onSelect = {
            form.ttsBackend = it
            form.lastRemoteEngine = it
            form.dirty = true
        },
    )
    // 讯飞发音人（backend=xfyun）：分组 + vcn，与配置卡同一状态。
    // ⚠️ 音色列表依赖凭据：三要素未填齐时隐藏（避免选出必然失败的音色）
    if (form.ttsBackend == "xfyun" && form.xfyunCredsReady()) {
        labeledSelect(
            label = "音色分组",
            selectedId = form.xfyunVoiceGroup,
            options = form.xfyunVoiceGroupOptions,
            onSelect = { form.selectXfyunVoiceGroup(it) },
        )
        if (form.xfyunVoiceGroup != "custom") {
            labeledSelect(
                label = "音色类型",
                selectedId = form.xfyunVoiceType,
                options = form.xfyunVoiceTypeOptions,
                onSelect = { form.selectXfyunVoiceType(it) },
            )
            labeledSelect(
                label = "发音人（vcn）",
                selectedId = form.xfyunVoice,
                options = form.xfyunVoiceOptions,
                onSelect = {
                    form.xfyunVoice = it
                    form.dirty = true
                },
            )
        }
        if (form.xfyunVoiceGroup == "custom") {
            labeledField(
                label = "voice（手填 vcn）",
                value = form.xfyunVoice,
                onValueChange = {
                    form.xfyunVoice = it
                    form.dirty = true
                },
            )
        }
    }
    if (form.ttsBackend == "xfyun" && !form.xfyunCredsReady()) {
        Text(
            text = "先在右侧 TTS 配置卡填写 app_id / api_key / api_secret 并保存，" +
                "再点「探测音色目录」生成可用音色列表。",
            style = Theme.typography.caption,
            color = Theme.colors.mutedForeground,
            modifier = Modifier.padding(top = GearUiSpace.xs),
        )
    }
    HintLine("密钥在右侧 TTS 配置卡维护；测试台合成会直接使用上方选中的音色（无需先保存配置）")
}
