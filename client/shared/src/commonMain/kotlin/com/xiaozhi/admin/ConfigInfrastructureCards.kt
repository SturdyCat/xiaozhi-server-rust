package com.xiaozhi.admin

import androidx.compose.runtime.Composable
import com.xiaozhi.admin.kit.labeledField
import com.xiaozhi.admin.kit.labeledIntField
import com.xiaozhi.admin.kit.labeledTextArea
import com.xiaozhi.admin.kit.switchRow

/**
 * 基础设施类配置卡：Server / AIUI 全链路（Compose 版，T08）。
 *
 * 「Audio · 音频与播放」已在 T07 迁走（`ConfigAudioCard.kt`），本文件只剩两个结构型 section。
 *
 * 文案口径同 `ConfigStageCards`：中文名 + 按需 ⓘ（长说明进弹层），不写常驻长段落解释字段。
 * 这两段都属**结构型**字段：始终是全局值，不参与设备 overlay 差量（设备模式改了也只影响全局）。
 */

/**
 * 「Server · 服务」section（`server`）：端口 / 鉴权 / 工作线程 / 管理页目录。
 *
 * 为什么这几项放在「全局配置」而不是语音流程里：它们在流水线里没有节点位置，
 * 改了影响所有会话与所有设备连接（`[server]` 段只能重启生效）。
 */
@Composable
fun ServerSection(form: ConfigFormState) {
    LabeledCard(title = "Server · 服务") {
        // 配置来源（路径 + 持久化）：一眼定位"改配置不生效 / 重启即丢"类问题
        if (form.configMetaMsg.isNotEmpty()) HintLine(form.configMetaMsg)
        // 整段档位提示（来自 /api/config/schema 的 app_sections，服务端为唯一权威）
        val note = form.schema.sectionNote("server")
        if (note.isNotEmpty()) HintLine(note)

        labeledIntField(
            label = "监听端口",
            value = form.port,
            onValueChange = {
                form.port = it
                form.markDirty()
            },
            placeholder = "8000",
            help = "WebSocket 与 HTTP 共用端口：设备固件、管理台、测试台都按它连接。" +
                "改端口后必须同步设备端配置，否则设备连不上。",
        )
        labeledField(
            label = "接入令牌",
            value = form.expectedToken,
            onValueChange = {
                form.expectedToken = it
                form.markDirty()
            },
            help = "设备与测试台连接时校验的令牌（hello 帧携带）。留空 = 不校验，仅建议在内网使用。",
        )
        labeledIntField(
            label = "工作线程数",
            value = form.workerThreads,
            onValueChange = {
                form.workerThreads = it
                form.markDirty()
            },
            help = "异步运行时的工作线程数；留空/0 = 按 CPU 核数自动决定。CPU 富余时调大能缓解多设备并发。",
        )
        labeledField(
            label = "管理页目录",
            value = form.adminDir,
            onValueChange = {
                form.adminDir = it
                form.markDirty()
            },
            help = "管理台前端静态资源目录（含 index.html 与 assets），由服务端同域托管，" +
                "因此浏览器打开 / 即为管理台。填错会 404。",
        )
    }
}

/**
 * 「AIUI · 全链路模式」section（`aiui`，`capabilityId = fullchain`）。
 *
 * ⚠️ 与本地级联能力**互斥**：启用后设备流水线变为 VAD 切段 → 讯飞 AIUI 云端
 * （ASR + 大模型 + TTS 闭环）→ 下行音频，本地 ASR/LLM/TTS 引擎全部闲置
 *（`[soul]`/`[memory]` 这类本地上下文能力同样不生效）——这条互斥说明必须常驻在卡上，
 * 不能只写进 help 弹层。
 *
 * 语义不变：启用/切换只能重启生效；字段整段回传（前端不回传就等于"这一段在 UI 里不可配置"）。
 */
