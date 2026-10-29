package com.xiaozhi.admin

import androidx.compose.runtime.Composable
import com.gearui.components.button.Button
import com.gearui.components.button.ButtonType
import com.gearui.foundation.layout.Spacing
import com.gearui.foundation.primitives.Text
import com.gearui.theme.Theme
import com.tencent.kuikly.compose.foundation.layout.fillMaxWidth
import com.tencent.kuikly.compose.foundation.layout.padding
import com.tencent.kuikly.compose.ui.Modifier
import com.xiaozhi.admin.kit.actionRow
import com.xiaozhi.admin.kit.labeledField
import com.xiaozhi.admin.kit.labeledIntField
import com.xiaozhi.admin.kit.labeledSelect
import com.xiaozhi.admin.kit.labeledTextArea
import com.xiaozhi.admin.kit.switchRow

/**
 * 「人格」配置卡（`[soul]`）—— Compose 版（从 476 行的 `ConfigSoulMemoryCards.kt` 按职责拆出）。
 *
 * ## 设计依据（与迁移前一致，请不要"简化"成表单堆叠）
 *
 * 人格是"看不见的"：不把**最终提示词**和**字段来源**摊开，用户只会得到"配了没用 / 不敢改"
 * 的结论。所以这张卡有三个不可省的动作：
 * `用内置默认人格启用` / `载入预设内容到表单` / `预览最终提示词`，
 * 而合并语义（留空=用预设、填写=覆盖）**只实现在服务端**（`soul::effective`）——
 * 客户端只做两件事：把当前草稿发过去、把回答显示出来，不在这里复制一份合并规则。
 *
 * ## 字段分组
 *
 * ```
 * 启用开关 → 预设/动作行 → 预览正文（只读）
 * 身份：自称 / 自我定位 / 形象 / 年龄感 / 语气 / 称呼用户
 * 性格：性格 / 价值观 / 红线 / 口头禅
 * 背景：世界观 / 来历 / 与用户的关系起点 / 风格范例
 * 表达：回复最多句数 / 最多字数 / 说人话 / 允许 emoji / 当前情境 / 设备提示
 * ```
 */
