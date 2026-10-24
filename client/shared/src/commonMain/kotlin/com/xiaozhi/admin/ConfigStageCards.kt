package com.xiaozhi.admin

import com.xiaozhi.admin.ui.*

import com.tencent.kuikly.core.base.ViewContainer
import com.tencent.kuikly.core.directives.vif
import com.tencent.kuikly.core.pager.Pager
import com.tencent.kuikly.core.views.Text

/**
 * 语音主流程四张配置卡（ASR / VAD / TTS / LLM）。
 *
 * 文案口径（2026-10，见 [fieldLabel] 与 [FieldHelpTip]）：
 * - 字段名只用**中文见名知意**的说法（CPU / AI / Token 这类通用术语保留英文），配置 key 不上界面；
 * - 需要额外解释的字段：说明写进 help（名称旁 ⓘ 弹层），行内只留名字，不再堆常驻长段落；
 * - 该功能的控制动作（测试凭据 / 探测音色目录 / 获取模型列表）放在名称右侧，与标题两端对齐。
 */
fun ViewContainer<*, *>.asrConfigCard(form: ConfigFormState) {
    groupedCard("ASR · 语音识别") {
        // 无 backend 选择：ASR 恒为 SenseVoice（sherpa），mock 已移除
        labeledField(
            "识别模型", { form.asrModel }, { form.asrModel = it; form.markDirty() },
            help = "sherpa-onnx 的 SenseVoice 模型文件路径（容器内绝对路径）。换模型只改这里，" +
                "保存后新会话生效。",
        )
        labeledField(
            "Tokens 词表", { form.asrTokens }, { form.asrTokens = it; form.markDirty() },
            help = "与模型配套的 tokens.txt 路径。必须与模型同批下载——混用会让识别结果变成乱码。",
        )
        labeledField(
            "识别语种", { form.asrLanguage }, { form.asrLanguage = it; form.markDirty() },
            "auto/zh/en/ja/ko/yue",
            help = "auto = 自动判别；明确指定语种可略提准确率。SenseVoice 支持中英日韩粤。",
        )
        labeledField(
            "识别引擎", { form.asrProvider }, { form.asrProvider = it; form.markDirty() },
            help = "ASR 提供方，当前固定 SenseVoice：本地 sherpa-onnx 推理，零外网、零按次计费。",
        )
        // ===== 高级（识别调优，默认折叠；P0-1）=====
        advancedToggleRow(form)
        vif({ form.showAdvanced }) {
            switchRow(
                "数字规范化（ITN）", form.asrUseItn == "true",
                help = "开启后「一千二百」输出成 1200、「三点半」输出成 3:30，" +
                    "方便大模型与记忆处理数字；关闭则保留原始汉字。",
            ) {
                form.toggleStringBool(form::asrUseItn)
            }
            labeledIntField(
                "推理线程数", { form.asrNumThreads }, { form.asrNumThreads = it; form.markDirty() },
                help = "SenseVoice 推理占用的 CPU 线程数；留空按引擎默认，机器核数少时调小可降低对其他阶段的抢占。",
            )
        }
    }
}

fun ViewContainer<*, *>.vadConfigCard(form: ConfigFormState, ctx: Pager) {
    groupedCard("VAD · 语音分段") {
        labeledField(
            "VAD 模型", { form.vadModel }, { form.vadModel = it; form.markDirty() },
            help = "sherpa-onnx 的 VAD（人声检测）模型路径，决定「什么时候算一句话说完了」。",
        )
        labeledDecimalField(
            "语音判定阈值", { form.vadThreshold }, { form.vadThreshold = it; form.markDirty() },
            help = "人声概率高于该值才判为「有人在说」，取值 0~1，常用 0.5。" +
                "调高会漏掉轻声细语，调低会把风扇、空调噪声当成说话。",
        )
        // ===== 高级（切句时序微调，默认折叠；P0-1）=====
        advancedToggleRow(form)
        vif({ form.showAdvanced }) {
            labeledDecimalField(
                "最短静音（秒）", { form.vadMinSilence }, { form.vadMinSilence = it; form.markDirty() },
                help = "连续静音超过该时长即断句：调小响应更快但容易把一句话切成两半，调大更稳但应答变慢。",
            )
            labeledDecimalField(
                "最短语音（秒）", { form.vadMinSpeech }, { form.vadMinSpeech = it; form.markDirty() },
                help = "短于该时长的声音片段直接丢弃，用于过滤咳嗽、碰撞等瞬时噪声。",
            )
        }
    }
}

