package com.xiaozhi.admin

import androidx.compose.runtime.Composable
import com.gearui.components.button.Button
import com.gearui.components.button.ButtonType
import com.gearui.foundation.primitives.Text
import com.gearui.theme.Theme
import com.tencent.kuikly.compose.foundation.layout.padding
import com.tencent.kuikly.compose.ui.Modifier
import com.tencent.kuikly.compose.ui.unit.dp
import com.xiaozhi.admin.kit.fieldLabelRow
import com.xiaozhi.admin.kit.labeledDecimalField
import com.xiaozhi.admin.kit.labeledField
import com.xiaozhi.admin.kit.labeledIntField
import com.xiaozhi.admin.kit.labeledSelect
import com.xiaozhi.admin.kit.labeledTextArea
import com.xiaozhi.admin.kit.switchRow

/**
 * 语音主流程四张配置卡（ASR / VAD / TTS / LLM）—— Compose 版。
 *
 * ## 文案与交互口径（与迁移前一致）
 *
 * - 字段名只用**中文见名知意**的说法（CPU / API / Token 这类通用术语保留英文），配置 key 不上界面；
 * - 需要额外解释的字段：说明进 `help`（名称旁 ⓘ 弹层），行内只留名字，不堆常驻长段落；
 * - 该功能的控制动作（测试凭据 / 探测音色目录 / 获取模型列表）放在名称右侧，与标题两端对齐；
 * - 每张卡内先给基础字段，再给「显示高级选项」开关，其后的高级字段用 `if (form.showAdvanced)` 展开。
 *
 * ## 与旧 DSL 版的两处结构性差异（都已登记，不是随手改）
 *
 * 1. **下拉不再有展开状态**：旧版用 `dropdownField` + `form.isDropdownOpen("tts_mode")` 这套
 *    自绘面板；现在换成 GearUI `Select`（自带 Popover/Overlay，展开状态归组件）。
 *    `ConfigFormState.openDropdown` 等方法按 T05 硬约束**保留不动**，只是不再被调用。
 * 2. **不再有 `vif`/`vbind` 包裹**：Compose 直接 `if (…)` 读快照状态，读写同一份 `stateVar`，
 *    所以「切合成方式 → 引擎组与字段跟着换」由重组自然完成。
 */

// ============================================================
// ASR
// ============================================================

/** `asr`：识别模型 / 词表 / 语种 / 引擎 + 高级（ITN、线程数）。 */
@Composable
fun AsrConfigCard(form: ConfigFormState) {
    LabeledCard(title = "ASR · 语音识别") {
        // 无 backend 选择：ASR 恒为 SenseVoice（sherpa），mock 已移除
        labeledField(
            label = "识别模型",
            value = form.asrModel,
            onValueChange = {
                form.asrModel = it
                form.markDirty()
            },
            help = "sherpa-onnx 的 SenseVoice 模型文件路径（容器内绝对路径）。换模型只改这里，保存后新会话生效。",
        )
        labeledField(
            label = "Tokens 词表",
            value = form.asrTokens,
            onValueChange = {
                form.asrTokens = it
                form.markDirty()
            },
            help = "与模型配套的 tokens.txt 路径。必须与模型同批下载——混用会让识别结果变成乱码。",
        )
        labeledField(
            label = "识别语种",
            value = form.asrLanguage,
            onValueChange = {
                form.asrLanguage = it
                form.markDirty()
            },
            placeholder = "auto/zh/en/ja/ko/yue",
            help = "auto = 自动判别；明确指定语种可略提准确率。SenseVoice 支持中英日韩粤。",
        )
        labeledField(
            label = "识别引擎",
            value = form.asrProvider,
            onValueChange = {
                form.asrProvider = it
                form.markDirty()
            },
            help = "ASR 提供方，当前固定 SenseVoice：本地 sherpa-onnx 推理，零外网、零按次计费。",
        )
        AdvancedToggleRow(form)
        if (form.showAdvanced) {
            switchRow(
                label = "数字规范化（ITN）",
                checked = form.asrUseItn == "true",
                onToggle = { form.toggleStringBool(form::asrUseItn) },
                help = "开启后「一千二百」输出成 1200、「三点半」输出成 3:30，方便大模型与记忆处理数字；" +
                    "关闭则保留原始汉字。",
            )
            labeledIntField(
                label = "推理线程数",
                value = form.asrNumThreads,
                onValueChange = {
                    form.asrNumThreads = it
                    form.markDirty()
                },
                help = "SenseVoice 推理占用的 CPU 线程数；留空按引擎默认，机器核数少时调小可降低对其他阶段的抢占。",
            )
        }
    }
}

