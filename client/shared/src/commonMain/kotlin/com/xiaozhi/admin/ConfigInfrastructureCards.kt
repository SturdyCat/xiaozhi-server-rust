package com.xiaozhi.admin

import com.tencent.kuikly.core.base.ViewContainer
import com.tencent.kuikly.core.directives.vif
import com.tencent.kuikly.core.views.Text

/**
 * 基础设施类配置卡：Server / Audio / AIUI 全链路。
 *
 * 文案口径同 [ConfigStageCards]：中文名 + 按需 ⓘ（长说明进弹层），不再用常驻长段落解释字段。
 */
fun ViewContainer<*, *>.serverConfigCard(form: ConfigFormState) {
    groupedCard("Server · 服务") {
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
        labeledIntField(
            "监听端口", { form.port }, { form.port = it; form.markDirty() }, "8000",
            help = "WebSocket 与 HTTP 共用端口：设备固件、管理台、测试台都按它连接。" +
                "改端口后必须同步设备端配置，否则设备连不上。",
        )
        labeledField(
            "接入令牌", { form.expectedToken }, { form.expectedToken = it; form.markDirty() },
            help = "设备与测试台连接时校验的令牌（hello 帧携带）。留空 = 不校验，仅建议在内网使用。",
        )
        labeledIntField(
            "工作线程数", { form.workerThreads }, { form.workerThreads = it; form.markDirty() },
            help = "异步运行时的工作线程数；留空/0 = 按 CPU 核数自动决定。CPU 富余时调大能缓解多设备并发。",
        )
        labeledField(
            "管理页目录", { form.adminDir }, { form.adminDir = it; form.markDirty() },
            help = "管理台前端静态资源目录（含 index.html 与 assets），由服务端同域托管，" +
                "因此浏览器打开 / 即为管理台。填错会 404。",
        )
    }
}

