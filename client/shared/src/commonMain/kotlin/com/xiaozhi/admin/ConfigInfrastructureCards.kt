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
        labeledIntField("port", { form.port }, { form.port = it; form.markDirty() }, "8000")
        labeledField("expected_token", { form.expectedToken }, { form.expectedToken = it; form.markDirty() })
        labeledIntField("worker_threads", { form.workerThreads }, { form.workerThreads = it; form.markDirty() })
        labeledField("admin_dir", { form.adminDir }, { form.adminDir = it; form.markDirty() })
    }
}

fun ViewContainer<*, *>.audioConfigCard(form: ConfigFormState) {
    groupedCard("Audio") {
        // 整段档位提示：下行参数与 hello/OTA 协商强绑定，全部只能重启生效
        vif({ form.schema.sectionNote("audio").isNotEmpty() }) {
            textNote(form.schema.sectionNote("audio"), micro = true)
        }
        labeledIntField("downlink_sample_rate", { form.downlinkSampleRate }, { form.downlinkSampleRate = it; form.markDirty() })
        labeledIntField("downlink_frame_duration_ms", { form.downlinkFrameMs }, { form.downlinkFrameMs = it; form.markDirty() })
        labeledIntField("channels", { form.channels }, { form.channels = it; form.markDirty() })
        labeledIntField("binary_protocol_version", { form.binaryProtocolVersion }, { form.binaryProtocolVersion = it; form.markDirty() })
        // 抖动缓冲提前量：下发时间表比实时提前该时长，使设备解码队列常备音频存货，
        // 吸收网络抖动 + 上游 LLM 出句/TTS 合成的停顿（长回复断音的主要来源）。
        labeledIntField("downlink_lead_ms", { form.downlinkLeadMs }, { form.downlinkLeadMs = it; form.markDirty() }, "1000")
        labeledIntField("downlink_queue_packets", { form.downlinkQueuePackets }, { form.downlinkQueuePackets = it; form.markDirty() }, "20")
        Text {
            attr {
                fontSize(AdminType.caption)
                color(AdminColors.textTertiary)
                text("缓冲越深，长回复越抗上游变慢（默认 1000，接近设备端极限 1080）。想要更深必须先放大固件解码队列 MAX_DECODE_PACKETS_IN_QUEUE，并同步 [audio].downlink_queue_packets。代价：打断后设备会把已缓存的 ~提前量 音频播完才停口。")
            }
        }
        // 服务器侧缓冲：设备队列只有 ~1.2s，上游慢于实时（LLM 逐句吐字 / 在线合成首片）
        // 时唯一还能加深缓冲的地方——用服务器内存换下行连续性。
        labeledIntField("downlink_buffer_ms", { form.downlinkBufferMs }, { form.downlinkBufferMs = it; form.markDirty() }, "30000")
        labeledIntField("downlink_buffer_start_ms", { form.downlinkBufferStartMs }, { form.downlinkBufferStartMs = it; form.markDirty() }, "1500")
        labeledIntField("downlink_buffer_start_max_ms", { form.downlinkBufferStartMaxMs }, { form.downlinkBufferStartMaxMs = it; form.markDirty() }, "4000")
        Text {
            attr {
                fontSize(AdminType.caption)
                color(AdminColors.textTertiary)
                text("服务器缓冲三段：TTS 流下载 → PCM 下载缓冲（按秒限速，由水库容量派生、上限 10s）→ 已编码 Opus 水库（默认 30s ≈ 60KB/会话）→ 按实时节拍下发。上游停顿 ≤ 水库水位 + 设备队列 时听感全程连续。起跑闸先攒后播（默认 1.5s，短回复立即放行），最长等待 4s 兜底。⚠️ 上游**持续**慢于实时（每轮日志“本轮吞吐 ≈ N× 实时”，N<1）时缺口会累积，缓冲只能推迟断音——根治要提速上游（提高 [tts].speed / 换更快 LLM）。全部重启生效。")
            }
        }
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
    groupedCard("AIUI 全链路") {
        // 整段档位提示（AIUI 决定整条流水线走向，启用/切换只能重启生效）
        vif({ form.schema.sectionNote("aiui").isNotEmpty() }) {
            textNote(form.schema.sectionNote("aiui"), micro = true)
        }
        switchRow("enabled（走云端闭环）", form.aiuiEnabled == "true") {
            form.toggleStringBool(form::aiuiEnabled)
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
            labeledField("appid", { form.aiuiAppid }, { form.aiuiAppid = it; form.markDirty() })
            labeledField("api_key", { form.aiuiApiKey }, { form.aiuiApiKey = it; form.markDirty() }, secretHint(form.aiuiApiKeyConfigured))
            labeledField("api_secret", { form.aiuiApiSecret }, { form.aiuiApiSecret = it; form.markDirty() }, secretHint(form.aiuiApiSecretConfigured))
            labeledField("scene", { form.aiuiScene }, { form.aiuiScene = it; form.markDirty() }, "main_box（测试）/ main（生产）")
            labeledField("sn_prefix", { form.aiuiSnPrefix }, { form.aiuiSnPrefix = it; form.markDirty() })
            labeledField("voice", { form.aiuiVoice }, { form.aiuiVoice = it; form.markDirty() }, "x6_dongmanshaonv_pro")
            labeledIntField("speed（0~100）", { form.aiuiSpeed }, { form.aiuiSpeed = it; form.markDirty() }, "50")
            labeledIntField("volume（0~100）", { form.aiuiVolume }, { form.aiuiVolume = it; form.markDirty() }, "50")
            labeledIntField("pitch（0~100）", { form.aiuiPitch }, { form.aiuiPitch = it; form.markDirty() }, "50")
            labeledTextArea("prompt（人设）", { form.aiuiPrompt }, { form.aiuiPrompt = it; form.markDirty() }, height = 80f)
            labeledIntField("pace_ms", { form.aiuiPaceMs }, { form.aiuiPaceMs = it; form.markDirty() }, "10")
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