// ============================================================
// VAD
// ============================================================

/** `vad`：人声检测模型与阈值 + 高级（切句时序）。 */
@Composable
fun VadConfigCard(form: ConfigFormState) {
    LabeledCard(title = "VAD · 语音分段") {
        labeledField(
            label = "VAD 模型",
            value = form.vadModel,
            onValueChange = {
                form.vadModel = it
                form.markDirty()
            },
            help = "sherpa-onnx 的 VAD（人声检测）模型路径，决定「什么时候算一句话说完了」。",
        )
        labeledDecimalField(
            label = "语音判定阈值",
            value = form.vadThreshold,
            onValueChange = {
                form.vadThreshold = it
                form.markDirty()
            },
            help = "人声概率高于该值才判为「有人在说」，取值 0~1，常用 0.5。" +
                "调高会漏掉轻声细语，调低会把风扇、空调噪声当成说话。",
        )
        AdvancedToggleRow(form)
        if (form.showAdvanced) {
            labeledDecimalField(
                label = "最短静音（秒）",
                value = form.vadMinSilence,
                onValueChange = {
                    form.vadMinSilence = it
                    form.markDirty()
                },
                help = "连续静音超过该时长即断句：调小响应更快但容易把一句话切成两半，调大更稳但应答变慢。",
            )
            labeledDecimalField(
                label = "最短语音（秒）",
                value = form.vadMinSpeech,
                onValueChange = {
                    form.vadMinSpeech = it
                    form.markDirty()
                },
                help = "短于该时长的声音片段直接丢弃，用于过滤咳嗽、碰撞等瞬时噪声。",
            )
        }
    }
}

// ============================================================
// TTS
// ============================================================

