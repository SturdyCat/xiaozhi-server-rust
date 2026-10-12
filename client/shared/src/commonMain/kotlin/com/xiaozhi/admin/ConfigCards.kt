package com.xiaozhi.admin

import com.tencent.kuikly.core.base.PagerScope
import com.tencent.kuikly.core.base.ViewContainer
import com.tencent.kuikly.core.directives.vfor
import com.tencent.kuikly.core.directives.vif
import com.tencent.kuikly.core.module.NetworkModule
import com.tencent.kuikly.core.nvi.serialization.json.JSONObject
import com.tencent.kuikly.core.pager.Pager
import com.tencent.kuikly.core.reactive.collection.ObservableList
import com.tencent.kuikly.core.reactive.handler.observable
import com.tencent.kuikly.core.reactive.handler.observableList
import com.tencent.kuikly.core.timer.setTimeout
import com.tencent.kuikly.core.base.Border
import com.tencent.kuikly.core.base.BorderStyle
import com.tencent.kuikly.core.views.Input
import com.tencent.kuikly.core.views.Scroller
import com.tencent.kuikly.core.views.Text
import com.tencent.kuikly.core.views.View

/**
 * section id → 对应能力 capability id（`GET /api/plugins` 的 `capabilities[].id`，见
 * `server/src/plugins/registry/mod.rs` 的 `Capability::id`）。除 AIUI 段在注册表里叫
 * `fullchain` 外，多数 section 与其能力 id 同名。server / audio / ota / mcp / overview
 * 无单一能力对应（不在表中 → 查询返回 null，不显示就绪点）。
 */
private val SECTION_TO_CAPABILITY: Map<String, String> = mapOf(
    "asr" to "asr",
    "vad" to "vad",
    "tts" to "tts",
    "llm" to "llm",
    "soul" to "soul",
    "memory" to "memory",
    "command" to "command",
    "wakeup" to "wakeup",
    "aiui" to "fullchain",
)

/**
 * 取某 section 对应的能力行（来自 [PluginMetaState]，即 `GET /api/plugins` 的权威就绪态）。
 * 无对应能力返回 null。
 *
 * ⚠️ 在闭包内读 `meta` 的 observable 列表 → meta 加载/刷新后自动重算（Kuikly 响应式铁律：
 * 闭包内读取才被跟踪）。[CapabilityRow] 本身是不可变快照，meta 整表替换（clear + add）时
 * 列表被观察、触发重渲染。
 */
private fun sectionCapability(meta: PluginMetaState, sectionId: String): CapabilityRow? {
    val capId = SECTION_TO_CAPABILITY[sectionId] ?: return null
    for (r in meta.requiredRows) if (r.id == capId) return r
    for (r in meta.optionalRows) if (r.id == capId) return r
    return null
}

/**
 * section 头部「就绪度」横幅：仅当对应能力 phase 偏离（`failed` / `degraded`）时显示
 * 状态徽标 + 原因（复用总览页同款 [statusBadge]，不另造样式）。
 * 正常（`active` / 未加载 / 无对应能力）不打扰。
 */
private fun ViewContainer<*, *>.sectionReadinessBanner(meta: PluginMetaState, sectionId: String) {
    vif({ sectionCapability(meta, sectionId)?.phase in setOf("failed", "degraded") }) {
        val row = sectionCapability(meta, sectionId)!!
        val (badgeState, badgeText) = when (row.phase) {
            "failed" -> "error" to "异常"
            "degraded" -> "busy" to "降级"
            else -> "idle" to row.phase
        }
        View {
            attr {
                flexDirectionRow()
                alignItemsCenter()
                marginBottom(AdminSpace.sm)
            }
            statusBadge(state = { badgeState }, text = { badgeText })
            vif({ row.reason.isNotEmpty() }) {
                View { attr { width(AdminSpace.sm) } }
                Text {
                    attr {
                        flex(1f)
                        fontSize(AdminType.micro)
                        color(if (row.phase == "failed") AdminColors.dangerTintText else AdminColors.warningTintText)
                        text(row.reason)
                    }
                }
            }
        }
    }
}

