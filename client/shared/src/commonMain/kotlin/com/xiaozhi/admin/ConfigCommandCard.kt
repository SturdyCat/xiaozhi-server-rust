package com.xiaozhi.admin

import androidx.compose.runtime.Composable
import com.xiaozhi.admin.kit.labeledField
import com.xiaozhi.admin.kit.labeledTextArea
import com.xiaozhi.admin.kit.switchRow

/** Compatibility editor for the model catalog's end_session definition, not a keyword gate. */
@Composable
fun CommandConfigCard(form: ConfigFormState) {
    val cmd = form.cmd
    LabeledCard(title = "结束会话技能") {
        val sectionNote = form.schema.sectionNote("command")
        if (sectionNote.isNotEmpty()) HintLine(sectionNote)
        switchRow(
            label = "目录包含结束会话技能",
            checked = cmd.enabled == "true",
            onToggle = { form.toggleStringBool(cmd::enabled) },
            help = "把 end_session 与下方训练例句纳入有效技能目录。仍然必须通过已训练并启用的真实模型识别；不会直接匹配关键词。",
        )
        labeledTextArea(
            label = "结束会话训练例句（每行一条）",
            value = cmd.keywords,
            onValueChange = { cmd.keywords = it; form.markDirty() },
            help = "这些句子用于模型训练，不是即时生效的关键词指令。修改并保存后需重建、测试并明确采用新模型；旧的匹配方式和字数限制已不参与识别。",
        )
        labeledField(
            label = "告别语",
            value = cmd.reply,
            onValueChange = { cmd.reply = it; form.markDirty() },
            placeholder = "好的，我先退下了。",
            help = "模型接受结束会话技能后，先说告别语，再请求设备结束聆听；设备不支持或调用失败时兼容关闭当前会话。保存配置或分类测试不表示设备已结束聆听。",
        )
        HintLine("结束会话与其他技能共用模型识别，没有关键词或长度判断的旁路。")
    }
}
