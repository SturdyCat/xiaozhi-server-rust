package com.xiaozhi.admin

import androidx.compose.runtime.Composable
import com.gearui.components.alert.Alert
import com.gearui.components.alert.AlertStatus
import com.tencent.kuikly.compose.foundation.layout.fillMaxWidth
import com.tencent.kuikly.compose.foundation.layout.padding
import com.tencent.kuikly.compose.ui.Modifier
import com.gearui.foundation.layout.Spacing
import com.xiaozhi.admin.kit.labeledField
import com.xiaozhi.admin.kit.switchRow

/**
 * 「唤醒」配置卡（`[wakeup]`）：**唤醒 → LLM 之间**的唤醒问候语（Compose 版）。
 *
 * 打开后，设备唤醒词命中（固件立刻上报 `listen state=detect`）时服务器马上说一句预制话术
 * （默认「我在」），**跳过 LLM 与正常回复**——唤醒是全系统最高频的动作，为一句固定的话付一次
 * 大模型调用没有道理。成品音频按音色三级复用：内存缓存（跨会话）→ 磁盘（跨重启）→
 * 真合成一次后自动落盘；稳态下每次唤醒**零 LLM、零 TTS**。
 *
 * 三条 ⚠️ 提示（问候语过长、缓存关闭导致落盘静默失效）是跨字段/跨段的一致性校验，
 * 与旧实现逐字一致：它们说明的正是"这个组合会静默失效"，删掉就等于把坑留给用户。
 */
@Composable
fun WakeupConfigCard(form: ConfigFormState) {
    val wake = form.wake
    LabeledCard(title = "唤醒问候语（唤醒 → LLM）") {
        // 整段档位提示（保存后新会话生效）：来自 /api/config/schema，服务端为唯一权威
        val sectionNote = form.schema.sectionNote("wakeup")
        if (sectionNote.isNotEmpty()) HintLine(sectionNote)

        switchRow(
            label = "启用唤醒问候",
            checked = wake.enabled == "true",
            onToggle = { form.toggleStringBool(wake::enabled) },
            help = "设备唤醒词命中后（需固件 CONFIG_SEND_WAKE_WORD_DATA=y），服务器立刻说一句问候语，" +
                "不再调用大模型、也不进入正常回复。默认关闭；关闭时唤醒后静默等待用户说话。",
        )

        if (wake.enabled == "true") {
            labeledField(
                label = "问候语",
                value = wake.greeting,
                onValueChange = {
                    wake.greeting = it
                    form.markDirty()
                },
                placeholder = "我在",
                help = "改这句话会换缓存键，因此会自动重新合成一次并落盘；之后唤醒直接复用。" +
                    "建议 ≤ 10 字：它播放期间用户的第一句话会在缓冲区排队，等它说完才被处理" +
                    "（超过 30 字服务端会直接拦下并提示）。",
            )
            // 问候语是下行音频，播放期间上行音频在缓冲区排队——太长会推迟用户第一句话。
            if (wake.greeting.trim().length > 10) {
                Alert(
                    title = "问候语偏长（建议 ≤ 10 字）",
                    description = "它播放期间用户的第一句话会在缓冲区排队，等它说完才被处理。",
                    status = AlertStatus.WARNING,
                    modifier = Modifier.fillMaxWidth().padding(top = Spacing.xs),
                )
            }
            switchRow(
                label = "成品音频落盘复用",
                checked = wake.persist == "true",
                onToggle = { form.toggleStringBool(wake::persist) },
                help = "把成品音频（裸 24kHz PCM，约 48KB/秒，ffplay -f s16le -ar 24000 可直接播放）" +
                    "写到配置文件同级的 hot-audio/ 目录，文件名即内容哈希；容器重建后仍在挂载卷上。" +
                    "下行格式（采样率/帧长）不影响复用，换音色会自动重新合成并另存一份，不会串用旧音色。",
            )
            // ⚠️ 跨段矛盾必须显式说出来：复用经内存缓存承载成品帧，缓存关掉时落盘会静默失效。
            if (wake.persist == "true" && (form.ttsCacheEntries.trim().toIntOrNull() ?: 256) <= 0) {
                Alert(
                    title = "音频无法落盘：TTS 结果缓存已关闭",
                    description = "当前「TTS · 结果缓存条目数」= 0，每次唤醒都会重新合成。" +
                        "请到「TTS」页把结果缓存条目数设为大于 0（推荐 256），或把这里的落盘复用关掉。" +
                        "保存时服务端也会拦下并提示。",
                    status = AlertStatus.WARNING,
                    modifier = Modifier.fillMaxWidth().padding(top = Spacing.xs),
                )
            }
        } else {
            HintLine("未启用：唤醒后静默等待用户说话（行为与加本插件之前一致）。")
        }
    }
}