/**
 * 配置页主体（P5 → 重组为「左侧分组导航 + 右侧单 section」两栏布局，替代原先 14 个平铺 tab）。
 *
 * ## 为什么从 tab 改成两栏导航
 *
 * 14 个平铺 tab 既挤占横向空间、又无法体现"这些配置属于哪条链路"的层次，用户反映"无从下手"。
 * 改为 macOS 系统设置式的**左侧分组导航**（服务与接入 / 语音与对话 / 智能体）+ 顶部**搜索框**，
 * 右侧只渲染当前选中的单个 section——既压缩了导航密度，又让"Server 属于服务接入、TTS 属于语音链路"
 * 一目了然。所有 `*ConfigCard` / `overviewCard` / `mcpToolsCard` / `otaCard` 内容函数原样复用，
 * 只是从"全部塞进 PageList"变为"按选中按需渲染"。
 *
 * 数据源语义不变：`meta` 总览只读、`mcp` 工具只读、`ota` 固件（不写 `/api/config`）、`toast` 弹结果。
 *
 * ⚠️ Kuikly DSL 约束（与渲染扩展同源）：`ViewBuilder` 接收者由调用点推断，渲染扩展必须在
 * 目标容器闭包内**非限定**调用，否则节点会逃逸到根容器导致布局错乱。
 */
fun ViewContainer<*, *>.renderConfigLayout(
    form: ConfigFormState,
    meta: PluginMetaState,
    mcp: McpToolsState,
    ota: OtaState,
    toast: ToastState,
    ctx: Pager,
) {
    View {
        attr {
            flex(1f)
            flexDirectionRow()
        }

        // ===== 左侧：搜索 + 分组导航 =====
        View {
            attr {
                width(220f)
                flexDirectionColumn()
                backgroundColor(AdminColors.sidebarBg)
                paddingTop(AdminSpace.md)
            }
            // 搜索框（绑定 form.configSearch：空 = 分组展示 / 非空 = 平铺命中项）
            configSearchField(form)
            Scroller {
                attr {
                    flex(1f)
                    paddingLeft(AdminSpace.sm)
                    paddingRight(AdminSpace.sm)
                    paddingBottom(AdminSpace.lg)
                }
                // 搜索态：平铺命中项（无分组标题）
                vif({ form.configSearch.isNotBlank() }) {
                    val hits = ConfigSections.search(form.configSearch)
                    if (hits.isEmpty()) {
                        textNote("没有匹配「${form.configSearch}」的配置项", micro = true)
                    } else {
                        for (d in hits) {
                            sidebarItem(
                                d.label,
                                isSelected = { form.configSection == d.id },
                                showDot = { sectionCapability(meta, d.id)?.phase in setOf("failed", "degraded") },
                                onClick = { form.configSection = d.id },
                            )
                        }
                    }
                }
                // 分组态：置顶项 + 三个分组
                vif({ form.configSearch.isBlank() }) {
                    for (d in ConfigSections.pinned) {
                        sidebarItem(
                            d.label,
                            isSelected = { form.configSection == d.id },
                            showDot = { sectionCapability(meta, d.id)?.phase in setOf("failed", "degraded") },
                            onClick = { form.configSection = d.id },
                        )
                    }
                    for ((gid, glabel) in ConfigSections.groups) {
                        configGroupHeader(glabel)
                        for (d in ConfigSections.sectionsIn(gid)) {
                            sidebarItem(
                                d.label,
                                isSelected = { form.configSection == d.id },
                                showDot = { sectionCapability(meta, d.id)?.phase in setOf("failed", "degraded") },
                                onClick = { form.configSection = d.id },
                            )
                        }
                    }
                }
            }
        }

        // 左栏右侧 1px 分隔
        View {
            attr {
                width(1f)
                backgroundColor(AdminColors.divider)
            }
        }

        // ===== 右侧：当前选中的单个 section =====
        View {
            attr {
                flex(1f)
                flexDirectionColumn()
                backgroundColor(AdminColors.windowBg)
            }
            opsScroller {
                // ⚠️ 用 vfor 包一层：configSection 是多选一（boolean 不会翻转），普通闭包不会随它重渲染；
                // vfor 监测到列表元素变化会重建该唯一项，从而切到对应 section 内容（否则点击导航不刷新右侧）。
                vfor({ form.configSectionList }) { sectionId ->
                    renderConfigSection(form, meta, mcp, ota, toast, ctx, sectionId)
                }
            }
        }
    }
}

