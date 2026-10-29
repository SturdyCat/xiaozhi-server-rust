package com.xiaozhi.admin

import androidx.compose.runtime.Composable
import com.gearui.components.segmented.SegmentedControl
import com.gearui.foundation.layout.Spacing
import com.gearui.foundation.primitives.Text
import com.gearui.theme.Theme
import com.tencent.kuikly.compose.foundation.layout.fillMaxWidth
import com.tencent.kuikly.compose.foundation.layout.padding
import com.tencent.kuikly.compose.ui.Modifier
import com.xiaozhi.admin.kit.labeledIntField

/**
 * 「Playback · 播放场景」卡（`[playback]`）—— Compose 版。
 *
 * ## 为什么是独立一段（不是 `[audio]` 的子项）
 *
 * NextSession 的**场景策略**与 RestartOnly 的 `hello`/音频协商是两件事：`[playback]`
 * 只调"开播时机"（交互继承现场 vs 播报先蓄水），不改协商档位，所以服务端把它放在独立键里。
 * 界面沿用旧版结构：挂在「音频与播放」页的**末尾**（旧 `audioConfigCard` 的最后一行就是
 * `playbackConfigCard(form)`），一页只出现一个保存按钮。
 *
 * ## 组件映射（与旧 DSL 逐条对应，不是自由发挥）
 *
 * | 旧 DSL | 现在 |
 * | --- | --- |
 * | `groupedCard("Playback · 播放场景")` | [LabeledCard] |
 * | `textNote(sectionNote("playback"))` | [HintLine]（服务端 schema 是唯一权威） |
 * | `segmentedChoice(场景策略, 交互/播报)` | GearUI `SegmentedControl`（自带选中块动画） |
 * | `vif { playbackMode == "announcement" }` | `if (form.playbackMode == "announcement")` |
 * | `vif { playbackMode !in … }` | 未知模式告警行（不静默纠正） |
 *
 * ## 状态语义（迁移前如此，必须保留）
 *
 * 服务端若保存了客户端**未识别**的场景模式，界面既不隐藏也不自动改写，而是明确告知
 * "不会自动改成另一种模式"——静默纠正会让用户在两端之间来回踩坑。
 */
@Composable
fun PlaybackConfigCard(form: ConfigFormState) {
    LabeledCard(title = "Playback · 播放场景") {
        val note = form.schema.sectionNote("playback")
        if (note.isNotEmpty()) HintLine(note)

        // 场景策略：交互 / 播报（互斥二选一，不提供"两者都开"的伪组合）
        Text(
            text = "场景策略",
            modifier = Modifier.fillMaxWidth().padding(top = Spacing.sm, bottom = Spacing.xs),
            style = Theme.typography.label,
            color = Theme.colors.foreground,
        )
        SegmentedControl(
            options = PLAYBACK_MODES.map { mode -> mode.first },
            selectedOption = form.playbackMode,
            onOptionSelected = { mode ->
                form.playbackMode = mode
                form.markDirty()
            },
            modifier = Modifier.fillMaxWidth(),
            labelProvider = { mode -> PLAYBACK_MODES.firstOrNull { it.first == mode }?.second ?: mode },
        )

        HintLine(
            "交互：继承已有终端协商、起跑闸与节拍，保持 ESP 兼容。播报：只增加有上限的服务端起播蓄水；" +
                "不会扩大固件播放队列，也不保证慢于实时的上游永不断音。",
        )

        // 播报场景才有的蓄水参数（交互场景不显示，避免"填了不生效"的困惑）
        if (form.playbackMode == "announcement") {
            labeledIntField(
                label = "播报起播蓄水",
                value = form.announcementStartMs,
                onValueChange = {
                    form.announcementStartMs = it
                    form.markDirty()
                },
                placeholder = "5000",
                note = form.schema.fieldNote("playback", "announcement_start_ms"),
                help = "播报场景下开播前先攒多久的音频（默认 5s）：攒得越多越抗上游变慢，" +
                    "但播报开始更晚。目标蓄水会按服务器水库容量自动收敛，短回复立即放行。",
            )
            labeledIntField(
                label = "播报最长等待",
                value = form.announcementStartMaxMs,
                onValueChange = {
                    form.announcementStartMaxMs = it
                    form.markDirty()
                },
                placeholder = "10000",
                note = form.schema.fieldNote("playback", "announcement_start_max_ms"),
                help = "攒不够也强制开播的兜底上限（默认 10s），避免上游慢时一直不播。" +
                    "有效编码、分帧与预算可在输出节点验证中查看。",
            )
        }

        // 未识别的场景模式：只告警，不纠正（服务端可能来自更新版本的客户端）
        if (form.playbackMode != "interactive" && form.playbackMode != "announcement") {
            Text(
                text = "⚠️ 此服务端保存了当前客户端未识别的场景模式（${form.playbackMode}）；不会自动改成另一种模式。",
                modifier = Modifier.fillMaxWidth().padding(top = Spacing.xs),
                style = Theme.typography.caption,
                color = Theme.colors.warning,
            )
        }
    }
}

/** 场景策略选项：`id → 中文标签`。顺序即分段控件的显示顺序（交互在前，播报在后）。 */
internal val PLAYBACK_MODES: List<Pair<String, String>> = listOf(
    "interactive" to "交互",
    "announcement" to "播报",
)
