package com.xiaozhi.admin

import com.tencent.kuikly.core.base.ViewContainer
import com.tencent.kuikly.core.directives.vif
import com.tencent.kuikly.core.views.Text

/**
 * 「指令」配置卡（`[command]`）：**ASR → LLM 之间**的指令闸门。
 *
 * 打开后，识别文本在送进大模型之前先过**长度闸门**（归一化后超过 max_chars 个字的长句一律放行，
 * 避免「帮我关闭卧室的灯」被误判），再按词表匹配；命中即先说一句告别语，随后
 * **断开本次 WebSocket 会话**（一次大模型调用都不发生）。默认关闭时行为与没有这个
 * 插件完全一致。
 *
 * 独立文件的原因：`AGENTS.md` §5.10 文件规模约定（`ConfigCards.kt` 已接近上限），
 * 且这张卡的字段/文案自成一组（与 `ConfigSoulMemoryCards.kt` 同思路）。
 */
fun ViewContainer<*, *>.commandConfigCard(form: ConfigFormState) {
    val cmd = form.cmd
    groupedCard("指令闸门（ASR → LLM）") {
        // 整段档位提示（保存后新会话生效）：来自 /api/config/schema，服务端为唯一权威
        vif({ form.schema.sectionNote("command").isNotEmpty() }) {
            textNote(form.schema.sectionNote("command"), micro = true)
        }
        switchRow("enabled（识别到指令就断开会话）", cmd.enabled == "true") {
            cmd.enabled = if (cmd.enabled == "true") "false" else "true"
            form.dirty = true
        }
        Text {
            attr {
                fontSize(AdminType.caption)
                color(AdminColors.textTertiary)
                text("命中后先说告别语再断开本次会话，不会再调用大模型；默认关闭。")
            }
        }
        vif({ cmd.enabled == "true" }) {
            labeledTextArea(
                "keywords（每行一个指令词）",
                { cmd.keywords },
                { cmd.keywords = it; form.dirty = true },
                height = 88f,
            )
            dropdownField(
                label = "匹配方式",
                currentLabel = {
                    cmd.matchModeOptions.firstOrNull { it.first == cmd.matchMode }?.second ?: cmd.matchMode
                },
                options = { cmd.matchModeOptions },
                selectedId = { cmd.matchMode },
                isOpen = { form.openDropdown == "command_match_mode" },
                onToggle = {
                    form.openDropdown = if (form.openDropdown == "command_match_mode") "" else "command_match_mode"
                },
                onSelect = {
                    cmd.matchMode = it
                    form.dirty = true
                    form.openDropdown = ""
                },
            )
            labeledField(
                "max_chars（最长字数，超过就不做指令判断）",
                { cmd.maxChars },
                { cmd.maxChars = it; form.dirty = true },
                "5",
            )
            labeledField(
                "reply（告别语）",
                { cmd.reply },
                { cmd.reply = it; form.dirty = true },
                "好的，我先退下了。",
            )
            Text {
                attr {
                    fontSize(AdminType.caption)
                    color(AdminColors.textTertiary)
                    text("识别文本会先去标点与句末语气词：「退下吧。」等价于「退下」；留空告别语 = 直接断开、不出声。")
                }
            }
            Text {
                attr {
                    fontSize(AdminType.caption)
                    color(AdminColors.textTertiary)
                    text("长度闸门：归一化后超过 max_chars（默认 5）个字的长句携带信息，直接交给大模型——「帮我关闭卧室的灯」不会被当成关机指令。留空按 5 处理。")
                }
            }
            // 0 = 不限制是显式拆掉保险，必须说出来（而不是让用户以为自己填错了）
            vif({ (cmd.maxChars.trim().toIntOrNull() ?: 5) <= 0 }) {
                Text {
                    attr {
                        fontSize(AdminType.caption)
                        color(AdminColors.warningTintText)
                        text("⚠️ max_chars = 0 表示不限制长度：任何长度、只要匹配上指令词就会断开会话，请确认这符合预期。")
                    }
                }
            }
            // ⚠️ contains 的代价必须显式说出来：它会把「关闭闹钟」也当成关机指令
            vif({ cmd.matchMode == "contains" }) {
                Text {
                    attr {
                        fontSize(AdminType.caption)
                        color(AdminColors.warningTintText)
                        text("⚠️ contains 会让「关闭闹钟」「把灯关闭」这类短请求也断开会话（长度闸门只挡 6 个字以上的长句）；不确定就用 exact。")
                    }
                }
            }
        }
        vif({ cmd.enabled != "true" }) {
            Text {
                attr {
                    fontSize(AdminType.caption)
                    color(AdminColors.textTertiary)
                    text("未启用：识别结果照常交给大模型（行为与加本插件之前一致）。")
                }
            }
        }
    }
}
