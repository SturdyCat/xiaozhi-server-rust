package com.xiaozhi.admin

import androidx.compose.runtime.Composable
import androidx.compose.runtime.key
import com.gearui.components.alert.Alert
import com.gearui.components.alert.AlertStatus
import com.gearui.components.button.Button
import com.gearui.components.button.ButtonType
import com.gearui.foundation.layout.Spacing
import com.gearui.foundation.primitives.Text
import com.gearui.primitives.composite.Card
import com.gearui.theme.Theme
import com.tencent.kuikly.compose.foundation.layout.Column
import com.tencent.kuikly.compose.foundation.layout.Row
import com.tencent.kuikly.compose.foundation.layout.fillMaxWidth
import com.tencent.kuikly.compose.foundation.layout.padding
import com.tencent.kuikly.compose.ui.Alignment
import com.tencent.kuikly.compose.ui.Modifier
import com.tencent.kuikly.compose.ui.unit.dp
import com.xiaozhi.admin.contracts.IconToken
import com.xiaozhi.admin.icons.GearIcon
import com.xiaozhi.admin.kit.fieldLabelRow
import com.xiaozhi.admin.kit.labeledField
import com.xiaozhi.admin.kit.labeledSelect
import com.xiaozhi.admin.kit.labeledTextArea
import com.xiaozhi.admin.kit.switchRow

/** Model-only intent recognition; skills compose registered capabilities using configuration. */
@Composable
fun NluConfigCard(form: ConfigFormState) {
    val nlu = form.nlu
    LabeledCard(title = "NLU · 意图与技能") {
        switchRow(
            label = "启用配置技能目录",
            checked = nlu.enabled == "true",
            onToggle = { form.toggleStringBool(nlu::enabled) },
            help = "控制下方配置技能是否进入有效目录；结束会话技能的目录开关独立。所有技能都必须通过已训练并启用的真实模型接受，才提取参数并检查条件、执行配置步骤。",
        )
        HintLine("没有启用可用模型就不能识别技能。训练例句只用于生成语料，不是快捷触发指令；技能条件与步骤由服务器能力白名单校验。")
        if (nlu.enabled != "true") HintLine("下方配置技能未进入有效目录；结束会话目录开关独立。可继续编辑配置，构建只使用保存态有效目录。")
        NluCatalogSection(form)
        NluIntentList(form, nlu)
        if (form.showAdvanced) NluSlotList(form, nlu)
        NluBuildModelSection(form, nlu)
    }
}

/** 自定义技能列表：标题行带「添加意图」（Plus 图标），逐条渲染 [NluIntentBlock]。 */
@Composable
private fun NluIntentList(form: ConfigFormState, nlu: NluSectionState) {
    fieldLabelRow(
        label = "自定义技能（${nlu.intents.size} 条）",
        help = "每个技能由名称、控制域、训练例句与配置动作组成。控制域按执行者分：设备=固件工具执行（含断开对话），服务器=服务端能力组合，IoT=第三方设备（tuya）。新技能可组合现有 Rust 能力或直调设备已声明工具，不需要改写服务器代码；新增或改名模型标签后仍须重新训练与验收。",
        trailing = {
            Button(
                text = "添加技能",
                onClick = {
                    nlu.addIntent()
                    form.markDirty()
                },
                type = ButtonType.OUTLINE,
                iconWidget = { color -> GearIcon(token = IconToken.Plus, size = 16.dp, tint = color) },
            )
        },
    )
    if (nlu.intents.isEmpty()) {
        HintLine("点「添加技能」填写名称、控制域、训练例句与动作，然后保存配置。无槽位技能无需添加槽位；每类训练至少需要两条不同句子。")
    }
    // P0-4 强提示：技能名 = 训练标签，改名（相对保存态）会使旧模型标签失效。
    // 只提示不拦截：沿用"保存 → 提交训练"既有链路。
    val renamedNote = nlu.renamedSkillsNote()
    if (renamedNote.isNotEmpty()) {
        Alert(
            title = "技能改名会使旧模型标签失效",
            description = "已改名：$renamedNote。技能名即模型训练标签，保存后必须重新训练并验收新模型，旧模型无法识别新名称；取消保存可保留旧标签。",
            status = AlertStatus.WARNING,
        )
    }
    for (draft in nlu.intents) key(draft) { NluIntentBlock(form, nlu, draft) }
}

