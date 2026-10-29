package com.xiaozhi.admin

import androidx.compose.runtime.Composable
import com.gearui.components.alert.Alert
import com.gearui.components.alert.AlertStatus
import com.gearui.foundation.layout.Spacing
import com.tencent.kuikly.compose.foundation.layout.fillMaxWidth
import com.tencent.kuikly.compose.foundation.layout.padding
import com.tencent.kuikly.compose.ui.Modifier
import com.xiaozhi.admin.kit.labeledField
import com.xiaozhi.admin.kit.labeledIntField
import com.xiaozhi.admin.kit.labeledSelect
import com.xiaozhi.admin.kit.labeledTextArea
import com.xiaozhi.admin.kit.switchRow

/**
 * 「指令」配置卡（`[command]`）：**ASR → LLM 之间**的指令闸门（Compose 版）。
 *
 * 打开后，识别文本在送进大模型之前先过**长度闸门**（归一化后超过 `max_chars` 个字的长句一律放行，
 * 避免「帮我关闭卧室的灯」被误判），再按词表匹配；命中即先说一句告别语，随后**用 MCP 让设备
 * 结束聆听**（服务器是 MCP Client，发 `tools/call` 调设备工具 `self.audio_listener.stop`：
 * 设备回到待唤醒、音频通道保留，下次唤醒不重连）；设备未声明 MCP 或调用失败时降级为断开本次
 * WebSocket 会话（一次大模型调用都不发生）。默认关闭时行为与没有这个插件完全一致。
 *
 * 文案里的两条 ⚠️（`max_chars = 0` 不限制、`contains` 匹配更宽松）是**安全提示**，
 * 不是装饰：它们说明"这样配会让什么短请求被误判成关机"，必须原样保留。
 */
@Composable
fun CommandConfigCard(form: ConfigFormState) {
    val cmd = form.cmd
    LabeledCard(title = "指令闸门（ASR → LLM）") {
        // 整段档位提示（保存后新会话生效）：来自 /api/config/schema，服务端为唯一权威
        val sectionNote = form.schema.sectionNote("command")
        if (sectionNote.isNotEmpty()) HintLine(sectionNote)

        switchRow(
            label = "启用指令闸门",
            checked = cmd.enabled == "true",
            onToggle = { form.toggleStringBool(cmd::enabled) },
            help = "识别到指令词就让设备结束聆听：先说告别语，再让设备退出（音频通道保留、下次唤醒不重连）；" +
                "设备不支持 MCP 时降级为断开本次会话。全程不调用大模型。默认关闭。",
        )

        if (cmd.enabled == "true") {
            labeledTextArea(
                label = "指令词（每行一个）",
                value = cmd.keywords,
                onValueChange = {
                    cmd.keywords = it
                    form.markDirty()
                },
                help = "命中任意一行即触发。识别文本会先去标点与句末语气词——「退下吧。」等价于「退下」。",
            )
            labeledSelect(
                label = "匹配方式",
                selectedId = cmd.matchMode,
                options = cmd.matchModeOptions,
                onSelect = {
                    cmd.matchMode = it
                    form.markDirty()
                },
                help = "整句相等 = 只认完整的一句话（推荐，安全）；句中出现即命中 = 更宽松，" +
                    "「关闭闹钟」「把灯关闭」这类短请求也会误触发结束聆听。不确定就用整句相等。",
            )
            labeledIntField(
                label = "最长字数",
                value = cmd.maxChars,
                onValueChange = {
                    cmd.maxChars = it
                    form.markDirty()
                },
                placeholder = "5",
                help = "归一化后超过该字数的句子携带信息，直接交给大模型——「帮我关闭卧室的灯」不会被当成关机指令。" +
                    "留空按 5 处理；0 = 不限制长度（任何长度只要命中指令词就触发）。",
            )
            if ((cmd.maxChars.trim().toIntOrNull() ?: 5) <= 0) {
                Alert(
                    title = "最长字数 = 0：不限制长度",
                    description = "任何长度、只要匹配上指令词就会让设备结束聆听" +
                        "（MCP 不可用时断开本次会话），请确认这符合预期。",
                    status = AlertStatus.WARNING,
                    modifier = Modifier.fillMaxWidth().padding(top = Spacing.xs),
                )
            }
            // ⚠️ contains 的代价必须显式说出来：它会把「关闭闹钟」也当成关机指令
            if (cmd.matchMode == "contains") {
                Alert(
                    title = "「句中出现即命中」更宽松",
                    description = "会让「关闭闹钟」「把灯关闭」这类短请求也触发结束聆听/断开" +
                        "（长度闸门只挡 6 个字以上的长句）；不确定就改用「整句相等」。",
                    status = AlertStatus.WARNING,
                    modifier = Modifier.fillMaxWidth().padding(top = Spacing.xs),
                )
            }
            labeledField(
                label = "告别语",
                value = cmd.reply,
                onValueChange = {
                    cmd.reply = it
                    form.markDirty()
                },
                placeholder = "好的，我先退下了。",
                help = "触发后先说这句再结束聆听。留空 = 直接断开、不出声。",
            )
        } else {
            HintLine("未启用：识别结果照常交给大模型（行为与加本插件之前一致）。")
        }
    }
}
