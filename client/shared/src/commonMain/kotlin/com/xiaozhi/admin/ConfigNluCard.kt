package com.xiaozhi.admin

import com.tencent.kuikly.core.base.ViewContainer
import com.tencent.kuikly.core.directives.vfor
import com.tencent.kuikly.core.directives.vif
import com.tencent.kuikly.core.pager.Pager
import com.tencent.kuikly.core.views.Text
import com.tencent.kuikly.core.views.View

/**
 * 「技能」配置卡（`[nlu]`）：三级技能级联的**本地两级**（规则 + 轻量模型）。
 *
 * 级联全貌（服务端为唯一权威，见 `server/src/plugins/skill/mod.rs`）：
 * 终端 → ASR → ①规则模板命中即执行 → ②轻量模型打分达阈值即执行 → ③云 LLM
 * （LLM 若识别为技能，会以 `execute_skill` 工具调用回目录校验执行，槽位/动作白名单由
 * 服务端配置决定；否则正常聊天走 TTS）。这张卡只配置 ①② 与自定义意图/槽位；
 * 内置 `end_session` 在「指令」卡（`[command]`）配置。
 *
 * 独立文件的原因：与 `ConfigCommandCard.kt` 同思路——字段/文案自成一组，
 * 且 `ConfigCards.kt` 已超文件规模约定，不再往里加内容。
 */
fun ViewContainer<*, *>.nluConfigCard(form: ConfigFormState, ctx: Pager) {
    val nlu = form.nlu
    groupedCard("技能级联（规则 → 轻量模型 → 云 LLM）") {
        // 整段档位提示（保存后新会话生效）：来自 /api/config/schema，服务端为唯一权威
        vif({ form.schema.sectionNote("nlu").isNotEmpty() }) {
            textNote(form.schema.sectionNote("nlu"), micro = true)
        }
        switchRow("enabled（启用技能级联）", nlu.enabled == "true") {
            form.toggleStringBool(nlu::enabled)
        }
        Text {
            attr {
                fontSize(AdminType.caption)
                color(AdminColors.textTertiary)
                text("识别文本先过本地规则模板（命中即执行、不调大模型），再过本地轻量模型打分；两级都未命中才交给大模型。大模型若把话识别成下面的技能，也会回到这份白名单校验后执行——大模型不能凭空发明技能或动作。")
            }
        }
        vif({ nlu.enabled == "true" }) {
            labeledDecimalField(
                "model_threshold（轻量模型执行阈值，0~1）",
                { nlu.modelThreshold },
                { nlu.modelThreshold = it; form.markDirty() },
                "0.9",
            )
            labeledDecimalField(
                "model_margin（领先次高分的最小差值，0~1）",
                { nlu.modelMargin },
                { nlu.modelMargin = it; form.markDirty() },
                "0.1",
            )
            vif({ (nlu.modelThreshold.trim().toDoubleOrNull() ?: 0.9) >= 1.0 }) {
                Text {
                    attr {
                        fontSize(AdminType.caption)
                        color(AdminColors.warningTintText)
                        text("⚠️ threshold = 1.0 表示关闭轻量模型级：只有规则模板完全命中的技能会本地执行，其余全部交给大模型。")
                    }
                }
            }
            Text {
                attr {
                    fontSize(AdminType.caption)
                    color(AdminColors.textTertiary)
                    text("封闭槽位容忍「包含唯一值」的口语化说法（「把卧室灯打开」可命中「把{room}的灯{action}」）；但含否定词（别/不要）或多个候选值（开还是关）时不本地执行，交大模型判断。")
                }
            }
            nluIntentList(form, nlu)
            nluSlotList(form, nlu)
            nluBuildModelSection(form, nlu, ctx)
        }
        vif({ nlu.enabled != "true" }) {
            Text {
                attr {
                    fontSize(AdminType.caption)
                    color(AdminColors.textTertiary)
                    text("未启用：识别结果照常交给大模型（内置指令闸门 [command] 不受影响，见「指令」卡）。")
                }
            }
        }
    }
}

// ===================== 生成 NLU 模型（本地轻量模型三件套） =====================

/**
 * 「生成 NLU 模型」入口：POST /api/nlu/model/build 由服务端在宿主 docker 里
 * 基于当前技能（内置提醒技能 + 自定义 intents）训练并导出 ONNX int8 三件套
 * （model.onnx / tokenizer.json / labels.txt），产物供 `[nlu]` bert_* 配置指向。
 */
private fun ViewContainer<*, *>.nluBuildModelSection(form: ConfigFormState, nlu: NluSectionState, ctx: Pager) {
    fieldLabel("轻量模型（TinyBERT 语义打分）")
    Text {
        attr {
            fontSize(AdminType.caption)
            color(AdminColors.textTertiary)
            text("可选增强：用当前技能定义自动生成一个本地语义分类模型，替换「轻量模型」级的启发式打分（对口语化说法识别更稳，零外网推理）。训练在服务端宿主 docker 里进行，约数分钟。")
        }
    }
    primaryButton("生成 NLU 模型", loading = { nlu.buildBusy }, loadingText = "训练中…") {
        nlu.startBuildModel(ctx, form.serverBase)
    }
    vif({ nlu.buildMsg.isNotEmpty() }) {
        Text {
            attr {
                fontSize(AdminType.caption)
                color(if (nlu.buildPhase == "failed") AdminColors.warningTintText else AdminColors.textTertiary)
                text(nlu.buildMsg)
            }
        }
    }
    vif({ nlu.buildLog.isNotEmpty() && (nlu.buildPhase == "running" || nlu.buildPhase == "failed") }) {
        Text {
            attr {
                fontSize(AdminType.caption)
                color(AdminColors.textTertiary)
                text(nlu.buildLog.takeLast(1200))
            }
        }
    }
}

