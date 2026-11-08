package com.xiaozhi.admin

import androidx.compose.runtime.Composable
import com.xiaozhi.admin.kit.HintLine
import com.xiaozhi.admin.kit.LabeledCard
import com.xiaozhi.admin.kit.labeledField
import com.xiaozhi.admin.kit.labeledTextArea
import com.xiaozhi.admin.kit.switchRow

/**
 * 设备域技能 end_session（断开对话）的训练词表编辑器，不是关键词闸门。
 * 词表是它的训练样本来源：保存态词表经统一目录注入 end_session 技能（domain=device）。
 */
@Composable
fun CommandConfigCard(form: ConfigFormState) {
    val cmd = form.cmd
    LabeledCard(title = "断开对话技能（end_session · 设备域）") {
        val sectionNote = form.schema.sectionNote("command")
        if (sectionNote.isNotEmpty()) HintLine(sectionNote)
        switchRow(
            label = "目录包含断开对话技能",
            checked = cmd.enabled == "true",
            onToggle = { form.toggleStringBool(cmd::enabled) },
            help = "把 end_session（设备域技能，作用于当前会话/设备）与下方训练例句纳入有效技能目录。仍然必须通过已训练并启用的真实模型识别；不会直接匹配关键词。",
        )
        labeledTextArea(
            label = "断开对话技能训练例句（每行一条）",
            value = cmd.keywords,
            onValueChange = { cmd.keywords = it; form.markDirty() },
            help = "这些句子是断开对话技能（end_session）的训练样本，不是即时生效的关键词指令。修改并保存后需重建、测试并明确采用新模型；旧的匹配方式和字数限制已不参与识别。",
        )
        labeledField(
            label = "告别语",
            value = cmd.reply,
            onValueChange = { cmd.reply = it; form.markDirty() },
            placeholder = "好的，我先退下了。",
            help = "模型接受断开对话技能后，先说告别语，再请求设备结束聆听；设备不支持或调用失败时兼容关闭当前会话。保存配置或分类测试不表示设备已结束聆听。",
        )
        HintLine("断开对话技能归设备域（作用于当前会话/设备），与其他技能共用模型识别，没有关键词或长度判断的旁路。")
    }
}