/**
 * 渲染单个配置 section（按 id 分发到既有卡片函数）。
 * 把 renderForm 里"全部 14 页塞进 PageList"的 `when` 收敛为"按需渲染一个"，
 * 卡片函数本身零改动、零重复。
 */
fun ViewContainer<*, *>.renderConfigSection(
    form: ConfigFormState,
    meta: PluginMetaState,
    mcp: McpToolsState,
    ota: OtaState,
    toast: ToastState,
    ctx: Pager,
    id: String,
) {
    // 改动影响高亮：上次保存的 hot 投影到当前 section 头部（仅确有非即时影响时显示）
    vif({ form.saveImpactNote.isNotEmpty() }) {
        textNote(form.saveImpactNote, tone = "warn", micro = true)
        View { attr { height(AdminSpace.sm) } }
    }
    // 就绪度徽标：对应能力 phase 偏离（degraded/failed）时显示状态 + 原因（位于 section 卡片上方）
    sectionReadinessBanner(meta, id)
    when (id) {
        "overview" -> overviewCard(meta, ctx)
        "mcp" -> mcpToolsCard(mcp, ctx)
        "ota" -> otaCard(ota, toast, ctx)
        "server" -> serverConfigCard(form)
        "audio" -> audioConfigCard(form)
        "asr" -> asrConfigCard(form)
        "vad" -> vadConfigCard(form)
        "tts" -> ttsConfigCard(form, ctx)
        "llm" -> llmConfigCard(form)
        "command" -> commandConfigCard(form)
        "wakeup" -> wakeupConfigCard(form)
        "aiui" -> aiuiConfigCard(form)
        "soul" -> soulConfigCard(form, ctx)
        "memory" -> memoryConfigCard(form, ctx)
        // 未注册内容不要静默留白（"新加了 section 却忘了写内容"会表现成点进去什么都没有）
        else -> groupedCard("未知配置页") {
            textNote("管理页没有为 section「$id」注册内容（请检查 ConfigSections.all）。")
        }
    }
}

/** 配置左侧导航的搜索框（绑定 form.configSearch，输入即筛选）。 */
fun ViewContainer<*, *>.configSearchField(form: ConfigFormState) {
    View {
        attr {
            height(40f)
            flexDirectionRow()
            alignItemsCenter()
            backgroundColor(AdminColors.fieldBg)
            border(Border(1f, BorderStyle.SOLID, AdminColors.divider))
            borderRadius(AdminShape.radiusSm)
            marginLeft(AdminSpace.sm)
            marginRight(AdminSpace.sm)
            marginBottom(AdminSpace.sm)
            paddingLeft(AdminSpace.sm)
            paddingRight(AdminSpace.sm)
        }
        Input {
            attr {
                flex(1f)
                height(40f)
                backgroundColor(AdminColors.transparent)
                fontSize(AdminType.body)
                color(AdminColors.textPrimary)
                text(form.configSearch)
                placeholder("搜索配置项…")
            }
            event { textDidChange { params -> form.configSearch = params.text } }
        }
    }
}

/** 配置左侧导航的分组标题（如「语音与对话」）。 */
fun ViewContainer<*, *>.configGroupHeader(label: String) {
    Text {
        attr {
            fontSize(AdminType.micro)
            fontWeightMedium()
            color(AdminColors.textTertiary)
            text(label)
            marginLeft(AdminSpace.sm)
            marginRight(AdminSpace.sm)
            marginTop(AdminSpace.md)
            marginBottom(AdminSpace.xs)
        }
    }
}

