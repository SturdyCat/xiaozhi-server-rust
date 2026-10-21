package com.xiaozhi.admin

import com.tencent.kuikly.core.base.ViewContainer
import com.tencent.kuikly.core.directives.vfor
import com.tencent.kuikly.core.directives.vif
import com.tencent.kuikly.core.pager.Pager
import com.tencent.kuikly.core.base.Border
import com.tencent.kuikly.core.base.BorderStyle
import com.tencent.kuikly.core.views.Input
import com.tencent.kuikly.core.views.TextInputState
import com.tencent.kuikly.core.views.Text
import com.tencent.kuikly.core.views.View

fun ViewContainer<*, *>.asrConfigCard(form: ConfigFormState) {
    groupedCard("ASR") {
        // 无 backend 选择：ASR 恒为 SenseVoice（sherpa），mock 已移除
        labeledField("model", { form.asrModel }, { form.asrModel = it; form.markDirty() })
        labeledField("tokens", { form.asrTokens }, { form.asrTokens = it; form.markDirty() })
        labeledField("language", { form.asrLanguage }, { form.asrLanguage = it; form.markDirty() }, "auto/zh/en/ja/ko/yue")
        labeledField("provider", { form.asrProvider }, { form.asrProvider = it; form.markDirty() })
        // ===== 高级（识别调优，默认折叠；P0-1）=====
        advancedToggleRow(form)
        vif({ form.showAdvanced }) {
            switchRow("use_itn", form.asrUseItn == "true") {
                form.toggleStringBool(form::asrUseItn)
            }
            labeledIntField("num_threads", { form.asrNumThreads }, { form.asrNumThreads = it; form.markDirty() })
        }
    }
}

fun ViewContainer<*, *>.vadConfigCard(form: ConfigFormState, ctx: Pager) {
    groupedCard("VAD") {
        labeledField("model", { form.vadModel }, { form.vadModel = it; form.markDirty() })
        labeledDecimalField("threshold", { form.vadThreshold }, { form.vadThreshold = it; form.markDirty() })
        // ===== 高级（切句时序微调，默认折叠；P0-1）=====
        advancedToggleRow(form)
        vif({ form.showAdvanced }) {
            labeledDecimalField("min_silence_duration", { form.vadMinSilence }, { form.vadMinSilence = it; form.markDirty() })
            labeledDecimalField("min_speech_duration", { form.vadMinSpeech }, { form.vadMinSpeech = it; form.markDirty() })
        }
    }
}

