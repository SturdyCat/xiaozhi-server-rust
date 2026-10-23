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
    groupedCard("NLU · 意图与技能") {
        // 整段档位提示（保存后新会话生效）：来自 /api/config/schema，服务端为唯一权威
        vif({ form.schema.sectionNote("nlu").isNotEmpty() }) {
            textNote(form.schema.sectionNote("nlu"), micro = true)
        }
        switchRow(
            "启用技能级联", nlu.enabled == "true",
            help = "识别文本先过本地规则模板（命中即执行、不调大模型），再过本地轻量模型打分；" +
                "两级都未命中才交给大模型。大模型若把话识别成下面的技能，也会回到这份白名单校验后执行——" +
                "大模型不能凭空发明技能或动作。",
        ) {
            form.toggleStringBool(nlu::enabled)
        }
        vif({ nlu.enabled == "true" }) {
            labeledDecimalField(
                "执行阈值",
                { nlu.modelThreshold },
                { nlu.modelThreshold = it; form.markDirty() },
                "0.9",
                help = "轻量模型打分达到该值即本地执行，取值 0~1（默认 0.9）。调到 1.0 等于关闭这一级：" +
                    "只有规则模板完全命中的技能会本地执行，其余全部交给大模型（更省钱但更慢）。",
            )
            vif({ (nlu.modelThreshold.trim().toDoubleOrNull() ?: 0.9) >= 1.0 }) {
                textNote(
                    "⚠️ 阈值 = 1.0 表示关闭轻量模型级：只有规则模板完全命中的技能会本地执行，其余全部交给大模型。",
                    tone = "warn",
                    micro = true,
                )
            }
            labeledDecimalField(
                "领先余量",
                { nlu.modelMargin },
                { nlu.modelMargin = it; form.markDirty() },
                "0.1",
                help = "第一名必须比第二名高出该差值（0~1）才算「有把握」；调大更保守、误触发更少，" +
                    "但更多请求会落到大模型。",
            )
            nluBuildModelSection(form, nlu, ctx)
            nluIntentList(form, nlu)
            nluSlotList(form, nlu)
        }
        vif({ nlu.enabled != "true" }) {
            textNote("未启用：识别结果照常交给大模型（内置指令闸门不受影响，见「指令」卡）。", micro = true)
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
    fieldLabel(
        "轻量模型（TinyBERT 语义打分）",
        help = "可选增强：用当前技能定义自动生成一个本地语义分类模型，替换「轻量模型」级的启发式打分" +
            "（对口语化说法识别更稳，零外网推理）。训练在服务端宿主 docker 里进行，约数分钟；" +
            "产物为 model.onnx / tokenizer.json / labels.txt 三件套。",
        trailing = {
            primaryButton("生成 NLU 模型", loading = { nlu.buildBusy }, loadingText = "训练中…") {
                nlu.startBuildModel(ctx, form.serverBase)
            }
        },
    )
    vif({ nlu.buildMsg.isNotEmpty() }) {
        textNote(nlu.buildMsg, tone = if (nlu.buildPhase == "failed") "warn" else "tertiary", micro = true)
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
    fieldLabel(
        "自定义技能（${nlu.intents.size} 条）",
        help = "用「一句话模板 + 动作」描述本地技能，例如模板「把{room}的灯{action}」配动作「派发设备工具」。" +
            "匹配成功就不再调用大模型，直接执行并回一句应答语。",
        trailing = {
            secondaryButton("＋ 添加意图") {
                nlu.addIntent()
                form.markDirty()
            }
        },
    )
    vif({ nlu.intents.size == 0 }) {
        textNote("还没有自定义技能。点右侧「添加意图」，用模板描述一句话指令（如「把{room}的灯{action}」），命名槽位后即可本地执行。", micro = true)
    }
    vfor({ nlu.intents }) { d ->
        nluIntentBlock(form, nlu, d)
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
            "技能名",
            { d.name },
            { d.name = it; form.markDirty() },
            "light_control",
            help = "技能唯一标识，仅用于本配置内部引用（可保留英文下划线写法）；重名会让后一条覆盖前一条。",
        )
        labeledTextArea(
            "触发模板（每行一条）",
            { d.templates },
            { d.templates = it; form.markDirty() },
            "把{room}的灯{action}",
            height = 88f,
            help = "每行一个句式，{槽位名} 是可变部分（需在下方「槽位定义」里声明）。" +
                "封闭槽位容忍「包含唯一值」的口语化说法（「把卧室灯打开」也能命中）。",
        )
        dropdownField(
            label = "命中后动作",
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
            help = "只应答 = 说一句应答语后结束；结束聆听 = 说应答语后让设备退出（等同指令闸门）；" +
                "派发设备工具 = 把槽位作为参数调用设备声明的 MCP 工具。",
        )
        vif({ d.actionKind == "mcp" }) {
            labeledField(
                "MCP 工具名",
                { d.mcpTool },
                { d.mcpTool = it; form.markDirty() },
                "set_volume",
                help = "设备在 MCP 清单里声明的工具名，需精确匹配（清单见「MCP 工具」页）。" +
                    "命中后先说应答语，再把槽位作为参数派发；设备没有同名工具则跳过、不报错。",
            )
        }
        labeledField(
            "应答语",
            { d.reply },
            { d.reply = it; form.markDirty() },
            "好的，已为你打开灯。",
            help = "命中后要说的话（走 TTS 合成）。留空 = 只执行动作、不出声。",
        )
    }
}

// ===================== 槽位定义（slots） =====================

private fun ViewContainer<*, *>.nluSlotList(form: ConfigFormState, nlu: NluSectionState) {
    fieldLabel(
        "槽位定义（${nlu.slots.size} 条）",
        help = "模板里用到的 {槽位} 在这里命名并给出封闭值表（用于校验与抽取）；不给值表 = 开放槽位，" +
            "任意短语都接受。值表里的词会做归一化比对（忽略标点差异），命中后按配置原文回填。" +
            "含否定词（别/不要）或出现多个候选值时不会本地执行，交大模型判断。",
        trailing = {
            secondaryButton("＋ 添加槽位") {
                nlu.addSlot()
                form.markDirty()
            }
        },
    )
    vif({ nlu.slots.size == 0 }) {
        textNote("模板里用到的 {槽位} 在这里命名并给封闭值表；不给值表 = 开放槽位（任意短语）。", micro = true)
    }
    vfor({ nlu.slots }) { s ->
        nluSlotBlock(form, nlu, s)
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
            "槽位名",
            { s.name },
            { s.name = it; form.markDirty() },
            "room",
            help = "必须与模板里的 {占位名} 完全一致（区分大小写）。改名后不会自动同步模板，需自行修改。",
        )
        labeledTextArea(
            "封闭值表（每行一个）",
            { s.values },
            { s.values = it; form.markDirty() },
            "卧室\n客厅\n书房",
            height = 80f,
            help = "列出这个槽位所有合法取值，用于校验与抽取；留空 = 开放槽位（任意短语都接受，不再校验）。",
        )
        switchRow(
            "允许缺省", s.optional == "true",
            help = "打开后模板里可以不说这个槽位（如「开灯」省略房间），缺省时以空值执行；" +
                "关闭则该槽位必须出现。",
        ) {
            form.toggleStringBool(s::optional)
        }
    }
}
