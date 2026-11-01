package com.xiaozhi.admin

import androidx.compose.runtime.Composable
import com.gearui.components.button.Button
import com.gearui.components.button.ButtonType
import com.gearui.components.slider.Slider
import com.tencent.kuikly.compose.foundation.layout.*
import com.tencent.kuikly.compose.ui.Modifier
import com.xiaozhi.admin.kit.*

@Composable
internal fun EmNativeWake(row: EchoMuseDeviceRow, d: EchoMuseConfigDrafts, enabled: Boolean) {
    EmNativeConfigStage("02", "唤醒词", "选择模型与识别位置。模型出现在清单中不代表已经安装到设备。") {
        EmNativeConfigColumns(left = {
            val options = d.owwModelOptions.toList()
            for (pair in options.chunked(2)) {
                Row(Modifier.fillMaxWidth()) {
                    for ((id, _) in pair) {
                        Button(text = EchoMuseWakeWord.displayName(id), modifier = Modifier.weight(1f),
                            type = if (d.owwModel == id) ButtonType.FILL else ButtonType.OUTLINE,
                            disabled = !enabled, onClick = { d.owwModel = id })
                    }
                }
            }
            EmNote("当前模型：${EchoMuseWakeWord.displayName(d.owwModel)}")
            EmWarn("自定义模型从「模型文件」上传；上传到服务器不等于已安装到设备，未安装时可能无法唤醒。")
        }, right = {
            switchRow("启用唤醒词", d.wakeWordEnabled, { d.wakeWordEnabled = it }, enabled = enabled)
            fieldLabelRow("唤醒灵敏度")
            EmNote("阈值 ${d.owwThreshold}；左侧更精确，右侧更容易触发。历史轨外值原样保留，拖动才改变。")
            Slider(value = EchoMuseWakeWord.trackProgress(d.owwThreshold),
                onValueChange = { d.owwThreshold = EchoMuseWakeWord.quantizeTrack(it) },
                valueRange = 0f..1f, steps = 34, enabled = enabled,
                leftLabel = "精确", rightLabel = "灵敏")
            switchRow("播放时允许唤醒打断", d.bargeInEnabled, { d.bargeInEnabled = it }, enabled = enabled,
                description = "建议先开启回声消除，避免播报内容触发自己。")
            labeledDecimalField("播放时打断阈值", d.bargeInThresholdDraft, { d.bargeInThresholdDraft = it },
                placeholder = "0～1；留空不修改", enabled = enabled)
            switchRow("唤醒提示音", d.wakeSound, { d.wakeSound = it }, enabled = enabled)
            if (d.wakeSound) labeledSelect("提示音音量", d.wakeSoundLevel,
                listOf("quiet" to "轻柔", "medium" to "适中", "loud" to "响亮"),
                { if (enabled) d.wakeSoundLevel = it })
            labeledSelect("唤醒识别位置", d.owwOnDevice,
                buildList {
                    add("on" to "设备本地识别")
                    add("off" to "控制器侧识别（本服务端暂未提供）")
                    if (d.owwOnDevice == "shadow") add("shadow" to "诊断：持续推流、双端打分")
                }, { if (enabled) d.owwOnDevice = it })
            if (d.owwOnDevice == "on") EmNote("设备需要支持本地唤醒并安装运行时及所选模型。保存成功不代表已具备识别能力。")
            else EmWarn("控制器侧 openWakeWord 评分与官方双端诊断未由本服务端实现；保存此档位不代表能唤醒。")
            if (row.online && "oww_trigger" !in row.capabilities) EmWarn("当前设备未声明本地唤醒触发能力；请核对固件与「更新」中的资产状态。")
            EmNote("官方 Speex 预降噪、唤醒样本保存、最小样本分数与多设备仲裁暂无服务端接口。")
        })
    }
}