fun ViewContainer<*, *>.ttsConfigCard(form: ConfigFormState, ctx: Pager) {
    groupedCard("TTS · 语音合成") {
        // ===== 第一级：合成方式（本地模型=离线 / 远程服务=在线）=====
        // 官方 AlertDialog 基座下拉；切方式即联动切换引擎/服务商组与下方字段。
        dropdownField(
            label = "合成方式",
            currentLabel = {
                if (form.ttsMode == "remote") "远程服务（在线 API）" else "本地模型（离线合成）"
            },
            options = { form.ttsModeOptions },
            selectedId = { form.ttsMode },
            isOpen = { form.isDropdownOpen("tts_mode") },
            onToggle = { form.toggleDropdown("tts_mode") },
            onSelect = { m ->
                if (m != form.ttsMode) {
                    form.ttsMode = m
                    // 切方式即切引擎组：本地→当前本地引擎；远程→上次使用的远程服务商
                    form.ttsBackend = if (m == "remote") form.lastRemoteEngine else "kokoro"
                    form.markDirty()
                    form.openDropdown = ""
                }
            },
            help = "本地模型 = 在服务器（容器）内离线合成，零外网、零按次计费，语气略平；" +
                "远程服务 = 调用在线合成 API，音色更自然但需网络与服务商账号。",
        )
        // ===== 缓存（本地/远程通用）：按句缓存已编码下行 Opus 帧，命中时零合成延迟 =====
        // 高级（缓存容量调优，默认折叠；P0-1）
        advancedToggleRow(form)
        vif({ form.showAdvanced }) {
            labeledIntField(
                "结果缓存条目数",
                { form.ttsCacheEntries },
                { form.ttsCacheEntries = it; form.markDirty() },
                "256",
                // 同段内档位不一致的例外：`[tts].engine` 是新会话生效，但缓存大小只能重启生效。
                // 这类"例外字段"必须用 fieldNote（段级提示会把整段说成需重启）。
                note = { form.schema.fieldNote("tts", "cache_entries") },
                help = "按句缓存已合成的下行音频（跨会话、跨设备共享），命中即刻出声、零合成延迟；" +
                    "0 = 关闭缓存。「唤醒问候语落盘复用」依赖它，关掉会一并失效。",
            )
        }
        // ===== 第二级：引擎 / 服务商（随方式联动）+ 各自字段 =====
        vif({ form.ttsMode == "local" }) {
            dropdownField(
                label = "本地引擎",
                currentLabel = {
                    form.ttsLocalEngines.firstOrNull { it.first == form.ttsBackend }?.second
                        ?: "本地 Kokoro INT8（离线）"
                },
                options = { form.ttsLocalEngines },
                selectedId = { form.ttsBackend },
                isOpen = { form.isDropdownOpen("tts_local_engine") },
                onToggle = { form.toggleDropdown("tts_local_engine") },
                onSelect = {
                    form.ttsBackend = it
                    form.markDirty()
                    form.openDropdown = ""
                },
            )
            // 本地模型参数（Kokoro）：写入 `[tts.kokoro]` 子段（P4 约定）
            labeledField(
                "模型文件", { form.ttsModel }, { form.ttsModel = it; form.markDirty() },
                help = "Kokoro ONNX 模型路径；INT8 量化版体积小、速度更快，音质略降。",
            )
            labeledField(
                "音色文件", { form.ttsVoices }, { form.ttsVoices = it; form.markDirty() },
                help = "音色向量表（voices 二进制）路径，必须与模型版本配套，否则合成会报错或串音。",
            )
            labeledField(
                "Tokens 词表", { form.ttsTokens }, { form.ttsTokens = it; form.markDirty() },
                help = "与模型配套的 tokens.txt 路径，负责把音素映射成模型输入 id。",
            )
            labeledField(
                "模型数据目录", { form.ttsDataDir }, { form.ttsDataDir = it; form.markDirty() },
                help = "模型自带的 data_dir（配置与字符表）所在目录。",
            )
            labeledField(
                "词典目录", { form.ttsDictDir }, { form.ttsDictDir = it; form.markDirty() },
                help = "中文分词与多音字词典目录（jieba）。目录缺失会导致多音字读错，不影响启动。",
            )
            labeledField(
                "自定义发音词典", { form.ttsLexicon }, { form.ttsLexicon = it; form.markDirty() },
                help = "lexicon 文件路径，每行「词 拼音」，用于修正人名的多音字读音；优先级高于内置词典。",
            )
            labeledField(
                "合成语种", { form.ttsLang }, { form.ttsLang = it; form.markDirty() },
                help = "发音规则所用语种代码（如 zh、en）；需与模型支持的语种一致。",
            )
            labeledIntField(
                "说话人编号", { form.ttsSpeaker }, { form.ttsSpeaker = it; form.markDirty() },
                help = "多说话人模型里的 speaker id（从 0 开始）；单说话人模型填 0。",
            )
            labeledDecimalField(
                "语速", { form.ttsSpeed }, { form.ttsSpeed = it; form.markDirty() },
                help = "大于 1 更快、小于 1 更慢。调快能缓解上游出字慢导致的断音，但过快会失真。",
            )
            labeledIntField(
                "推理线程数", { form.ttsNumThreads }, { form.ttsNumThreads = it; form.markDirty() },
                help = "合成占用的 CPU 线程数；留空按引擎默认。与 ASR 抢 CPU 时可适当调小。",
            )
        }
        vif({ form.ttsMode == "remote" }) {
            dropdownField(
                label = "服务商",
                currentLabel = {
                    form.ttsRemoteEngines.firstOrNull { it.first == form.ttsBackend }?.second
                        ?: "科大讯飞（在线）"
                },
                options = { form.ttsRemoteEngines },
                selectedId = { form.ttsBackend },
                isOpen = { form.isDropdownOpen("tts_remote_engine") },
                onToggle = { form.toggleDropdown("tts_remote_engine") },
                onSelect = {
                    form.ttsBackend = it
                    form.lastRemoteEngine = it
                    form.markDirty()
                    form.openDropdown = ""
                },
            )
            textNote("远程合成需网络与服务商账号；保存后新会话/测试台立即用新引擎（无需重启）。", micro = true)
            // 讯飞凭据与音色（engine=xfyun；未来其他供应商在此按 id 追加各自字段区）
            vif({ form.ttsBackend == "xfyun" }) {
                dividerH()
                // 凭据三要素：用**当前表单值**真实发起一次短合成验证（未保存也可测）
                fieldLabel(
                    "讯飞凭据",
                    help = "在讯飞开放平台「在线语音合成」控制台创建应用后获取。凭据只用于换取合成授权，" +
                        "服务端打码存储、不回传明文。",
                    trailing = { secondaryButton("测试凭据") { form.testXfyunCreds(ctx) } },
                )
                vif({ form.credsTestMsg.isNotEmpty() }) {
                    textNote(
                        form.credsTestMsg,
                        tone = if (form.credsTestOk) "tertiary" else "warn",
                        micro = true,
                    )
                }
                labeledField("应用 AppID", { form.xfyunAppId }, { form.xfyunAppId = it; form.markDirty() })
                labeledField(
                    "API Key", { form.xfyunApiKey }, { form.xfyunApiKey = it; form.markDirty() },
                    secretHint(form.xfyunApiKeyConfigured),
                )
                labeledField(
                    "API Secret", { form.xfyunApiSecret }, { form.xfyunApiSecret = it; form.markDirty() },
                    secretHint(form.xfyunApiSecretConfigured),
                )
                // ⚠️ 音色列表依赖凭据：三要素未填齐时隐藏选择区（避免选出必然失败的音色）
                vif({ form.xfyunCredsReady() }) {
                    fieldLabel(
                        "音色",
                        help = "先保存三要素，再点「探测音色目录」——服务端会对候选音色逐个真实合成验证，" +
                            "只把可用的列进下拉（约需 1 分钟）。改动会重新探测。",
                        trailing = { secondaryButton("探测音色目录") { form.refreshVoices(ctx) } },
                    )
                    vif({ form.voicesRefreshMsg.isNotEmpty() }) {
                        textNote(form.voicesRefreshMsg, micro = true)
                    }
                    // 音色：分组下拉（女声/男声/自定义）→ 发音人下拉；完整清单以控制台为准
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
                        isOpen = { form.isDropdownOpen("xfyun_voice_group") },
                        onToggle = { form.toggleDropdown("xfyun_voice_group") },
                        onSelect = {
                            form.selectXfyunVoiceGroup(it)
                            form.openDropdown = ""
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
                            isOpen = { form.isDropdownOpen("xfyun_voice_type") },
                            onToggle = { form.toggleDropdown("xfyun_voice_type") },
                            onSelect = {
                                form.selectXfyunVoiceType(it)
                                form.openDropdown = ""
                            },
                        )
                        dropdownField(
                            label = "发音人",
                            currentLabel = {
                                form.xfyunVoiceOptions.firstOrNull { it.first == form.xfyunVoice }?.second
                                    ?: form.xfyunVoice
                            },
                            options = { form.xfyunVoiceOptions },
                            selectedId = { form.xfyunVoice },
                            isOpen = { form.isDropdownOpen("xfyun_voice") },
                            onToggle = { form.toggleDropdown("xfyun_voice") },
                            onSelect = {
                                form.xfyunVoice = it
                                form.markDirty()
                                form.openDropdown = ""
                            },
                            help = "括号里是音色的适用场景标签（如「多情感」「童声」）。完整清单以讯飞控制台为准。",
                        )
                    }
                    vif({ form.xfyunVoiceGroup == "custom" }) {
                        labeledField(
                            "发音人标识（手填 vcn）", { form.xfyunVoice }, { form.xfyunVoice = it; form.markDirty() },
                            "x4_yezi",
                            help = "直接填讯飞控制台上的发音人标识（vcn）。服务端不会校验是否存在，填错会在合成时失败。",
                        )
                    }
                } // vif(xfyunCredsReady)：音色列表依赖凭据，未填齐时隐藏
                vif({ !form.xfyunCredsReady() }) {
                    Text {
                        attr {
                            fontSize(AdminType.caption)
                            color(AdminColors.textTertiary)
                            text("填写 AppID / API Key / API Secret 并保存后，可「探测音色目录」生成可用音色列表（真实合成验证）。")
                        }
                    }
                }
            }
        }
    }
}