// ============================================================
// 各配置分组卡（renderForm 与测试台 tab 右栏共用；字段变更统一 markDirty）
// ============================================================

fun ViewContainer<*, *>.serverConfigCard(form: ConfigFormState) {
    groupedCard("Server") {
        // 配置来源（路径 + 持久化）：一眼定位"改配置不生效/重启即丢"类问题
        vif({ form.configMetaMsg.isNotEmpty() }) {
            Text {
                attr {
                    fontSize(AdminType.micro)
                    color(AdminColors.textTertiary)
                    marginBottom(AdminSpace.xs)
                    text(form.configMetaMsg)
                }
            }
        }
        // 整段档位提示（来自 /api/config/schema 的 app_sections，服务端为唯一权威）
        vif({ form.schema.sectionNote("server").isNotEmpty() }) {
            textNote(form.schema.sectionNote("server"), micro = true)
        }
        labeledField("port", { form.port }, { form.port = it; form.dirty = true }, "8000")
        labeledField("expected_token", { form.expectedToken }, { form.expectedToken = it; form.dirty = true })
        labeledField("worker_threads", { form.workerThreads }, { form.workerThreads = it; form.dirty = true })
        labeledField("admin_dir", { form.adminDir }, { form.adminDir = it; form.dirty = true })
    }
}

fun ViewContainer<*, *>.audioConfigCard(form: ConfigFormState) {
    groupedCard("Audio") {
        // 整段档位提示：下行参数与 hello/OTA 协商强绑定，全部只能重启生效
        vif({ form.schema.sectionNote("audio").isNotEmpty() }) {
            textNote(form.schema.sectionNote("audio"), micro = true)
        }
        labeledField("downlink_sample_rate", { form.downlinkSampleRate }, { form.downlinkSampleRate = it; form.dirty = true })
        labeledField("downlink_frame_duration_ms", { form.downlinkFrameMs }, { form.downlinkFrameMs = it; form.dirty = true })
        labeledField("channels", { form.channels }, { form.channels = it; form.dirty = true })
        labeledField("binary_protocol_version", { form.binaryProtocolVersion }, { form.binaryProtocolVersion = it; form.dirty = true })
        // 抖动缓冲提前量：下发时间表比实时提前该时长，使设备解码队列常备音频存货，
        // 吸收网络抖动 + 上游 LLM 出句/TTS 合成的停顿（长回复断音的主要来源）。
        labeledField("downlink_lead_ms", { form.downlinkLeadMs }, { form.downlinkLeadMs = it; form.dirty = true }, "1000")
        labeledField("downlink_queue_packets", { form.downlinkQueuePackets }, { form.downlinkQueuePackets = it; form.dirty = true }, "20")
        Text {
            attr {
                fontSize(AdminType.caption)
                color(AdminColors.textTertiary)
                text("缓冲越深，长回复越抗上游变慢（默认 1000，接近设备端极限 1080）。想要更深必须先放大固件解码队列 MAX_DECODE_PACKETS_IN_QUEUE，并同步 [audio].downlink_queue_packets。代价：打断后设备会把已缓存的 ~提前量 音频播完才停口。")
            }
        }
        // 服务器侧缓冲：设备队列只有 ~1.2s，上游慢于实时（LLM 逐句吐字 / 在线合成首片）
        // 时唯一还能加深缓冲的地方——用服务器内存换下行连续性。
        labeledField("downlink_buffer_ms", { form.downlinkBufferMs }, { form.downlinkBufferMs = it; form.dirty = true }, "30000")
        labeledField("downlink_buffer_start_ms", { form.downlinkBufferStartMs }, { form.downlinkBufferStartMs = it; form.dirty = true }, "1500")
        labeledField("downlink_buffer_start_max_ms", { form.downlinkBufferStartMaxMs }, { form.downlinkBufferStartMaxMs = it; form.dirty = true }, "4000")
        Text {
            attr {
                fontSize(AdminType.caption)
                color(AdminColors.textTertiary)
                text("服务器缓冲三段：TTS 流下载 → PCM 下载缓冲（按秒限速，由水库容量派生、上限 10s）→ 已编码 Opus 水库（默认 30s ≈ 60KB/会话）→ 按实时节拍下发。上游停顿 ≤ 水库水位 + 设备队列 时听感全程连续。起跑闸先攒后播（默认 1.5s，短回复立即放行），最长等待 4s 兜底。⚠️ 上游**持续**慢于实时（每轮日志“本轮吞吐 ≈ N× 实时”，N<1）时缺口会累积，缓冲只能推迟断音——根治要提速上游（提高 [tts].speed / 换更快 LLM）。全部重启生效。")
            }
        }
    }
}

