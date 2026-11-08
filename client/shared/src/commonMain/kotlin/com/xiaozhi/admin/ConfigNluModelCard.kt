package com.xiaozhi.admin

import androidx.compose.runtime.Composable
import com.gearui.components.button.Button
import com.gearui.components.button.ButtonType
import com.gearui.foundation.primitives.Text
import com.gearui.theme.Theme
import com.tencent.kuikly.compose.ui.unit.dp
import com.xiaozhi.admin.contracts.IconToken
import com.xiaozhi.admin.icons.GearIcon
import com.xiaozhi.admin.kit.HintLine
import com.xiaozhi.admin.kit.fieldLabelRow
import com.xiaozhi.admin.kit.labeledDecimalField
import com.xiaozhi.admin.kit.labeledField
import com.xiaozhi.admin.kit.labeledTextArea
import com.xiaozhi.admin.kit.switchRow

/** Native Kuikly/GearUI controls, shared by Web and macOS. No HTML or global test toggle. */
@Composable
internal fun NluBuildModelSection(form: ConfigFormState, nlu: NluSectionState) {
    val ctx = LocalAdminPagerOrThrow()
    val state = nlu.model
    val view = state.view
    val unsaved = form.dirty || form.workflow.dirty
    NluInsetPanel {
        Text("模型：保存 → 构建 → 独立测试 → 明确启用", style = Theme.typography.bodyMedium)
        HintLine("构建只读取服务器已保存配置，不读取当前输入草稿；手动构建成功只生成候选，不自动启用。候选测试不改变全局开关，也不执行设备动作。")
        // 首启自举会改配置版本：管理台手里的旧 revision 下次保存必然 409，
        // 不说清楚会被当成"保存坏了"（自举采用只写 4 个 bert 字段，其余一个字节没动）。
        HintLine("服务首次启动会在约 30 秒后自动构建模型并可能自动启用，这会更新配置版本；若草稿保存报 409（配置已被其他窗口修改），请重新加载配置后再改。")
        if (nlu.bert.model.isEmpty()) {
            HintLine("当前未配置语义模型路径：上述自动构建会生效。不希望自动训练/启用请设环境变量 NLU_BOOTSTRAP=off（=build-only 只训练不启用）。")
        }
        if (unsaved) HintLine("有未保存改动。请先保存，再构建新模型；当前草稿不会进入训练。")
        fieldLabelRow(
            label = "第一步：保存技能配置",
            trailing = {
                Button(
                    text = if (form.saving) "保存中…" else "保存配置",
                    onClick = { form.save(ctx, form.serverBase) },
                    disabled = !unsaved || form.saving || form.editDeviceId.isNotEmpty(),
                    loading = form.saving,
                    type = ButtonType.OUTLINE,
                    iconWidget = { color -> GearIcon(IconToken.Save, size = 16.dp, tint = color) },
                )
            },
        )
        // 底座不再可选：服务端只有唯一底座，构建请求里也没有 base_model 字段。
        HintLine("当前固定使用预训练底座 $NLU_BASE_MODEL_LABEL：体积与推理延迟更低，不代表更准。")
        fieldLabelRow(
            label = "第二步：构建候选模型",
            help = "使用保存态配置版本微调中文分类模型并导出 ONNX。训练可能持续数分钟，旧模型不受构建影响；失败时保留原配置。",
            trailing = {
                Button(
                    text = if (view.busy) "训练状态读取中…" else "构建新模型",
                    onClick = { state.start(ctx, form) },
                    loading = view.busy,
                    disabled = unsaved || form.saving || form.conflict || form.revision.isEmpty() || view.busy || view.testBusy || form.editDeviceId.isNotEmpty(),
                    iconWidget = { color -> GearIcon(IconToken.Model, size = 16.dp, tint = color) },
                )
            },
        )
        fieldLabelRow(
            label = "训练状态",
            trailing = {
                Button(text = "刷新状态", type = ButtonType.OUTLINE,
                    onClick = { state.refresh(ctx, form) }, disabled = view.busy || view.testBusy,
                    iconWidget = { color -> GearIcon(IconToken.Refresh, size = 16.dp, tint = color) })
            },
        )
        if (view.message.isNotEmpty()) HintLine(view.message)
        if (view.log.isNotEmpty()) Text(view.log.takeLast(1600), style = Theme.typography.caption, color = Theme.colors.mutedForeground)
        val candidate = view.candidate
        if (candidate != null) {
            HintLine("候选版本：${candidate.buildId}；训练配置版本：${candidate.sourceRevision}")
            // 历史 manifest 没记底座时如实说明，不猜成当前底座。
            HintLine(
                if (candidate.baseModel.isEmpty()) "训练底座：未记录（历史构建）"
                else "训练底座：${candidate.baseModel}"
            )
            HintLine("模型指纹：${candidate.sha256}")
            if (candidate.sourceRevision != form.revision) {
                HintLine("保存态配置已经变化。此候选仍可独立测试，但不能直接启用；请按最新配置重新构建。")
            }
            labeledTextArea(
                label = "第三步：新技能测试文本",
                value = view.testText, onValueChange = { state.editText(it) },
                placeholder = "输入没有照抄训练模板的口语表达", minLines = 2,
                help = "直接推理当前候选，不经过规则或启发式，也不执行技能。测试正例之外，还应测试闲聊、否定和歧义句。",
            )
            labeledField(
                label = "预期技能名（拒识填 other）",
                value = view.expectedIntent, onValueChange = { state.editExpected(it) },
                placeholder = "填写上面新增的技能名",
                trailing = {
                    Button(text = if (view.testBusy) "测试中…" else "测试候选模型",
                        onClick = { state.test(ctx, form) }, loading = view.testBusy,
                        disabled = view.testBusy || view.busy || view.testText.isBlank() || view.expectedIntent.isBlank())
                },
            )
            if (view.testMessage.isNotEmpty()) HintLine(view.testMessage)
            if (view.verified) HintLine("这一个正例达到阈值且与预期一致；不代表整体准确率、槽位抽取或设备动作已验收。")
            fieldLabelRow(
                label = "第四步：明确启用候选",
                help = "需要当前候选至少一个正例达阈值且命中预期技能，配置仍与构建源版本相同。操作将候选路径写入当前唯一配置草稿，并通过原保存流程提交；以保存结果及服务端生效档位为准。",
                trailing = {
                    Button(text = "采用候选并保存启用",
                        onClick = { state.adopt(ctx, form) },
                        disabled = !state.canAdopt(ctx, form),
                        iconWidget = { color -> GearIcon(IconToken.Save, size = 16.dp, tint = color) })
                },
            )
        }
    }
    NluInsetPanel {
        switchRow(
            label = "配置中的语义模型开关",
            checked = nlu.bert.enabled == "true",
            onToggle = { nlu.bert = nlu.bert.copy(enabled = if (nlu.bert.enabled == "true") "false" else "true"); form.markDirty() },
            help = "这里是当前配置草稿，不是运行状态。修改必须保存；候选独立测试无需开启。",
        )
        HintLine(if (nlu.bert.enabled == "true") "草稿中已开启语义模型；是否保存、生效及加载成功请以保存反馈和服务端状态为准。" else "语义模型关闭：不能识别技能，不会通过关键词或其他方式补选技能；普通聊天仍可继续。")
        if (nlu.bert.model.isNotEmpty()) HintLine("配置模型：${nlu.bert.model}")
        if (form.showAdvanced) {
            labeledDecimalField("语义接受阈值", nlu.bert.threshold,
                { nlu.bert = nlu.bert.copy(threshold = it); form.markDirty() }, placeholder = "0.75",
                help = "候选测试使用服务器保存的阈值。不要为了显示命中而盲目降低阈值，应同时检查负例误接受。")
            labeledField("模型文件路径", nlu.bert.model, { nlu.bert = nlu.bert.copy(model = it); form.markDirty() })
            labeledField("分词器文件路径", nlu.bert.vocab, { nlu.bert = nlu.bert.copy(vocab = it); form.markDirty() }, help = "使用与模型配套的 tokenizer.json，而不是普通文本词表。")
            labeledField("标签文件路径", nlu.bert.labels, { nlu.bert = nlu.bert.copy(labels = it); form.markDirty() })
        }
    }
}