@Composable
fun AiuiSection(form: ConfigFormState) {
    LabeledCard(title = "AIUI · 全链路模式") {
        // 整段档位提示（AIUI 决定整条流水线走向，启用/切换只能重启生效）
        val note = form.schema.sectionNote("aiui")
        if (note.isNotEmpty()) HintLine(note)

        switchRow(
            label = "启用 AIUI 全链路",
            checked = form.aiuiEnabled == "true",
            onToggle = { form.toggleStringBool(form::aiuiEnabled) },
            help = "识别、理解、合成全部交给讯飞 AIUI 云端闭环（延迟最低、音色最自然）。" +
                "启用后本地 ASR / LLM / TTS 全部闲置，「人格」「记忆」等本地上下文能力不再参与；" +
                "按云端调用计费，且只能重启生效。",
        )

        if (form.aiuiEnabled == "true") {
            WarnLine("本地 ASR / LLM / TTS 全部闲置（云端闭环计费）；「人格」「记忆」等本地上下文能力不生效。")
            labeledField(
                label = "应用 AppID",
                value = form.aiuiAppid,
                onValueChange = {
                    form.aiuiAppid = it
                    form.markDirty()
                },
            )
            labeledField(
                label = "API Key",
                value = form.aiuiApiKey,
                onValueChange = {
                    form.aiuiApiKey = it
                    form.markDirty()
                },
                note = secretHint(form.aiuiApiKeyConfigured),
            )
            labeledField(
                label = "API Secret",
                value = form.aiuiApiSecret,
                onValueChange = {
                    form.aiuiApiSecret = it
                    form.markDirty()
                },
                note = secretHint(form.aiuiApiSecretConfigured),
            )
            labeledField(
                label = "场景",
                value = form.aiuiScene,
                onValueChange = {
                    form.aiuiScene = it
                    form.markDirty()
                },
                placeholder = "main_box（测试）/ main（生产）",
                help = "讯飞控制台里的「场景」标识：测试环境用 main_box，正式对外用 main。" +
                    "两者计费与配额不同，上线前务必换成 main。",
            )
            labeledField(
                label = "设备 SN 前缀",
                value = form.aiuiSnPrefix,
                onValueChange = {
                    form.aiuiSnPrefix = it
                    form.markDirty()
                },
                help = "拼在设备序列号前面，用于在讯飞侧区分同一应用下的不同设备；多台设备可共用同一前缀。",
            )
            labeledField(
                label = "发音人",
                value = form.aiuiVoice,
                onValueChange = {
                    form.aiuiVoice = it
                    form.markDirty()
                },
                placeholder = "x6_dongmanshaonv_pro",
                help = "讯飞的发音人标识（vcn）。极速拟人系列以 x6_ 开头，可在讯飞控制台试听后复制。",
            )
            labeledIntField(
                label = "语速",
                value = form.aiuiSpeed,
                onValueChange = {
                    form.aiuiSpeed = it
                    form.markDirty()
                },
                placeholder = "50",
                help = "0~100，50 为正常语速；调快可缓解下行停顿，但过快会影响清晰度。",
            )
            labeledIntField(
                label = "音量",
                value = form.aiuiVolume,
                onValueChange = {
                    form.aiuiVolume = it
                    form.markDirty()
                },
                placeholder = "50",
                help = "0~100，云端合成时的音量基准；设备端还有独立音量控制，两者相乘生效。",
            )
            labeledIntField(
                label = "音调",
                value = form.aiuiPitch,
                onValueChange = {
                    form.aiuiPitch = it
                    form.markDirty()
                },
                placeholder = "50",
                help = "0~100，50 为原始音高；调高更年轻、调低更沉稳。",
            )
            labeledTextArea(
                label = "人设提示词",
                value = form.aiuiPrompt,
                onValueChange = {
                    form.aiuiPrompt = it
                    form.markDirty()
                },
                minLines = 3,
                help = "云端大模型的人设与规则（相当于本地的系统提示词）。本地「人格档案」在此模式下不生效。",
            )
            labeledIntField(
                label = "上行分帧间隔",
                value = form.aiuiPaceMs,
                onValueChange = {
                    form.aiuiPaceMs = it
                    form.markDirty()
                },
                placeholder = "10",
                help = "上行音频按该间隔（ms）分帧推送：云端流式 VAD 按实时节奏判断说话状态，" +
                    "瞬时灌入整段会被判为静音、拿不到识别结果。",
            )
        } else {
            HintLine("未启用：设备按本地级联流水线工作（SenseVoice ASR → 本地 LLM → TTS）。")
        }
    }
}