/** `tts`：合成方式（本地/远程）→ 引擎组 → 各自字段；远程讯飞段含凭据与音色探测。 */
@Composable
fun TtsConfigCard(form: ConfigFormState) {
    LabeledCard(title = "TTS · 语音合成") {
        // ===== 第一级：合成方式（本地模型=离线 / 远程服务=在线 API）=====
        labeledSelect(
            label = "合成方式",
            selectedId = form.ttsMode,
            options = form.ttsModeOptions,
            onSelect = { mode ->
                if (mode != form.ttsMode) {
                    form.ttsMode = mode
                    // 切方式即切引擎组：本地→当前本地引擎；远程→上次使用的远程服务商
                    form.ttsBackend = if (mode == "remote") form.lastRemoteEngine else "kokoro"
                    form.markDirty()
                }
            },
            help = "本地模型 = 在服务器（容器）内离线合成，零外网、零按次计费，语气略平；" +
                "远程服务 = 调用在线合成 API，音色更自然但需网络与服务商账号。",
        )
        // ===== 缓存（本地/远程通用）：按句缓存已编码下行 Opus 帧，命中时零合成延迟 =====
        AdvancedToggleRow(form)
        if (form.showAdvanced) {
            labeledIntField(
                label = "结果缓存条目数",
                value = form.ttsCacheEntries,
                onValueChange = {
                    form.ttsCacheEntries = it
                    form.markDirty()
                },
                placeholder = "256",
                // 同段内档位不一致的例外：`[tts].engine` 是新会话生效，但缓存大小只能重启生效。
                // 这类"例外字段"必须用字段级 note（段级提示会把整段说成需重启）。
                note = form.schema.fieldNote("tts", "cache_entries"),
                help = "按句缓存已合成的下行音频（跨会话、跨设备共享），命中即刻出声、零合成延迟；" +
                    "0 = 关闭缓存。「唤醒问候语落盘复用」依赖它，关掉会一并失效。",
            )
        }

        // ===== 第二级：本地引擎 + Kokoro 参数 =====
        if (form.ttsMode != "remote") {
            labeledSelect(
                label = "本地引擎",
                selectedId = form.ttsBackend,
                options = form.ttsLocalEngines,
                onSelect = {
                    form.ttsBackend = it
                    form.markDirty()
                },
            )
            labeledField(
                label = "模型文件",
                value = form.ttsModel,
                onValueChange = {
                    form.ttsModel = it
                    form.markDirty()
                },
                help = "Kokoro ONNX 模型路径；INT8 量化版体积小、速度更快，音质略降。",
            )
            labeledField(
                label = "音色文件",
                value = form.ttsVoices,
                onValueChange = {
                    form.ttsVoices = it
                    form.markDirty()
                },
                help = "音色向量表（voices 二进制）路径，必须与模型版本配套，否则合成会报错或串音。",
            )
            labeledField(
                label = "Tokens 词表",
                value = form.ttsTokens,
                onValueChange = {
                    form.ttsTokens = it
                    form.markDirty()
                },
                help = "与模型配套的 tokens.txt 路径，负责把音素映射成模型输入 id。",
            )
            labeledField(
                label = "模型数据目录",
                value = form.ttsDataDir,
                onValueChange = {
                    form.ttsDataDir = it
                    form.markDirty()
                },
                help = "模型自带的 data_dir（配置与字符表）所在目录。",
            )
            labeledField(
                label = "词典目录",
                value = form.ttsDictDir,
                onValueChange = {
                    form.ttsDictDir = it
                    form.markDirty()
                },
                help = "中文分词与多音字词典目录（jieba）。目录缺失会导致多音字读错，不影响启动。",
            )
            labeledField(
                label = "自定义发音词典",
                value = form.ttsLexicon,
                onValueChange = {
                    form.ttsLexicon = it
                    form.markDirty()
                },
                help = "lexicon 文件路径，每行「词 拼音」，用于修正人名的多音字读音；优先级高于内置词典。",
            )
            labeledField(
                label = "合成语种",
                value = form.ttsLang,
                onValueChange = {
                    form.ttsLang = it
                    form.markDirty()
                },
                help = "发音规则所用语种代码（如 zh、en）；需与模型支持的语种一致。",
            )
            labeledIntField(
                label = "说话人编号",
                value = form.ttsSpeaker,
                onValueChange = {
                    form.ttsSpeaker = it
                    form.markDirty()
                },
                help = "多说话人模型里的 speaker id（从 0 开始）；单说话人模型填 0。",
            )
            labeledDecimalField(
                label = "语速",
                value = form.ttsSpeed,
                onValueChange = {
                    form.ttsSpeed = it
                    form.markDirty()
                },
                help = "大于 1 更快、小于 1 更慢。调快能缓解上游出字慢导致的断音，但过快会失真。",
            )
            labeledIntField(
                label = "推理线程数",
                value = form.ttsNumThreads,
                onValueChange = {
                    form.ttsNumThreads = it
                    form.markDirty()
                },
                help = "合成占用的 CPU 线程数；留空按引擎默认。与 ASR 抢 CPU 时可适当调小。",
            )
        }

        // ===== 第二级：远程服务商 =====
        if (form.ttsMode == "remote") {
            labeledSelect(
                label = "服务商",
                selectedId = form.ttsBackend,
                options = form.ttsRemoteEngines,
                onSelect = {
                    form.ttsBackend = it
                    form.lastRemoteEngine = it
                    form.markDirty()
                },
            )
            HintLine("远程合成需网络与服务商账号；保存后新会话/测试台立即用新引擎（无需重启）。")
            if (form.ttsBackend == "xfyun") XfyunRemoteSection(form)
        }
    }
}

