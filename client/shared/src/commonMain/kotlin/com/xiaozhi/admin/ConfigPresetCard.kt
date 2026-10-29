package com.xiaozhi.admin

import androidx.compose.runtime.Composable
import com.gearui.components.button.Button
import com.gearui.components.button.ButtonTheme
import com.gearui.components.button.ButtonType
import com.gearui.foundation.layout.Spacing
import com.gearui.foundation.primitives.Text
import com.gearui.theme.Theme
import com.tencent.kuikly.compose.foundation.layout.Arrangement
import com.tencent.kuikly.compose.foundation.layout.Row
import com.tencent.kuikly.compose.foundation.layout.fillMaxWidth
import com.tencent.kuikly.compose.foundation.layout.padding
import com.tencent.kuikly.compose.ui.Alignment
import com.tencent.kuikly.compose.ui.Modifier
import com.xiaozhi.admin.kit.actionRow
import com.xiaozhi.admin.kit.dividerH
import com.xiaozhi.admin.kit.labeledField

/**
 * 「预设」section（`presets`）—— Compose 版：把当前**全局**配置快照为预设，必要时一键回滚。
 *
 * ## 为什么单独一个文件（而不是塞回 `ConfigCards.kt`）
 *
 * `ConfigCards.kt` 是 section 分发器（T07 交付物，每行一个分支）；把一张有状态的卡塞进分发器
 * 会让"分发"与"实现"混在一处，后续每次加 section 都要在那棵树里追加逻辑。预设自带
 * 输入框 / 动作 / 列表，是完整的卡，故独立成文件。
 *
 * ## 语义（与旧实现逐条一致）
 *
 * - **只快照服务端盘上的生效值**，不是表单草稿 —— 保存前先在配置页点「保存配置」；
 * - **只作用于全局配置**：设备个性化模式（`editDeviceId` 非空）下**只显示禁用说明**，
 *   不渲染表单（否则用户会以为能按设备存预设，而服务端只有全局预设）；
 * - 应用预设成功后服务端会覆盖全局配置 → 表单必须重载（`applyPreset` 内部已做）；
 * - 失败文案与成功文案同样就地显示（不弹 toast：toast 只说"操作已完成"，失败原因要留在页面上）。
 */
@Composable
fun PresetSection(form: ConfigFormState) {
    // 设备个性化模式：预设卡不可用，只给禁用说明（原语义）
    if (form.editDeviceId.isNotEmpty()) {
        LabeledCard(title = "预设") {
            HintLine(
                "预设仅作用于全局配置；当前正在编辑设备的个性化配置，" +
                    "请先点顶部「返回全局」再管理预设。",
            )
        }
        return
    }

    val pager = LocalAdminPagerOrThrow()
    LabeledCard(title = "配置预设") {
        HintLine(
            "把当前全局配置快照为预设，随时一键应用（回滚）。预设仅作用于全局配置；" +
                "保存的是服务端盘上的生效值（不是表单草稿）。",
        )
        labeledField(
            label = "预设名",
            value = form.presetNameDraft,
            onValueChange = { form.presetNameDraft = it },
            placeholder = "如 night（字母数字与 - _ .）",
        )
        actionRow {
            Button(
                text = if (form.presetBusy) "保存中…" else "保存当前配置为预设",
                onClick = { form.savePreset(pager, form.serverBase) },
                type = ButtonType.OUTLINE,
                loading = form.presetBusy,
                disabled = form.presetBusy,
            )
        }
        // 结果就地显示：失败要留住原因（toast 只说"操作已完成"，会把原因冲掉）
        if (form.presetMsg.isNotEmpty()) {
            val failed = form.presetMsg.contains("失败")
            if (failed) WarnLine(form.presetMsg, error = true) else HintLine(form.presetMsg)
        }
        dividerH()
        if (form.presetNames.isEmpty()) {
            HintLine("还没有预设。填好名称点「保存当前配置为预设」即可创建。")
        }
        // Compose 循环：presetNames 是 SnapshotStateList，加载/删除后整表重建即重组
        for (name in form.presetNames) PresetRow(form, name, pager)
    }
}

/** 一行预设：名称 + 「应用」/「删除」。删除是幂等动作，不做二次确认（服务端留有其它预设可回滚）。 */
@Composable
private fun PresetRow(form: ConfigFormState, name: String, pager: com.tencent.kuikly.core.pager.Pager) {
    Row(
        modifier = Modifier.fillMaxWidth().padding(top = Spacing.xs),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(Spacing.sm),
    ) {
        Text(
            text = name,
            modifier = Modifier.weight(1f),
            style = Theme.typography.bodyMedium,
            color = Theme.colors.foreground,
        )
        Button(
            text = "应用",
            onClick = { form.applyPreset(pager, form.serverBase, name) },
            type = ButtonType.OUTLINE,
            disabled = form.presetBusy,
        )
        Button(
            text = "删除",
            onClick = { form.deletePreset(pager, form.serverBase, name) },
            type = ButtonType.OUTLINE,
            theme = ButtonTheme.DANGER,
            disabled = form.presetBusy,
        )
    }
}