fun ViewContainer<*, *>.audioConfigCard(form: ConfigFormState) {
    groupedCard("Audio · 音频与播放") {
        // 整段档位提示：下行参数与 hello/OTA 协商强绑定，全部只能重启生效
        vif({ form.schema.sectionNote("audio").isNotEmpty() }) {
            textNote(form.schema.sectionNote("audio"), micro = true)
        }
        labeledIntField(
            "下行采样率", { form.downlinkSampleRate }, { form.downlinkSampleRate = it; form.markDirty() },
            help = "下发给设备的音频采样率（Hz），在 hello/OTA 协商时确认。必须与固件解码配置一致，" +
                "不一致会变调或杂音，且只能重启生效。",
        )
        labeledIntField(
            "下行帧长", { form.downlinkFrameMs }, { form.downlinkFrameMs = it; form.markDirty() },
            help = "每包音频的时长（ms）。帧短首包更快但包数更多，帧长更省带宽但延迟更高；改它要同步固件解码队列。",
        )
        labeledIntField(
            "声道数", { form.channels }, { form.channels = it; form.markDirty() },
            help = "下行音频声道数，通常 1（单声道）；固件按协商值解码，填错会只有噪声。",
        )
        labeledIntField(
            "二进制协议版本", { form.binaryProtocolVersion }, { form.binaryProtocolVersion = it; form.markDirty() },
            help = "与固件协商的二进制帧协议版本。固件不认识时会退回 JSON 文本帧（更慢、更费流量）。",
        )
        // 抖动缓冲提前量：下发时间表比实时提前该时长，使设备解码队列常备音频存货，
        // 吸收网络抖动 + 上游 LLM 出句/TTS 合成的停顿（长回复断音的主要来源）。
        labeledIntField(
            "下行提前量", { form.downlinkLeadMs }, { form.downlinkLeadMs = it; form.markDirty() }, "1000",
            help = "下发时间表比实时提前该时长，让设备解码队列常备存货，吸收网络抖动与" +
                "上游出句/合成停顿（长回复断音的主要来源）。缓冲越深，长回复越抗上游变慢" +
                "（默认 1000ms，接近设备端极限 1080ms）。想要更深必须先放大固件解码队列 " +
                "MAX_DECODE_PACKETS_IN_QUEUE，并同步「设备解码队列」。代价：打断后设备会把" +
                "已缓存的这部分音频播完才停口。",
        )
        labeledIntField(
            "设备解码队列", { form.downlinkQueuePackets }, { form.downlinkQueuePackets = it; form.markDirty() }, "20",
            help = "设备端解码队列可容纳的包数，决定「提前量」能开多大（约 1.2s 上限）。" +
                "改这里要同时改固件的 MAX_DECODE_PACKETS_IN_QUEUE，否则只是改了个数字。",
        )
        // 服务器侧缓冲：设备队列只有 ~1.2s，上游慢于实时（LLM 逐句吐字 / 在线合成首片）
        // 时唯一还能加深缓冲的地方——用服务器内存换下行连续性。
        labeledIntField(
            "服务器水库容量", { form.downlinkBufferMs }, { form.downlinkBufferMs = it; form.markDirty() }, "30000",
            help = "服务器侧缓冲的总时长上限：TTS 流下载 → PCM 下载缓冲（按秒限速，由水库容量派生、" +
                "上限 10s）→ 已编码 Opus 水库（默认 30s ≈ 60KB/会话）→ 按实时节拍下发。" +
                "上游停顿不超过「水库水位 + 设备队列」时听感全程连续。⚠️ 上游**持续**慢于实时" +
                "（每轮日志「本轮吞吐 ≈ N× 实时」，N<1）时缺口会累积，缓冲只能推迟断音——" +
                "根治要提速上游（提高 TTS 语速 / 换更快的 LLM）。",
        )
        labeledIntField(
            "起跑闸", { form.downlinkBufferStartMs }, { form.downlinkBufferStartMs = it; form.markDirty() }, "1500",
            help = "开始播放前先攒多久的音频（默认 1.5s）。攒够即开播，短回复会立即放行；" +
                "调大更抗抖动，但第一声更晚。",
        )
        labeledIntField(
            "起跑闸最长等待", { form.downlinkBufferStartMaxMs }, { form.downlinkBufferStartMaxMs = it; form.markDirty() }, "4000",
            help = "起跑闸的兜底上限：等这么久还没攒够也强制开播，避免上游慢时一直不出声。",
        )
        textNote("下行参数与 hello/OTA 协商强绑定，改动均需重启 server 生效；设备侧需固件同步调整。", micro = true)
    }
    playbackConfigCard(form)
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
    groupedCard("AIUI · 全链路模式") {
        // 整段档位提示（AIUI 决定整条流水线走向，启用/切换只能重启生效）
        vif({ form.schema.sectionNote("aiui").isNotEmpty() }) {
            textNote(form.schema.sectionNote("aiui"), micro = true)
        }
        switchRow(
            "启用 AIUI 全链路", form.aiuiEnabled == "true",
            help = "识别、理解、合成全部交给讯飞 AIUI 云端闭环（延迟最低、音色最自然）。" +
                "启用后本地 ASR / LLM / TTS 全部闲置，「人格」「记忆」等本地上下文能力不再参与；" +
                "按云端调用计费，且只能重启生效。",
        ) {
            form.toggleStringBool(form::aiuiEnabled)
        }
        vif({ form.aiuiEnabled == "true" }) {
            textNote(
                "⚠️ 本地 ASR / LLM / TTS 全部闲置（云端闭环计费）；「人格」「记忆」等本地上下文能力不生效。",
                tone = "warn",
                micro = true,
            )
            labeledField("应用 AppID", { form.aiuiAppid }, { form.aiuiAppid = it; form.markDirty() })
            labeledField(
                "API Key", { form.aiuiApiKey }, { form.aiuiApiKey = it; form.markDirty() },
                secretHint(form.aiuiApiKeyConfigured),
            )
            labeledField(
                "API Secret", { form.aiuiApiSecret }, { form.aiuiApiSecret = it; form.markDirty() },
                secretHint(form.aiuiApiSecretConfigured),
            )
            labeledField(
                "场景", { form.aiuiScene }, { form.aiuiScene = it; form.markDirty() }, "main_box（测试）/ main（生产）",
                help = "讯飞控制台里的「场景」标识：测试环境用 main_box，正式对外用 main。" +
                    "两者计费与配额不同，上线前务必换成 main。",
            )
            labeledField(
                "设备 SN 前缀", { form.aiuiSnPrefix }, { form.aiuiSnPrefix = it; form.markDirty() },
                help = "拼在设备序列号前面，用于在讯飞侧区分同一应用下的不同设备；多台设备可共用同一前缀。",
            )
            labeledField(
                "发音人", { form.aiuiVoice }, { form.aiuiVoice = it; form.markDirty() }, "x6_dongmanshaonv_pro",
                help = "讯飞的发音人标识（vcn）。极速拟人系列以 x6_ 开头，可在讯飞控制台试听后复制。",
            )
            labeledIntField(
                "语速", { form.aiuiSpeed }, { form.aiuiSpeed = it; form.markDirty() }, "50",
                help = "0~100，50 为正常语速；调快可缓解下行停顿，但过快会影响清晰度。",
            )
            labeledIntField(
                "音量", { form.aiuiVolume }, { form.aiuiVolume = it; form.markDirty() }, "50",
                help = "0~100，云端合成时的音量基准；设备端还有独立音量控制，两者相乘生效。",
            )
            labeledIntField(
                "音调", { form.aiuiPitch }, { form.aiuiPitch = it; form.markDirty() }, "50",
                help = "0~100，50 为原始音高；调高更年轻、调低更沉稳。",
            )
            labeledTextArea(
                "人设提示词", { form.aiuiPrompt }, { form.aiuiPrompt = it; form.markDirty() },
                height = 80f,
                help = "云端大模型的人设与规则（相当于本地的系统提示词）。本地「人格档案」在此模式下不生效。",
            )
            labeledIntField(
                "上行分帧间隔", { form.aiuiPaceMs }, { form.aiuiPaceMs = it; form.markDirty() }, "10",
                help = "上行音频按该间隔（ms）分帧推送：云端流式 VAD 按实时节奏判断说话状态，" +
                    "瞬时灌入整段会被判为静音、拿不到识别结果。",
            )
        }
        vif({ form.aiuiEnabled != "true" }) {
            textNote("未启用：设备按本地级联流水线工作（SenseVoice ASR → 本地 LLM → TTS）。", micro = true)
        }
    }
}