/**
 * 讯飞凭据与音色段（`engine=xfyun`）。未来其他供应商在此按 id 追加各自字段区。
 *
 * ⚠️ 音色列表依赖凭据：三要素未填齐时**隐藏选择区**（避免选出必然失败的音色）。
 */
@Composable
private fun XfyunRemoteSection(form: ConfigFormState) {
    val ctx = LocalAdminPagerOrThrow()
    fieldLabelRow(
        label = "讯飞凭据",
        help = "在讯飞开放平台「在线语音合成」控制台创建应用后获取。凭据只用于换取合成授权，" +
            "服务端打码存储、不回传明文。",
        trailing = {
            Button(
                text = "测试凭据",
                onClick = { form.testXfyunCreds(ctx) },
                type = ButtonType.OUTLINE,
            )
        },
    )
    if (form.credsTestMsg.isNotEmpty()) {
        Text(
            text = form.credsTestMsg,
            modifier = Modifier.padding(top = 4.dp),
            style = Theme.typography.caption,
            color = if (form.credsTestOk) Theme.colors.mutedForeground else Theme.colors.warning,
        )
    }
    labeledField(
        label = "应用 AppID",
        value = form.xfyunAppId,
        onValueChange = {
            form.xfyunAppId = it
            form.markDirty()
        },
    )
    labeledField(
        label = "API Key",
        value = form.xfyunApiKey,
        onValueChange = {
            form.xfyunApiKey = it
            form.markDirty()
        },
        placeholder = secretHint(form.xfyunApiKeyConfigured),
    )
    labeledField(
        label = "API Secret",
        value = form.xfyunApiSecret,
        onValueChange = {
            form.xfyunApiSecret = it
            form.markDirty()
        },
        placeholder = secretHint(form.xfyunApiSecretConfigured),
    )

    if (form.xfyunCredsReady()) {
        fieldLabelRow(
            label = "音色",
            help = "先保存三要素，再点「探测音色目录」——服务端会对候选音色逐个真实合成验证，" +
                "只把可用的列进下拉（约需 1 分钟）。改动会重新探测。",
            trailing = {
                Button(
                    text = "探测音色目录",
                    onClick = { form.refreshVoices(ctx) },
                    type = ButtonType.OUTLINE,
                )
            },
        )
        if (form.voicesRefreshMsg.isNotEmpty()) HintLine(form.voicesRefreshMsg)
        // 音色：分组（女声/男声/自定义）→ 类型（普通/极速/全部）→ 发音人；完整清单以控制台为准
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
                label = "发音人",
                selectedId = form.xfyunVoice,
                options = form.xfyunVoiceOptions,
                onSelect = {
                    form.xfyunVoice = it
                    form.markDirty()
                },
                help = "括号里是音色的适用场景标签（如「多情感」「童声」）。完整清单以讯飞控制台为准。",
            )
        } else {
            labeledField(
                label = "发音人标识（手填 vcn）",
                value = form.xfyunVoice,
                onValueChange = {
                    form.xfyunVoice = it
                    form.markDirty()
                },
                placeholder = "x4_yezi",
                help = "直接填讯飞控制台上的发音人标识（vcn）。服务端不会校验是否存在，填错会在合成时失败。",
            )
        }
    } else {
        HintLine("填写 AppID / API Key / API Secret 并保存后，可「探测音色目录」生成可用音色列表（真实合成验证）。")
    }
}

// ============================================================
// LLM
// ============================================================