// ===================== 自定义意图（intents） =====================

private fun ViewContainer<*, *>.nluIntentList(form: ConfigFormState, nlu: NluSectionState) {
    fieldLabel("自定义技能 intents（${nlu.intents.size} 条）")
    vif({ nlu.intents.size == 0 }) {
        Text {
            attr {
                fontSize(AdminType.caption)
                color(AdminColors.textTertiary)
                text("还没有自定义技能。点下方「添加意图」，用模板描述一句话指令（如「把{room}的灯{action}」），命名槽位后即可本地执行。")
            }
        }
    }
    vfor({ nlu.intents }) { d ->
        nluIntentBlock(form, nlu, d)
    }
    secondaryButton("＋ 添加意图") {
        nlu.addIntent()
        form.markDirty()
    }
}

/** 一条意图的编辑块：名称 / 模板 / 动作 / 应答语 + 删除。 */
private fun ViewContainer<*, *>.nluIntentBlock(form: ConfigFormState, nlu: NluSectionState, d: IntentDraft) {
    View {
        attr {
            marginTop(AdminSpace.sm)
            padding(AdminSpace.sm)
            backgroundColor(AdminColors.insetBg)
            borderRadius(AdminShape.radiusSm)
        }
        View {
            attr {
                flexDirectionRow()
                alignItemsCenter()
            }
            Text {
                attr {
                    flex(1f)
                    fontSize(AdminType.body)
                    fontWeightMedium()
                    color(AdminColors.textSecondary)
                    text(d.name.ifEmpty { "(未命名技能)" })
                }
            }
            secondaryButton("删除") {
                nlu.removeIntent(d)
                form.markDirty()
            }
        }
        labeledField(
            "name（技能名，须唯一）",
            { d.name },
            { d.name = it; form.markDirty() },
            "light_control",
        )
        labeledTextArea(
            "templates（每行一个模板，{槽位名} 为占位）",
            { d.templates },
            { d.templates = it; form.markDirty() },
            "把{room}的灯{action}",
            height = 88f,
        )
        dropdownField(
            label = "action.kind（命中后做什么）",
            currentLabel = {
                nlu.actionOptions.firstOrNull { it.first == d.actionKind }?.second ?: d.actionKind
            },
            options = { nlu.actionOptions },
            selectedId = { d.actionKind },
            isOpen = { d.actionMenuOpen },
            onToggle = { d.actionMenuOpen = !d.actionMenuOpen },
            onSelect = {
                d.actionKind = it
                d.actionMenuOpen = false
                form.markDirty()
            },
        )
        vif({ d.actionKind == "mcp" }) {
            labeledField(
                "mcp 工具名（设备声明的工具，精确匹配）",
                { d.mcpTool },
                { d.mcpTool = it; form.markDirty() },
                "set_volume",
            )
            Text {
                attr {
                    fontSize(AdminType.caption)
                    color(AdminColors.textTertiary)
                    text("命中后先说 reply，再把槽位作为参数派发到该设备工具；设备清单里没有同名工具则零调用跳过（清单见「MCP 工具」页）。")
                }
            }
        }
        labeledField(
            "reply（命中后的应答语）",
            { d.reply },
            { d.reply = it; form.markDirty() },
            "好的，已为你打开灯。",
        )
    }
}

// ===================== 槽位定义（slots） =====================

private fun ViewContainer<*, *>.nluSlotList(form: ConfigFormState, nlu: NluSectionState) {
    fieldLabel("槽位定义 slots（${nlu.slots.size} 条）")
    vif({ nlu.slots.size == 0 }) {
        Text {
            attr {
                fontSize(AdminType.caption)
                color(AdminColors.textTertiary)
                text("模板里用到的 {槽位} 在这里命名并给封闭值表（用于校验与抽取）；不给值表 = 开放槽位（任意短语）。")
            }
        }
    }
    vfor({ nlu.slots }) { s ->
        nluSlotBlock(form, nlu, s)
    }
    secondaryButton("＋ 添加槽位") {
        nlu.addSlot()
        form.markDirty()
    }
}

/** 一条槽位的编辑块：名称 / 值表 / 可缺省 + 删除。 */
private fun ViewContainer<*, *>.nluSlotBlock(form: ConfigFormState, nlu: NluSectionState, s: SlotDraft) {
    View {
        attr {
            marginTop(AdminSpace.sm)
            padding(AdminSpace.sm)
            backgroundColor(AdminColors.insetBg)
            borderRadius(AdminShape.radiusSm)
        }
        View {
            attr {
                flexDirectionRow()
                alignItemsCenter()
            }
            Text {
                attr {
                    flex(1f)
                    fontSize(AdminType.body)
                    fontWeightMedium()
                    color(AdminColors.textSecondary)
                    text("{${s.name.ifEmpty { "未命名槽位" }}}")
                }
            }
            secondaryButton("删除") {
                nlu.removeSlot(s)
                form.markDirty()
            }
        }
        labeledField(
            "name（槽位名，对应模板占位）",
            { s.name },
            { s.name = it; form.markDirty() },
            "room",
        )
        labeledTextArea(
            "values（封闭值表，每行一个；留空 = 开放槽位）",
            { s.values },
            { s.values = it; form.markDirty() },
            "卧室\n客厅\n书房",
            height = 80f,
        )
        switchRow("optional（模板中可缺省）", s.optional == "true") {
            form.toggleStringBool(s::optional)
        }
        Text {
            attr {
                fontSize(AdminType.caption)
                color(AdminColors.textTertiary)
                text("值表里的词会做归一化比对（忽略标点差异），命中后按配置原文回填；可选槽位缺省时以空值执行。")
            }
        }
    }
}