fun ViewContainer<*, *>.llmConfigCard(form: ConfigFormState, ctx: Pager) {
    groupedCard("LLM · 对话生成") {
        // 无 backend 选择：LLM 恒为 OpenAI 兼容 HTTP（Responses API），mock 已移除
        labeledField(
            "接口地址", { form.llmApiBase }, { form.llmApiBase = it; form.markDirty() },
            help = "OpenAI 兼容端点，通常要带 /v1（如 https://api.openai.com/v1）。" +
                "必须是服务器能直连的地址；内网反代请填反代地址。",
        )
        // 密钥不回传明文（服务端打码）：已配置时提示"留空则不修改"
        labeledField(
            "API Key", { form.llmApiKey }, { form.llmApiKey = it; form.markDirty() },
            secretHint(form.llmApiKeyConfigured),
            help = "服务端打码存储、不回传明文；留空保存 = 不修改已配置的密钥。",
        )
        // ===== 模型在线列表：服务端代理 GET {base}/models；可下拉选，也可继续手输 =====
        labeledField(
            "模型名", { form.llmModel }, { form.llmModel = it; form.markDirty() },
            help = "上游的模型标识（如 gpt-4o-mini、qwen-plus）。可点右侧「获取模型列表」从上游现拉，也可直接手输。",
            trailing = {
                secondaryButton("获取模型列表", loading = { form.llmModelsBusy }) { form.fetchLlmModels(ctx) }
            },
        )
        vif({ form.llmModelsMsg.isNotEmpty() }) {
            textNote(form.llmModelsMsg, tone = if (form.llmModelsMsg.startsWith("✓")) "tertiary" else "warn", micro = true)
        }
        vif({ form.llmModelOptions.isNotEmpty() }) {
            dropdownField(
                label = "从在线列表选择",
                currentLabel = { form.llmModel.ifEmpty { "点击选择模型" } },
                options = { form.llmModelOptions },
                selectedId = { form.llmModel },
                isOpen = { form.isDropdownOpen("llm_model") },
                onToggle = { form.toggleDropdown("llm_model") },
                onSelect = { m ->
                    if (m != form.llmModel) {
                        form.llmModel = m
                        form.markDirty()
                    }
                },
            )
        }
        // 系统提示词是散文输入：官方 TextArea 多行（此前用单行 Input 输不了换行）
        labeledTextArea(
            "系统提示词", { form.llmSystemPrompt }, { form.llmSystemPrompt = it; form.markDirty() },
            height = 100f,
            help = "每次对话都放在最前面的角色设定（人设、口吻、约束）。" +
                "启用「人格档案」后由人格字段拼在上面，这里退化为补充说明。",
        )
        labeledDecimalField(
            "随机性（temperature）", { form.llmTemperature }, { form.llmTemperature = it; form.markDirty() },
            help = "0~2：越低越稳定、越高越发散。语音对话建议 0.7 以下——太高的随机性会让回答跑题、也更费 token。",
        )
        switchRow(
            "启用 LLM 回复", form.llmEnabled == "true",
            help = "技能未命中时，把识别文本交给大模型生成回复；关闭后直接念识别原文，" +
                "省一次调用（调试 ASR/TTS 链路时很有用）。",
        ) {
            form.toggleStringBool(form::llmEnabled)
        }
        switchRow(
            "流式输出（SSE）", form.llmStream == "true",
            help = "边生成边下发：首字延迟更低，长回答听感更顺。上游不支持 SSE 时关闭，否则会等整段生成完才出声。",
        ) {
            form.toggleStringBool(form::llmStream)
        }
        // ===== 高级（上下文长度调优，默认折叠；P0-1）=====
        advancedToggleRow(form)
        vif({ form.showAdvanced }) {
            labeledIntField(
                "上下文历史轮数", { form.llmMaxHistory }, { form.llmMaxHistory = it; form.markDirty() },
                help = "注入最近多少轮历史问答。调大更「记得住」，但每轮 token 更贵、也更容易超出模型上下文。",
            )
        }
    }
}