@Composable
fun SoulConfigCard(form: ConfigFormState) {
    val sm = form.sm
    val ctx = LocalAdminPagerOrThrow()
    LabeledCard(title = "人格档案") {
        switchRow(
            label = "启用人格档案",
            checked = sm.soulEnabled == "true",
            onToggle = { form.toggleStringBool(sm::soulEnabled) },
            help = "开启后，下面填写的内容会在调用大模型前拼成系统提示词；留空的字段由「预设」补，" +
                "填了的字段逐字段覆盖预设。关闭则完全使用 LLM 卡里的系统提示词。",
        )

        if (sm.soulEnabled == "true") {
            actionRow {
                Button(
                    text = "用内置默认人格启用",
                    onClick = {
                        sm.soulPreset = "xiaozhi"
                        sm.soulEnabled = "true"
                        form.markDirty()
                    },
                    type = ButtonType.OUTLINE,
                )
            }
            labeledSelect(
                label = "内置预设",
                selectedId = sm.soulPreset,
                options = sm.soulPresetOptions,
                onSelect = {
                    sm.soulPreset = it
                    form.markDirty()
                },
                help = "选一个预设作为基线：留空的字段由它补，填了的字段覆盖它。" +
                    "「不使用预设」= 完全自定义，此时 name / address_user 必须自己填。",
            )
            actionRow {
                Button(
                    text = "载入预设内容到表单",
                    onClick = { sm.loadDefaultSoul(ctx, form.serverBase) { form.markDirty() } },
                    type = ButtonType.OUTLINE,
                )
                Button(
                    text = "清空人格字段",
                    onClick = { sm.clearSoulProfile { form.markDirty() } },
                    type = ButtonType.OUTLINE,
                )
                Button(
                    text = "预览最终提示词",
                    onClick = {
                        sm.syncSoulPresets(ctx, form.serverBase)
                        sm.previewSoul(ctx, form.serverBase)
                    },
                    type = ButtonType.OUTLINE,
                )
            }
            if (sm.soulPresetMsg.isNotEmpty()) HintLine(sm.soulPresetMsg)

            // 预览正文（只读）：服务端展开的最终提示词，长文本给足行数
            if (sm.soulPreviewText.isNotEmpty()) {
                Text(
                    text = "最终提示词（预览，只读）",
                    modifier = Modifier.padding(top = Spacing.sm),
                    style = Theme.typography.label,
                    color = Theme.colors.foreground,
                )
                Text(
                    text = sm.soulPreviewText,
                    modifier = Modifier.fillMaxWidth().padding(top = Spacing.xs),
                    style = Theme.typography.caption,
                    color = Theme.colors.mutedForeground,
                )
            }

            labeledField(
                label = "自称",
                value = sm.soulName,
                onValueChange = {
                    sm.soulName = it
                    form.markDirty()
                },
            )
            labeledField(
                label = "自我定位",
                value = sm.soulSelfIntro,
                onValueChange = {
                    sm.soulSelfIntro = it
                    form.markDirty()
                },
            )
            labeledField(
                label = "形象 / 物种",
                value = sm.soulForm,
                onValueChange = {
                    sm.soulForm = it
                    form.markDirty()
                },
            )
            labeledField(
                label = "年龄感",
                value = sm.soulAgeFeel,
                onValueChange = {
                    sm.soulAgeFeel = it
                    form.markDirty()
                },
            )
            labeledField(
                label = "语气",
                value = sm.soulTone,
                onValueChange = {
                    sm.soulTone = it
                    form.markDirty()
                },
            )
            labeledField(
                label = "称呼用户",
                value = sm.soulAddressUser,
                onValueChange = {
                    sm.soulAddressUser = it
                    form.markDirty()
                },
            )
            labeledTextArea(
                label = "性格",
                value = sm.soulTraits,
                onValueChange = {
                    sm.soulTraits = it
                    form.markDirty()
                },
                placeholder = "每行一条：标签：遇到 X 会怎么做",
                minLines = 3,
            )
            labeledTextArea(
                label = "价值观",
                value = sm.soulValues,
                onValueChange = {
                    sm.soulValues = it
                    form.markDirty()
                },
                minLines = 2,
            )
            labeledTextArea(
                label = "红线 / 拒绝清单",
                value = sm.soulBoundaries,
                onValueChange = {
                    sm.soulBoundaries = it
                    form.markDirty()
                },
                minLines = 2,
            )
            labeledTextArea(
                label = "口头禅",
                value = sm.soulCatchphrases,
                onValueChange = {
                    sm.soulCatchphrases = it
                    form.markDirty()
                },
                minLines = 2,
            )
            labeledTextArea(
                label = "世界观",
                value = sm.soulWorldview,
                onValueChange = {
                    sm.soulWorldview = it
                    form.markDirty()
                },
                minLines = 2,
            )
            labeledTextArea(
                label = "来历",
                value = sm.soulBackstory,
                onValueChange = {
                    sm.soulBackstory = it
                    form.markDirty()
                },
                minLines = 2,
            )
            labeledTextArea(
                label = "与用户的关系起点",
                value = sm.soulRelationshipOrigin,
                onValueChange = {
                    sm.soulRelationshipOrigin = it
                    form.markDirty()
                },
                minLines = 2,
            )
            labeledTextArea(
                label = "风格范例",
                value = sm.soulExamples,
                onValueChange = {
                    sm.soulExamples = it
                    form.markDirty()
                },
                placeholder = "每行一组 Q/A",
                minLines = 3,
            )
            labeledIntField(
                label = "回复最多句数",
                value = sm.soulMaxSentences,
                onValueChange = {
                    sm.soulMaxSentences = it
                    form.markDirty()
                },
            )
            labeledIntField(
                label = "回复最多字数",
                value = sm.soulMaxChars,
                onValueChange = {
                    sm.soulMaxChars = it
                    form.markDirty()
                },
            )
            switchRow(
                label = "说人话（短句、可省略主语）",
                checked = sm.soulColloquial == "true",
                onToggle = { form.toggleStringBool(sm::soulColloquial) },
            )
            // ⚠️ 语音默认关：TTS 会把 emoji 念出来
            switchRow(
                label = "允许 emoji",
                checked = sm.soulEmoji == "true",
                onToggle = { form.toggleStringBool(sm::soulEmoji) },
                help = "语音默认关闭：TTS 会把 emoji 念出来。",
            )
            labeledField(
                label = "当前情境",
                value = sm.soulScenario,
                onValueChange = {
                    sm.soulScenario = it
                    form.markDirty()
                },
            )
            labeledField(
                label = "设备提示",
                value = sm.soulDeviceHint,
                onValueChange = {
                    sm.soulDeviceHint = it
                    form.markDirty()
                },
            )
        } else {
            HintLine("未启用：使用「LLM · 对话生成」里的系统提示词（行为与加人格之前一致）。")
        }
    }
}