/** `llm`：接口地址 / 密钥 / 模型名（可在线拉取）/ 系统提示词 / 采样与开关 + 高级（历史轮数）。 */
@Composable
fun LlmConfigCard(form: ConfigFormState) {
    val ctx = LocalAdminPagerOrThrow()
    LabeledCard(title = "LLM · 对话生成") {
        // 无 backend 选择：LLM 恒为 OpenAI 兼容 HTTP（Responses API），mock 已移除
        labeledField(
            label = "接口地址",
            value = form.llmApiBase,
            onValueChange = {
                form.llmApiBase = it
                form.markDirty()
            },
            help = "OpenAI 兼容端点，通常要带 /v1（如 https://api.openai.com/v1）。" +
                "必须是服务器能直连的地址；内网反代请填反代地址。",
        )
        // 密钥不回传明文（服务端打码）：已配置时提示"留空则不修改"
        labeledField(
            label = "API Key",
            value = form.llmApiKey,
            onValueChange = {
                form.llmApiKey = it
                form.markDirty()
            },
            placeholder = secretHint(form.llmApiKeyConfigured),
            help = "服务端打码存储、不回传明文；留空保存 = 不修改已配置的密钥。",
        )
        // ===== 模型在线列表：服务端代理 GET {base}/models；可下拉选，也可继续手输 =====
        labeledField(
            label = "模型名",
            value = form.llmModel,
            onValueChange = {
                form.llmModel = it
                form.markDirty()
            },
            help = "上游的模型标识（如 gpt-4o-mini、qwen-plus）。可点右侧「获取模型列表」从上游现拉，也可直接手输。",
            trailing = {
                Button(
                    text = if (form.llmModelsBusy) "获取中…" else "获取模型列表",
                    onClick = { form.fetchLlmModels(ctx) },
                    type = ButtonType.OUTLINE,
                    loading = form.llmModelsBusy,
                )
            },
        )
        if (form.llmModelsMsg.isNotEmpty()) {
            Text(
                text = form.llmModelsMsg,
                modifier = Modifier.padding(top = 4.dp),
                style = Theme.typography.caption,
                color = if (form.llmModelsMsg.startsWith("✓")) Theme.colors.mutedForeground else Theme.colors.warning,
            )
        }
        if (form.llmModelOptions.isNotEmpty()) {
            labeledSelect(
                label = "从在线列表选择",
                selectedId = form.llmModel,
                options = form.llmModelOptions,
                onSelect = { model ->
                    if (model != form.llmModel) {
                        form.llmModel = model
                        form.markDirty()
                    }
                },
                placeholder = "点击选择模型",
            )
        }
        // 系统提示词是散文输入：多行 Textarea（单行 Input 输不了换行）
        labeledTextArea(
            label = "系统提示词",
            value = form.llmSystemPrompt,
            onValueChange = {
                form.llmSystemPrompt = it
                form.markDirty()
            },
            help = "每次对话都放在最前面的角色设定（人设、口吻、约束）。" +
                "启用「人格档案」后由人格字段拼在上面，这里退化为补充说明。",
        )
        labeledDecimalField(
            label = "随机性（temperature）",
            value = form.llmTemperature,
            onValueChange = {
                form.llmTemperature = it
                form.markDirty()
            },
            help = "0~2：越低越稳定、越高越发散。语音对话建议 0.7 以下——太高的随机性会让回答跑题、也更费 token。",
        )
        switchRow(
            label = "启用 LLM 回复",
            checked = form.llmEnabled == "true",
            onToggle = { form.toggleStringBool(form::llmEnabled) },
            help = "技能未命中时，把识别文本交给大模型生成回复；关闭后直接念识别原文，" +
                "省一次调用（调试 ASR/TTS 链路时很有用）。",
        )
        switchRow(
            label = "流式输出（SSE）",
            checked = form.llmStream == "true",
            onToggle = { form.toggleStringBool(form::llmStream) },
            help = "边生成边下发：首字延迟更低，长回答听感更顺。上游不支持 SSE 时关闭，否则会等整段生成完才出声。",
        )
        AdvancedToggleRow(form)
        if (form.showAdvanced) {
            labeledIntField(
                label = "上下文历史轮数",
                value = form.llmMaxHistory,
                onValueChange = {
                    form.llmMaxHistory = it
                    form.markDirty()
                },
                help = "注入最近多少轮历史问答。调大更「记得住」，但每轮 token 更贵、也更容易超出模型上下文。",
            )
        }
    }
}