/** 一条意图的编辑块：名称 / 模板 / 动作 /（MCP 工具名）/ 应答语 + 删除。 */
@Composable
private fun NluIntentBlock(form: ConfigFormState, nlu: NluSectionState, draft: IntentDraft) {
    NluInsetPanel {
        Row(modifier = Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
            Text(
                text = draft.name.ifEmpty { "(未命名技能)" },
                modifier = Modifier.weight(1f),
                style = Theme.typography.bodyMedium,
                color = Theme.colors.mutedForeground,
            )
            Button(
                text = "删除",
                onClick = {
                    nlu.removeIntent(draft)
                    form.markDirty()
                },
                type = ButtonType.OUTLINE,
                iconWidget = { color -> GearIcon(token = IconToken.Delete, size = 16.dp, tint = color) },
            )
        }
        labeledField(
            label = "技能名",
            value = draft.name,
            onValueChange = {
                draft.name = it
                form.markDirty()
            },
            placeholder = "light_control",
            help = "技能唯一标识 = 模型训练标签（可用英文下划线）；改名会使旧模型标签失效、需重新训练。同名技能会被服务端拒绝，不能使用保留标签 other/end_session。",
        )
        labeledSelect(
            label = "控制域",
            selectedId = draft.domain,
            options = nlu.domainOptions,
            onSelect = {
                draft.domain = it
                form.markDirty()
            },
            help = "技能的一级分组 = 控制目标/执行者：设备域由设备固件工具执行（如音量、播放控制），服务器域由服务端执行（如提醒类能力组合），IoT 域由服务端执行（tuya 第三方设备，如语音开关插座）。改域只影响目录分组展示，不影响训练标签。",
        )
        labeledTextArea(
            label = "训练例句（每行一条）",
            value = draft.templates,
            onValueChange = {
                draft.templates = it
                form.markDirty()
            },
            placeholder = "把{room}的灯{action}",
            help = "每行一个不同句式，可直接写无槽位句子；{槽位名} 是可变部分，需在下方声明。" +
                "模型训练每类至少需要两条不同的展开句子；少于两条会明确失败，不会复制样本伪造评测。",
            minLines = 3,
        )
        labeledSelect(
            label = "技能动作",
            selectedId = draft.actionKind,
            options = nlu.actionOptions,
            onSelect = {
                draft.actionKind = it
                form.markDirty()
            },
            help = "能力组合 = 声明条件与系统能力步骤；已有应答、结束聆听与设备工具动作继续兼容。动作名称与工具名由配置确定，模型不能创建任意 API 或脚本。",
        )
        if (draft.actionKind == "plan") {
            labeledTextArea(
                label = "条件与步骤（完整 JSON）",
                value = draft.planJson,
                onValueChange = { draft.planJson = it; form.markDirty() },
                placeholder = "{\"conditions\":[],\"steps\":[{\"api\":\"replies.say\",\"args\":{\"text\":\"你好\"}}]}",
                minLines = 6,
                help = "完整保留条件、步骤与嵌套参数。条件支持 slot_present、slot_equals、device_kind；步骤使用读取技能清单得到的能力定义。参数支持 {{slots.名称}}、{{ctx.device_id}}、{{ctx.session_id}}；仅应答步骤的文本可引用前序任务创建、查询或取消结果，如 {{steps.0.speech}}。不可引用未来步骤、改写 API/工具名或把结果插入有副作用的参数。最多 16 条条件与 16 个步骤，最终以服务端校验为准。",
            )
            val planError = draft.validationError()
            if (planError.isNotEmpty()) HintLine(planError)
            HintLine("分类测试不会运行这些步骤。提醒任务创建、到期投递与设备实际播报是不同环节，需分别验收。")
        }
        if (draft.actionKind == "mcp") {
            labeledField(
                label = "设备工具名",
                value = draft.mcpTool,
                onValueChange = {
                    draft.mcpTool = it
                    form.markDirty()
                },
                placeholder = "self.audio_speaker.set_volume",
                help = "设备固件在 MCP 清单里声明的工具名（静态字面量），需精确匹配（清单见「MCP 工具」页）。" +
                    "槽位按工具参数名对应后派发；设备没有声明同名工具则走固定降级话术拒识、不报错崩会话。",
            )
        }
        if (draft.actionKind != "plan") labeledField(
            label = "应答语",
            value = draft.reply,
            onValueChange = {
                draft.reply = it
                form.markDirty()
            },
            placeholder = "好的，已为你打开灯。",
            help = "命中后要说的话（走 TTS 合成）。留空 = 只执行动作、不出声。",
        )
    }
}

