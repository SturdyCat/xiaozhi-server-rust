package com.xiaozhi.admin

import androidx.compose.runtime.Composable
import com.gearui.components.button.Button
import com.gearui.components.button.ButtonType
import com.gearui.components.slider.Slider
import com.tencent.kuikly.compose.foundation.layout.*
import com.tencent.kuikly.compose.ui.Modifier
import com.tencent.kuikly.compose.ui.text.input.KeyboardType
import com.tencent.kuikly.compose.ui.unit.dp
import com.tencent.kuikly.core.nvi.serialization.json.JSONArray
import com.xiaozhi.admin.kit.*

@Composable
internal fun EmNativePlayback(d: EchoMuseConfigDrafts, enabled: Boolean) {
    EmNativeConfigStage("01", "播放", "均衡器 → 输出处理 → 设备扬声器。官方八段均衡器预设；保留旧设备自定义频段数，不静默重写。") {
        EmNativeConfigColumns(left = {
            val bands = try { JSONArray(d.eqBandsDraft.ifBlank { EM_EQ_PRESET_VALUES.getValue("flat") }) } catch (_: Exception) { null }
            if (bands != null && bands.length() in 1..16) {
                val labels = listOf("125 Hz", "250 Hz", "500 Hz", "1 kHz", "2 kHz", "3.5 kHz", "5.5 kHz", "8 kHz")
                for (i in 0 until bands.length()) {
                    val value = (bands.opt(i) as? Number)?.toDouble() ?: continue
                    fieldLabelRow(if (bands.length() == 8) labels[i] else "频段 ${i + 1}")
                    EmNote("${emNum(value, 1)} dB")
                    Slider(value = value.toFloat().coerceIn(-12f, 12f), enabled = enabled,
                        valueRange = -12f..12f, steps = 23, onValueChange = { next ->
                            val updated = JSONArray()
                            for (index in 0 until bands.length()) {
                                updated.put(if (index == i) next.toDouble() else bands.opt(index))
                            }
                            d.eqBandsDraft = updated.toString()
                        })
                }
            }
            actionRow {
                for ((id, label) in emEqPresetOptions()) Button(text = label, type = ButtonType.OUTLINE,
                    disabled = !enabled, onClick = { d.eqBandsDraft = EM_EQ_PRESET_VALUES.getValue(id) })
            }
            EmNote("预设：${d.eqPresetLabel}；上传或保存不等于听感已验证。")
            labeledTextArea("均衡器频段", d.eqBandsDraft, { d.eqBandsDraft = it }, enabled = enabled,
                help = "非空数字数组；留空不修改。老设备的不同频段数保留原值；选择预设后使用官方八段。")
        }, right = {
            labeledSelect("语音播报电平", d.responseLevel, d.responseLevels, { if (enabled) d.responseLevel = it },
                help = "相对设备音量：正常、增加 6 dB 或增加 12 dB；高音量时提升会减弱。")
            EmNativeSignedNumber("音乐压低深度（dB）", d.duckDbDraft, { d.duckDbDraft = it }, "-60～0", enabled)
            switchRow("音量按键提示音", d.volumeButtonSound, { d.volumeButtonSound = it }, enabled = enabled)
            EmNote("当前音量在「状态」中调整；开机音量仅作兼容设置，不代表实时音量。")
            EmNativeConfigAdvanced {
                labeledIntField("开机音量", d.startupVolumeDraft, { d.startupVolumeDraft = it }, placeholder = "0～127；留空不修改", enabled = enabled)
                switchRow("低音保护", d.bassGuardEnabled, { d.bassGuardEnabled = it }, enabled = enabled)
                switchRow("响度补偿", d.eqLoudness, { d.eqLoudness = it }, enabled = enabled)
                EmNativeSignedNumber("低音保护阈值（dB）", d.bassGuardDbDraft, { d.bassGuardDbDraft = it }, "-60～24", enabled)
                switchRow("输出限幅器", d.limiterEnabled, { d.limiterEnabled = it }, enabled = enabled)
                EmWarn("建议保持限幅器开启，避免均衡器提升造成削波。")
                EmNativeSignedNumber("限幅阈值（dB）", d.limiterThresholdDraft, { d.limiterThresholdDraft = it }, "-60～0", enabled)
                labeledDecimalField("限幅释放时间（毫秒）", d.limiterReleaseDraft, { d.limiterReleaseDraft = it },
                    placeholder = "10～2000；留空不修改", enabled = enabled)
            }
            EmNote("边生成边播报由官方控制器提供，本服务器暂无对应设备设置。")
        })
    }
}

