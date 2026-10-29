package com.xiaozhi.admin

import androidx.compose.runtime.Composable
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
import com.xiaozhi.admin.kit.fieldLabelRow
import com.xiaozhi.admin.kit.labeledDecimalField
import com.xiaozhi.admin.kit.labeledField
import com.xiaozhi.admin.kit.labeledSelect
import com.xiaozhi.admin.kit.labeledTextArea
import com.xiaozhi.admin.kit.switchRow

/**
 * 「技能」配置卡（`[nlu]`）：三级技能级联的**本地两级**（规则 + 轻量模型）—— Compose 版。
 *
 * 级联全貌（服务端为唯一权威，见 `server/src/plugins/skill/mod.rs`）：
 * 终端 → ASR → ①规则模板命中即执行 → ②轻量模型打分达阈值即执行 → ③云 LLM。
 * 这张卡只配置 ①② 与自定义意图/槽位；内置 `end_session` 在「指令」卡（`[command]`）配置。
 *
 * ## 结构（5 个子块，与旧版一一对应）
 *
 * ```
 * 启用技能级联（开关）
 * ├ 执行阈值 / 领先余量
 * ├ 轻量模型（生成 NLU 模型：POST /api/nlu/model/build）
 * ├ 自定义技能（intents：名称 / 模板 / 动作 / MCP 工具名 / 应答语）
 * └ 槽位定义（slots：名称 / 封闭值表 / 允许缺省）
 * ```
 *
 * 旧版的列表指令循环改成 `for (d in nlu.intents)` —— `nlu.intents` 是快照列表
 * (`SnapshotStateList`)，本身可观察，增删一项即重组，不需要桥接层。
 */
@Composable
fun NluConfigCard(form: ConfigFormState) {
    val nlu = form.nlu
    LabeledCard(title = "NLU · 意图与技能") {
        val sectionNote = form.schema.sectionNote("nlu")
        if (sectionNote.isNotEmpty()) HintLine(sectionNote)

        switchRow(
            label = "启用技能级联",
            checked = nlu.enabled == "true",
            onToggle = { form.toggleStringBool(nlu::enabled) },
            help = "识别文本先过本地规则模板（命中即执行、不调大模型），再过本地轻量模型打分；" +
                "两级都未命中才交给大模型。大模型若把话识别成下面的技能，也会回到这份白名单校验后执行——" +
                "大模型不能凭空发明技能或动作。",
        )

        if (nlu.enabled == "true") {
            labeledDecimalField(
                label = "执行阈值",
                value = nlu.modelThreshold,
                onValueChange = {
                    nlu.modelThreshold = it
                    form.markDirty()
                },
                placeholder = "0.9",
                help = "轻量模型打分达到该值即本地执行，取值 0~1（默认 0.9）。调到 1.0 等于关闭这一级：" +
                    "只有规则模板完全命中的技能会本地执行，其余全部交给大模型（更省钱但更慢）。",
            )
            if ((nlu.modelThreshold.trim().toDoubleOrNull() ?: 0.9) >= 1.0) {
                HintLine("⚠️ 阈值 = 1.0 表示关闭轻量模型级：只有规则模板完全命中的技能会本地执行，其余全部交给大模型。")
            }
            labeledDecimalField(
                label = "领先余量",
                value = nlu.modelMargin,
                onValueChange = {
                    nlu.modelMargin = it
                    form.markDirty()
                },
                placeholder = "0.1",
                help = "第一名必须比第二名高出该差值（0~1）才算「有把握」；调大更保守、误触发更少，" +
                    "但更多请求会落到大模型。",
            )
            NluBuildModelSection(form, nlu)
            NluIntentList(form, nlu)
            NluSlotList(form, nlu)
        } else {
            HintLine("未启用：识别结果照常交给大模型（内置指令闸门不受影响，见「指令」卡）。")
        }
    }
}

/**
 * 「生成 NLU 模型」入口：`POST /api/nlu/model/build` 由服务端在宿主 docker 里
 * 基于当前技能训练并导出 ONNX int8 三件套（model.onnx / tokenizer.json / labels.txt）。
 */
@Composable
private fun NluBuildModelSection(form: ConfigFormState, nlu: NluSectionState) {
    val ctx = LocalAdminPagerOrThrow()
    fieldLabelRow(
        label = "轻量模型（TinyBERT 语义打分）",
        help = "可选增强：用当前技能定义自动生成一个本地语义分类模型，替换「轻量模型」级的启发式打分" +
            "（对口语化说法识别更稳，零外网推理）。训练在服务端宿主 docker 里进行，约数分钟；" +
            "产物为 model.onnx / tokenizer.json / labels.txt 三件套。",
        trailing = {
            Button(
                text = if (nlu.buildBusy) "训练中…" else "生成 NLU 模型",
                onClick = { nlu.startBuildModel(ctx, form.serverBase) },
                loading = nlu.buildBusy,
            )
        },
    )
    if (nlu.buildMsg.isNotEmpty()) {
        Text(
            text = nlu.buildMsg,
            modifier = Modifier.padding(top = 4.dp),
            style = Theme.typography.caption,
            color = if (nlu.buildPhase == "failed") Theme.colors.warning else Theme.colors.mutedForeground,
        )
    }
    if (nlu.buildLog.isNotEmpty() && (nlu.buildPhase == "running" || nlu.buildPhase == "failed")) {
        Text(
            text = nlu.buildLog.takeLast(1200),
            modifier = Modifier.padding(top = Spacing.xs),
            style = Theme.typography.caption,
            color = Theme.colors.mutedForeground,
        )
    }
}