/** 槽位定义列表：标题行带「添加槽位」（Plus 图标），逐条渲染 [NluSlotBlock]。 */
@Composable
private fun NluSlotList(form: ConfigFormState, nlu: NluSectionState) {
    fieldLabelRow(
        label = "槽位定义（${nlu.slots.size} 条）",
        help = "训练例句中的 {槽位} 在这里声明。只有模型已经接受同一技能后，才提取、校验这些参数；缺少必填参数时明确请求补充，不从分数、其他技能或大模型猜造参数。",
        trailing = {
            Button(
                text = "添加槽位",
                onClick = {
                    nlu.addSlot()
                    form.markDirty()
                },
                type = ButtonType.OUTLINE,
                iconWidget = { color -> GearIcon(token = IconToken.Plus, size = 16.dp, tint = color) },
            )
        },
    )
    if (nlu.slots.isEmpty()) {
        HintLine("模板里用到的 {槽位} 在这里命名并给封闭值表；不给值表 = 开放槽位（任意短语）。")
    }
    for (slot in nlu.slots) key(slot) { NluSlotBlock(form, nlu, slot) }
}

/** 一条槽位的编辑块：名称 / 封闭值表 / 允许缺省 + 删除。 */
@Composable
private fun NluSlotBlock(form: ConfigFormState, nlu: NluSectionState, slot: SlotDraft) {
    NluInsetPanel {
        Row(modifier = Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
            Text(
                text = "{${slot.name.ifEmpty { "未命名槽位" }}}",
                modifier = Modifier.weight(1f),
                style = Theme.typography.bodyMedium,
                color = Theme.colors.mutedForeground,
            )
            Button(
                text = "删除",
                onClick = {
                    nlu.removeSlot(slot)
                    form.markDirty()
                },
                type = ButtonType.OUTLINE,
                iconWidget = { color -> GearIcon(token = IconToken.Delete, size = 16.dp, tint = color) },
            )
        }
        labeledField(
            label = "槽位名",
            value = slot.name,
            onValueChange = {
                slot.name = it
                form.markDirty()
            },
            placeholder = "room",
            help = "必须与模板里的 {占位名} 完全一致（区分大小写）。改名后不会自动同步模板，需自行修改。",
        )
        labeledTextArea(
            label = "封闭值表（每行一个）",
            value = slot.values,
            onValueChange = {
                slot.values = it
                form.markDirty()
            },
            placeholder = "卧室\n客厅\n书房",
            help = "列出这个槽位所有合法取值，用于校验与抽取；留空 = 开放槽位（任意短语都接受，不再校验）。",
            minLines = 3,
        )
        switchRow(
            label = "允许缺省",
            checked = slot.optional == "true",
            onToggle = { form.toggleStringBool(slot::optional) },
            help = "打开后模板里可以不说这个槽位（如「开灯」省略房间），缺省时以空值执行；" +
                "关闭则该槽位必须出现。",
        )
    }
}

/**
 * 内嵌面板（旧 DSL `insetPanel` 的等价物）：一张浅色卡片，用来把"一条意图/一个槽位"
 * 在视觉上收成一块，避免多个编辑块混成一长串字段。
 */
@Composable
internal fun NluInsetPanel(content: @Composable () -> Unit) {
    Card(
        modifier = Modifier.fillMaxWidth().padding(vertical = Spacing.xs),
        containerColor = Theme.colors.muted,
    ) {
        Column(modifier = Modifier.fillMaxWidth()) { content() }
    }
}