fun ViewContainer<*, *>.asrConfigCard(form: ConfigFormState) {
    groupedCard("ASR") {
        // 无 backend 选择：ASR 恒为 SenseVoice（sherpa），mock 已移除
        labeledField("model", { form.asrModel }, { form.asrModel = it; form.dirty = true })
        labeledField("tokens", { form.asrTokens }, { form.asrTokens = it; form.dirty = true })
        labeledField("language", { form.asrLanguage }, { form.asrLanguage = it; form.dirty = true }, "auto/zh/en/ja/ko/yue")
        switchRow("use_itn", form.asrUseItn == "true") {
            form.asrUseItn = if (form.asrUseItn == "true") "false" else "true"; form.dirty = true
        }
        labeledField("num_threads", { form.asrNumThreads }, { form.asrNumThreads = it; form.dirty = true })
        labeledField("provider", { form.asrProvider }, { form.asrProvider = it; form.dirty = true })
    }
}

fun ViewContainer<*, *>.vadConfigCard(form: ConfigFormState) {
    groupedCard("VAD") {
        labeledField("model", { form.vadModel }, { form.vadModel = it; form.dirty = true })
        labeledField("threshold", { form.vadThreshold }, { form.vadThreshold = it; form.dirty = true })
        labeledField("min_silence_duration", { form.vadMinSilence }, { form.vadMinSilence = it; form.dirty = true })
        labeledField("min_speech_duration", { form.vadMinSpeech }, { form.vadMinSpeech = it; form.dirty = true })
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
            isOpen = { form.openDropdown == "tts_mode" },
            onToggle = { form.openDropdown = if (form.openDropdown == "tts_mode") "" else "tts_mode" },
            onSelect = { m ->
                if (m != form.ttsMode) {
                    form.ttsMode = m
                    // 切方式即切引擎组：本地→当前本地引擎；远程→上次使用的远程服务商
                    form.ttsBackend = if (m == "remote") form.lastRemoteEngine else "kokoro"
                    form.dirty = true
                    form.openDropdown = ""
                }
            },
        )
        // ===== 缓存（本地/远程通用）：按句缓存已编码下行 Opus 帧，命中时零合成延迟 =====
        labeledField(
            "cache_entries",
            { form.ttsCacheEntries },
            { form.ttsCacheEntries = it; form.dirty = true },
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
                isOpen = { form.openDropdown == "tts_local_engine" },
                onToggle = { form.openDropdown = if (form.openDropdown == "tts_local_engine") "" else "tts_local_engine" },
                onSelect = {
                    form.ttsBackend = it
                    form.dirty = true
                    form.openDropdown = ""
                },
            )
            // 本地模型参数（Kokoro）：写入 `[tts.kokoro]` 子段（P4 约定）
            labeledField("model", { form.ttsModel }, { form.ttsModel = it; form.dirty = true })
            labeledField("voices", { form.ttsVoices }, { form.ttsVoices = it; form.dirty = true })
            labeledField("tokens", { form.ttsTokens }, { form.ttsTokens = it; form.dirty = true })
            labeledField("data_dir", { form.ttsDataDir }, { form.ttsDataDir = it; form.dirty = true })
            labeledField("dict_dir", { form.ttsDictDir }, { form.ttsDictDir = it; form.dirty = true })
            labeledField("lexicon", { form.ttsLexicon }, { form.ttsLexicon = it; form.dirty = true })
            labeledField("lang", { form.ttsLang }, { form.ttsLang = it; form.dirty = true })
            labeledField("speaker", { form.ttsSpeaker }, { form.ttsSpeaker = it; form.dirty = true })
            labeledField("speed", { form.ttsSpeed }, { form.ttsSpeed = it; form.dirty = true })
            labeledField("num_threads", { form.ttsNumThreads }, { form.ttsNumThreads = it; form.dirty = true })
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
                isOpen = { form.openDropdown == "tts_remote_engine" },
                onToggle = { form.openDropdown = if (form.openDropdown == "tts_remote_engine") "" else "tts_remote_engine" },
                onSelect = {
                    form.ttsBackend = it
                    form.lastRemoteEngine = it
                    form.dirty = true
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
                labeledField("app_id", { form.xfyunAppId }, { form.xfyunAppId = it; form.dirty = true })
                labeledField("api_key", { form.xfyunApiKey }, { form.xfyunApiKey = it; form.dirty = true }, secretHint(form.xfyunApiKeyConfigured))
                labeledField("api_secret", { form.xfyunApiSecret }, { form.xfyunApiSecret = it; form.dirty = true }, secretHint(form.xfyunApiSecretConfigured))
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
                    isOpen = { form.openDropdown == "xfyun_voice_group" },
                    onToggle = { form.openDropdown = if (form.openDropdown == "xfyun_voice_group") "" else "xfyun_voice_group" },
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
                        isOpen = { form.openDropdown == "xfyun_voice_type" },
                        onToggle = { form.openDropdown = if (form.openDropdown == "xfyun_voice_type") "" else "xfyun_voice_type" },
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
                        isOpen = { form.openDropdown == "xfyun_voice" },
                        onToggle = { form.openDropdown = if (form.openDropdown == "xfyun_voice") "" else "xfyun_voice" },
                        onSelect = {
                            form.xfyunVoice = it
                            form.dirty = true
                            form.openDropdown = ""
                        },
                    )
                }
                vif({ form.xfyunVoiceGroup == "custom" }) {
                    labeledField("voice（手填 vcn）", { form.xfyunVoice }, { form.xfyunVoice = it; form.dirty = true })
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


fun ViewContainer<*, *>.llmConfigCard(form: ConfigFormState) {
    groupedCard("LLM") {
        // 无 backend 选择：LLM 恒为 OpenAI 兼容 HTTP（Responses API），mock 已移除
        labeledField("api_base", { form.llmApiBase }, { form.llmApiBase = it; form.dirty = true })
        // 密钥不回传明文（服务端打码）：已配置时提示"留空则不修改"
        labeledField("api_key", { form.llmApiKey }, { form.llmApiKey = it; form.dirty = true }, secretHint(form.llmApiKeyConfigured))
        labeledField("model", { form.llmModel }, { form.llmModel = it; form.dirty = true })
        // 系统提示词是散文输入：官方 TextArea 多行（此前用单行 Input 输不了换行）
        labeledTextArea("system_prompt", { form.llmSystemPrompt }, { form.llmSystemPrompt = it; form.dirty = true }, height = 100f)
        labeledField("max_history", { form.llmMaxHistory }, { form.llmMaxHistory = it; form.dirty = true })
        labeledField("temperature", { form.llmTemperature }, { form.llmTemperature = it; form.dirty = true })
        switchRow("stream（SSE 流式）", form.llmStream == "true") {
            form.llmStream = if (form.llmStream == "true") "false" else "true"; form.dirty = true
        }
    }
}

/**
 * AIUI 全链路（极速超拟人）配置卡。
 *
 * ⚠️ 与前面的能力**互斥**：启用后设备流水线变为 VAD 切段 → 讯飞 AIUI 云端
 * （ASR + 大模型 + TTS 闭环）→ 下行音频，本地 ASR/LLM/TTS 引擎全部闲置
 *（`[soul]`/`[memory]` 这类本地上下文能力同样不生效）。
 *
 * 此前管理页**完全没有这一段**（`renderForm` 只有六个 tab），用户在 TOML 里配好
 * 全链路模式后，只要在管理页点一次保存就会被静默关掉（根因见
 * `docs/plugin-architecture-unification.md` §1.7 bug-1：前端不发 + 后端整份覆盖）。
 * 后端已改为部分更新语义兜底，这里把前端补齐，使该模式可在 UI 里配置与保留。
 */
fun ViewContainer<*, *>.aiuiConfigCard(form: ConfigFormState) {
    groupedCard("AIUI 全链路") {
        // 整段档位提示（AIUI 决定整条流水线走向，启用/切换只能重启生效）
        vif({ form.schema.sectionNote("aiui").isNotEmpty() }) {
            textNote(form.schema.sectionNote("aiui"), micro = true)
        }
        switchRow("enabled（走云端闭环）", form.aiuiEnabled == "true") {
            form.aiuiEnabled = if (form.aiuiEnabled == "true") "false" else "true"
            form.dirty = true
        }
        vif({ form.aiuiEnabled == "true" }) {
            Text {
                attr {
                    fontSize(AdminType.caption)
                    color(AdminColors.warningTintText)
                    marginBottom(AdminSpace.xs)
                    text("⚠️ 启用后本地 ASR / LLM / TTS 全部闲置（云端闭环计费）；「灵魂」「记忆」等本地上下文能力不生效。")
                }
            }
            labeledField("appid", { form.aiuiAppid }, { form.aiuiAppid = it; form.dirty = true })
            labeledField("api_key", { form.aiuiApiKey }, { form.aiuiApiKey = it; form.dirty = true }, secretHint(form.aiuiApiKeyConfigured))
            labeledField("api_secret", { form.aiuiApiSecret }, { form.aiuiApiSecret = it; form.dirty = true }, secretHint(form.aiuiApiSecretConfigured))
            labeledField("scene", { form.aiuiScene }, { form.aiuiScene = it; form.dirty = true }, "main_box（测试）/ main（生产）")
            labeledField("sn_prefix", { form.aiuiSnPrefix }, { form.aiuiSnPrefix = it; form.dirty = true })
            labeledField("voice", { form.aiuiVoice }, { form.aiuiVoice = it; form.dirty = true }, "x6_dongmanshaonv_pro")
            labeledField("speed（0~100）", { form.aiuiSpeed }, { form.aiuiSpeed = it; form.dirty = true }, "50")
            labeledField("volume（0~100）", { form.aiuiVolume }, { form.aiuiVolume = it; form.dirty = true }, "50")
            labeledField("pitch（0~100）", { form.aiuiPitch }, { form.aiuiPitch = it; form.dirty = true }, "50")
            labeledTextArea("prompt（人设）", { form.aiuiPrompt }, { form.aiuiPrompt = it; form.dirty = true }, height = 80f)
            labeledField("pace_ms", { form.aiuiPaceMs }, { form.aiuiPaceMs = it; form.dirty = true }, "10")
            Text {
                attr {
                    fontSize(AdminType.caption)
                    color(AdminColors.textTertiary)
                    text("pace_ms = 上行音频分帧推送间隔：云端流式 VAD 按实时节奏处理，瞬时灌入整段会被判为静音。")
                }
            }
        }
        vif({ form.aiuiEnabled != "true" }) {
            Text {
                attr {
                    fontSize(AdminType.caption)
                    color(AdminColors.textTertiary)
                    text("未启用：设备按本地级联流水线工作（SenseVoice ASR → 本地 LLM → TTS）。")
                }
            }
        }
    }
}

/**
 * 密钥输入框的占位提示。
 *
 * 服务端 `GET /api/config` 已打码（只回 `has_<field>` presence 标记），因此：
 * - `configured = true` 且输入框为空 → 提示"留空则不修改"，避免用户误以为密钥丢了、或
 *   以为必须重填（重填会覆盖）；
 * - `configured = false` → 明确提示尚未配置。
 */
private fun secretHint(configured: Boolean): String =
    if (configured) "已配置（留空则不修改）" else "未配置"
