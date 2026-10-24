package com.xiaozhi.admin

import com.xiaozhi.admin.ui.*

import com.tencent.kuikly.core.base.ViewContainer
import com.tencent.kuikly.core.directives.vif

/** NextSession scenario policy is separate from RestartOnly hello/audio negotiation. */
fun ViewContainer<*, *>.playbackConfigCard(form: ConfigFormState) {
    groupedCard("Playback · 播放场景") {
        textNote({ form.schema.sectionNote("playback") }, micro = true)
        segmentedChoice(
            "场景策略",
            listOf(ChoiceOption("interactive", "交互", IconToken.Waveform), ChoiceOption("announcement", "播报", IconToken.Speaker)),
            selectedValue = { form.playbackMode },
            onSelect = { form.playbackMode = it; form.markDirty() },
        )
        textNote(
            "交互：继承已有终端协商、起跑闸与节拍，保持 ESP 兼容。播报：只增加有上限的服务端起播蓄水；" +
                "不会扩大固件播放队列，也不保证慢于实时的上游永不断音。",
            micro = true,
        )
        vif({ form.playbackMode == "announcement" }) {
            labeledIntField(
                "播报起播蓄水", { form.announcementStartMs },
                { form.announcementStartMs = it; form.markDirty() }, "5000",
                note = { form.schema.fieldNote("playback", "announcement_start_ms") },
                help = "播报场景下开播前先攒多久的音频（默认 5s）：攒得越多越抗上游变慢，" +
                    "但播报开始更晚。目标蓄水会按服务器水库容量自动收敛，短回复立即放行。",
            )
            labeledIntField(
                "播报最长等待", { form.announcementStartMaxMs },
                { form.announcementStartMaxMs = it; form.markDirty() }, "10000",
                note = { form.schema.fieldNote("playback", "announcement_start_max_ms") },
                help = "攒不够也强制开播的兜底上限（默认 10s），避免上游慢时一直不播。" +
                    "有效编码、分帧与预算可在输出节点验证中查看。",
            )
        }
        vif({ form.playbackMode !in listOf("interactive", "announcement") }) {
            textNote("此服务端保存了当前客户端未识别的场景模式；不会自动改成另一种模式。", tone = "warn", micro = true)
        }
    }
}