/** Signed values cannot use the unsigned kit decimal filter (it would turn -18 into +18). */
@Composable
internal fun EmNativeSignedNumber(label: String, value: String, change: (String) -> Unit, range: String, enabled: Boolean) {
    labeledField(label, value, change, placeholder = "$range；留空不修改", enabled = enabled,
        keyboardType = KeyboardType.Decimal,
        inputFilter = { raw -> raw.filter { it in '0'..'9' || it == '-' || it == '.' } })
}

@Composable
internal fun EmNativeMicrophones(d: EchoMuseConfigDrafts, enabled: Boolean) {
    EmNativeConfigStage("03", "麦克风", "七麦阵列采集。方向预设影响语音轮次；唤醒麦克风单独设置。") {
        actionRow {
            for ((label, angle) in listOf("全向" to "-1", "前方" to "90", "后方" to "270")) {
                Button(text = label, type = ButtonType.OUTLINE, disabled = !enabled,
                    onClick = { d.beamAngleDraft = angle; d.beamformingEnabled = angle != "-1" })
            }
        }
        EmNativeConfigAdvanced {
            EmNativeConfigColumns(left = {
                labeledIntField("模拟前端增益", d.adcMicpgaDraft, { d.adcMicpgaDraft = it }, placeholder = "0～127", enabled = enabled)
                labeledIntField("数字增益", d.adcDigitalGainDraft, { d.adcDigitalGainDraft = it }, placeholder = "0～127", enabled = enabled)
                labeledIntField("处理增益（dB）", d.micGainDbDraft, { d.micGainDbDraft = it }, placeholder = "0～42", enabled = enabled)
                EmNativeSignedNumber("波束角度", d.beamAngleDraft, { d.beamAngleDraft = it }, "-1～360；-1 为自动", enabled)
                switchRow("波束成形", d.beamformingEnabled, { d.beamformingEnabled = it }, enabled = enabled)
                labeledIntField("唤醒麦克风", d.wakeMic, { d.wakeMic = it }, placeholder = "0～6；0 为中心麦克风", enabled = enabled)
            }, right = {
                switchRow("回声消除（AEC）", d.aecEnabled, { d.aecEnabled = it }, enabled = enabled)
                labeledIntField("回声参考延时（毫秒）", d.aecDelayMsDraft, { d.aecDelayMsDraft = it }, placeholder = "0～1000", enabled = enabled)
                labeledIntField("回声滤波尾长（毫秒）", d.aecTailMsDraft, { d.aecTailMsDraft = it }, placeholder = "0～500", enabled = enabled)
                labeledSelect("回声参考源", d.aecRefSource, d.aecRefSources, { if (enabled) d.aecRefSource = it })
                EmNote("硬件参考源需要设备支持；强制硬件而设备无环回时可能无法消除回声。")
                EmNote("官方 DTLN 降噪与语音片段保存不在当前服务端白名单内。")
            })
        }
    }
}

@Composable
internal fun EmNativeAdvanced(d: EchoMuseConfigDrafts, enabled: Boolean) {
    EmNativeConfigStage("05", "进阶", "按键语音轮次的处理增益与语音门限。唤醒与按键轮次的实际结束判定以服务端和设备实现为准。") {
        switchRow("自动增益（AGC）", d.agcEnabled, { d.agcEnabled = it }, enabled = enabled)
        EmNativeConfigColumns(left = {
            labeledDecimalField("语音活动门限", d.vadThresholdDraft, { d.vadThresholdDraft = it },
                placeholder = "大于 0 且不超过 1；留空不修改", enabled = enabled)
            labeledIntField("开口判定时长（毫秒）", d.vadSpeechMsDraft, { d.vadSpeechMsDraft = it },
                placeholder = "0～60000；留空不修改", enabled = enabled)
        }, right = {
            labeledIntField("收尾静音时长（毫秒）", d.vadSilenceMsDraft, { d.vadSilenceMsDraft = it },
                placeholder = "0～60000；留空不修改", enabled = enabled)
            EmNote("按键自动化事件、USB 控制台密码/超时与控制器地址暂无服务端设置接口。")
        })
        EmNativeConfigAdvanced("原始设置 JSON") {
            labeledTextArea("补充设置", d.extraJsonDraft, { d.extraJsonDraft = it }, enabled = enabled,
                help = "只接受服务端白名单内的键与正确类型；非空对象。补充数值可覆盖上方数值，开关、枚举、播放器名称和非空动画以可见控件为准；请勿重复填写。不支持的键会明确报错。")
        }
    }
}