fun ViewContainer<*, *>.ttsConfigCard(form: ConfigFormState, ctx: Pager) {
    groupedCard("TTS") {
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
        )
        // ===== 缓存（本地/远程通用）：按句缓存已编码下行 Opus 帧，命中时零合成延迟 =====
        // 高级（缓存容量调优，默认折叠；P0-1）
        advancedToggleRow(form)
        vif({ form.showAdvanced }) {
            labeledIntField(
                "cache_entries",
                { form.ttsCacheEntries },
                { form.ttsCacheEntries = it; form.markDirty() },
                "256",
                // 同段内档位不一致的例外：`[tts].engine` 是新会话生效，但缓存大小只能重启生效。
                // 这类"例外字段"必须用 fieldNote（段级提示会把整段说成需重启）。
                note = { form.schema.fieldNote("tts", "cache_entries") },
            )
            Text {
                attr {
                    fontSize(AdminType.caption)
                    color(AdminColors.textTertiary)
                    text("缓存已合成句子的下行音频（跨会话共享），命中即免合成延迟；0 = 关闭。")
                }
            }
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
            labeledField("model", { form.ttsModel }, { form.ttsModel = it; form.markDirty() })
            labeledField("voices", { form.ttsVoices }, { form.ttsVoices = it; form.markDirty() })
            labeledField("tokens", { form.ttsTokens }, { form.ttsTokens = it; form.markDirty() })
            labeledField("data_dir", { form.ttsDataDir }, { form.ttsDataDir = it; form.markDirty() })
            labeledField("dict_dir", { form.ttsDictDir }, { form.ttsDictDir = it; form.markDirty() })
            labeledField("lexicon", { form.ttsLexicon }, { form.ttsLexicon = it; form.markDirty() })
            labeledField("lang", { form.ttsLang }, { form.ttsLang = it; form.markDirty() })
            labeledIntField("speaker", { form.ttsSpeaker }, { form.ttsSpeaker = it; form.markDirty() })
            labeledDecimalField("speed", { form.ttsSpeed }, { form.ttsSpeed = it; form.markDirty() })
            labeledIntField("num_threads", { form.ttsNumThreads }, { form.ttsNumThreads = it; form.markDirty() })
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
            Text {
                attr {
                    fontSize(AdminType.caption)
                    color(AdminColors.textSecondary)
                    text("远程合成需网络与对应服务商账号；保存后新会话/测试台立即用新引擎（无需重启）。")
                }
            }
            // 讯飞凭据与音色（engine=xfyun；未来其他供应商在此按 id 追加各自字段区）
            vif({ form.ttsBackend == "xfyun" }) {
                dividerH()
                Text {
                    attr {
                        fontSize(AdminType.caption)
                        color(AdminColors.textSecondary)
                        text("科大讯飞在线合成（在讯飞开放平台「在线语音合成」控制台获取密钥）")
                    }
                }
                labeledField("app_id", { form.xfyunAppId }, { form.xfyunAppId = it; form.markDirty() })
                labeledField("api_key", { form.xfyunApiKey }, { form.xfyunApiKey = it; form.markDirty() }, secretHint(form.xfyunApiKeyConfigured))
                labeledField("api_secret", { form.xfyunApiSecret }, { form.xfyunApiSecret = it; form.markDirty() }, secretHint(form.xfyunApiSecretConfigured))
                // 测试凭据：用**当前表单值**（未保存也可测）真实发起一次短合成，验证密钥/服务/音色
                actionRow {
                    secondaryButton("测试凭据") { form.testXfyunCreds(ctx) }
                    vif({ form.credsTestMsg.isNotEmpty() }) {
                        View { attr { width(AdminSpace.sm) } }
                        Text {
                            attr {
                                fontSize(AdminType.micro)
                                color(if (form.credsTestOk) AdminColors.textTertiary else AdminColors.dangerTintText)
                                text(form.credsTestMsg)
                            }
                        }
                    }
                }
                // ⚠️ 音色列表依赖凭据：三要素未填齐时隐藏选择区（避免选出必然失败的音色）
                vif({ form.xfyunCredsReady() }) {
                actionRow {
                    secondaryButton("探测音色目录") { form.refreshVoices(ctx) }
                    vif({ form.voicesRefreshMsg.isNotEmpty() }) {
                        View { attr { width(AdminSpace.sm) } }
                        Text {
                            attr {
                                fontSize(AdminType.micro)
                                color(AdminColors.textTertiary)
                                text(form.voicesRefreshMsg)
                            }
                        }
                    }
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
                        label = "发音人（vcn）",
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
                    )
                }
                vif({ form.xfyunVoiceGroup == "custom" }) {
                    labeledField("voice（手填 vcn）", { form.xfyunVoice }, { form.xfyunVoice = it; form.markDirty() })
                }
                } // vif(xfyunCredsReady)：音色列表依赖凭据，未填齐时隐藏
                vif({ !form.xfyunCredsReady() }) {
                    Text {
                        attr {
                            fontSize(AdminType.caption)
                            color(AdminColors.textTertiary)
                            text("填写 app_id / api_key / api_secret 并保存后，点「探测音色目录」生成可用音色列表（真实合成验证）。")
                        }
                    }
                }
            }
        }
    }
}


fun ViewContainer<*, *>.llmConfigCard(form: ConfigFormState, ctx: Pager) {
    groupedCard("LLM") {
        // 无 backend 选择：LLM 恒为 OpenAI 兼容 HTTP（Responses API），mock 已移除
        labeledField("api_base", { form.llmApiBase }, { form.llmApiBase = it; form.markDirty() })
        // 密钥不回传明文（服务端打码）：已配置时提示"留空则不修改"
        labeledField("api_key", { form.llmApiKey }, { form.llmApiKey = it; form.markDirty() }, secretHint(form.llmApiKeyConfigured))
        labeledField("model", { form.llmModel }, { form.llmModel = it; form.markDirty() })
        // ===== 模型在线列表：服务端代理 GET {base}/models；可下拉选，也可继续手输 =====
        actionRow {
            secondaryButton("获取模型列表", loading = { form.llmModelsBusy }) { form.fetchLlmModels(ctx) }
        }
        vif({ form.llmModelsMsg.isNotEmpty() }) {
            textNote(form.llmModelsMsg, tone = if (form.llmModelsMsg.startsWith("✓")) "tertiary" else "warn", micro = true)
        }
        vif({ form.llmModelOptions.isNotEmpty() }) {
            dropdownField(
                label = "模型（在线列表）",
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
        labeledTextArea("system_prompt", { form.llmSystemPrompt }, { form.llmSystemPrompt = it; form.markDirty() }, height = 100f)
        labeledDecimalField("temperature", { form.llmTemperature }, { form.llmTemperature = it; form.markDirty() })
        switchRow("enabled（LLM 回声开关；关闭后技能未命中原文回声，不走 LLM）", form.llmEnabled == "true") {
            form.toggleStringBool(form::llmEnabled)
        }
        switchRow("stream（SSE 流式）", form.llmStream == "true") {
            form.toggleStringBool(form::llmStream)
        }
        // ===== 高级（上下文长度调优，默认折叠；P0-1）=====
        advancedToggleRow(form)
        vif({ form.showAdvanced }) {
            labeledIntField("max_history", { form.llmMaxHistory }, { form.llmMaxHistory = it; form.markDirty() })
        }
    }
}