/** 自定义技能列表：标题行带「＋ 添加意图」，逐条渲染 [NluIntentBlock]。 */
@Composable
private fun NluIntentList(form: ConfigFormState, nlu: NluSectionState) {
    fieldLabelRow(
        label = "自定义技能（${nlu.intents.size} 条）",
        help = "用「一句话模板 + 动作」描述本地技能，例如模板「把{room}的灯{action}」配动作「派发设备工具」。" +
            "匹配成功就不再调用大模型，直接执行并回一句应答语。",
        trailing = {
            Button(
                text = "＋ 添加意图",
                onClick = {
                    nlu.addIntent()
                    form.markDirty()
                },
                type = ButtonType.OUTLINE,
            )
        },
    )
    if (nlu.intents.isEmpty()) {
        HintLine("还没有自定义技能。点右侧「添加意图」，用模板描述一句话指令（如「把{room}的灯{action}」），命名槽位后即可本地执行。")
    }
    for (draft in nlu.intents) NluIntentBlock(form, nlu, draft)
}

/** 一条意图的编辑块：名称 / 模板 / 动作 /（MCP 工具名）/ 应答语 + 删除。 */
@Composable
private fun NluIntentBlock(form: ConfigFormState, nlu: NluSectionState, draft: IntentDraft) {
    InsetPanel {
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
            help = "技能唯一标识，仅用于本配置内部引用（可保留英文下划线写法）；重名会让后一条覆盖前一条。",
        )
        labeledTextArea(
            label = "触发模板（每行一条）",
            value = draft.templates,
            onValueChange = {
                draft.templates = it
                form.markDirty()
            },
            placeholder = "把{room}的灯{action}",
            help = "每行一个句式，{槽位名} 是可变部分（需在下方「槽位定义」里声明）。" +
                "封闭槽位容忍「包含唯一值」的口语化说法（「把卧室灯打开」也能命中）。",
            minLines = 3,
        )
        labeledSelect(
            label = "命中后动作",
            selectedId = draft.actionKind,
            options = nlu.actionOptions,
            onSelect = {
                draft.actionKind = it
                form.markDirty()
            },
            help = "只应答 = 说一句应答语后结束；结束聆听 = 说应答语后让设备退出（等同指令闸门）；" +
                "派发设备工具 = 把槽位作为参数调用设备声明的 MCP 工具。",
        )
        if (draft.actionKind == "mcp") {
            labeledField(
                label = "MCP 工具名",
                value = draft.mcpTool,
                onValueChange = {
                    draft.mcpTool = it
                    form.markDirty()
                },
                placeholder = "set_volume",
                help = "设备在 MCP 清单里声明的工具名，需精确匹配（清单见「MCP 工具」页）。" +
                    "命中后先说应答语，再把槽位作为参数派发；设备没有同名工具则跳过、不报错。",
            )
        }
        labeledField(
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

/** 槽位定义列表：标题行带「＋ 添加槽位」，逐条渲染 [NluSlotBlock]。 */
@Composable
private fun NluSlotList(form: ConfigFormState, nlu: NluSectionState) {
    fieldLabelRow(
        label = "槽位定义（${nlu.slots.size} 条）",
        help = "模板里用到的 {槽位} 在这里命名并给出封闭值表（用于校验与抽取）；不给值表 = 开放槽位，" +
            "任意短语都接受。值表里的词会做归一化比对（忽略标点差异），命中后按配置原文回填。" +
            "含否定词（别/不要）或出现多个候选值时不会本地执行，交大模型判断。",
        trailing = {
            Button(
                text = "＋ 添加槽位",
                onClick = {
                    nlu.addSlot()
                    form.markDirty()
                },
                type = ButtonType.OUTLINE,
            )
        },
    )
    if (nlu.slots.isEmpty()) {
        HintLine("模板里用到的 {槽位} 在这里命名并给封闭值表；不给值表 = 开放槽位（任意短语）。")
    }
    for (slot in nlu.slots) NluSlotBlock(form, nlu, slot)
}

/** 一条槽位的编辑块：名称 / 封闭值表 / 允许缺省 + 删除。 */
@Composable
private fun NluSlotBlock(form: ConfigFormState, nlu: NluSectionState, slot: SlotDraft) {
    InsetPanel {
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
private fun InsetPanel(content: @Composable () -> Unit) {
    Card(
        modifier = Modifier.fillMaxWidth().padding(vertical = Spacing.xs),
        containerColor = Theme.colors.muted,
    ) {
        Column(modifier = Modifier.fillMaxWidth()) { content() }
    }
}
