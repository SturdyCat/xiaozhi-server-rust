package com.xiaozhi.admin

import com.tencent.kuikly.core.annotations.Page
import com.tencent.kuikly.core.base.ViewBuilder
import com.tencent.kuikly.core.base.attr.Color
import com.tencent.kuikly.core.module.Module
import com.tencent.kuikly.core.module.RouterModule
import com.tencent.kuikly.core.nvi.serialization.json.JSONObject
import com.tencent.kuikly.core.pager.Pager
import com.tencent.kuikly.core.reactive.handler.observable
import com.tencent.kuikly.core.views.Button
import com.tencent.kuikly.core.views.Input
import com.tencent.kuikly.core.views.Scroller
import com.tencent.kuikly.core.views.Text
import com.tencent.kuikly.core.views.View

/**
 * ASR / TTS 测试台（macOS 专用页面，仅编译进 macOS 框架）。
 *
 * 替代原 web 版测试台：浏览器受麦克风 / WebSocket 二进制等限制，不适合做语音测试；
 * 原生 Mac App 直接用 XiaoZhiModule 桥接麦克风与音频播放，复用 server 的测试协议
 * （asr_test 一次性识别、tts_test 流式合成）。管理后台（config）为跨平台页面，
 * 本页提供「管理后台」入口跳转过去。
 */
@Page("test")
class TestPage : Pager() {

    var serverUrl by observable("ws://127.0.0.1:8000/api/ws")
    var token by observable("")
    var connected by observable(false)
    var recording by observable(false)
    var asrText by observable("")
    var ttsText by observable("你好，小智")
    var ttsLang by observable("auto")     // 参考 SenseVoice：auto 自动检测 zh/en/ja/ko/yue
    var ttsSpeed by observable("1.0")     // 参考 kokoro.js demo：语速
    var ttsSpeaker by observable("0")     // Kokoro 语音角色 sid
    var statusMsg by observable("")

    override fun createExternalModules(): Map<String, Module> {
        return mapOf(XiaoZhiModule.MODULE_NAME to XiaoZhiModule())
    }

    override fun body(): ViewBuilder {
        return {
            attr {
                flex(1f)
                flexDirectionColumn()
                backgroundColor(Color.WHITE)
            }

            // 顶部标题栏
            View {
                attr {
                    height(56f)
                    flexDirectionRow()
                    alignItemsCenter()
                    paddingHorizontal(16f)
                    backgroundColor(Color(0xFF1F1F1FL))
                }
                Text {
                    attr {
                        flex(1f)
                        fontSize(18f)
                        color(Color.WHITE)
                        text("XiaoZhi 测试台 · ASR/TTS")
                    }
                }
                Button {
                    attr {
                        height(36f)
                        paddingHorizontal(12f)
                        backgroundColor(Color(0xFF3A3A3AL))
                        color(Color.WHITE)
                        fontSize(13f)
                        text("管理后台")
                    }
                    event { click { openAdmin() } }
                }
            }

            Scroller {
                attr {
                    flex(1f)
                    paddingHorizontal(16f)
                    paddingVertical(12f)
                }

                sectionTitle("连接")
                field("server url", { serverUrl }) { serverUrl = it }
                field("token（可选）", { token }) { token = it }
                View {
                    attr {
                        flexDirectionRow()
                        marginTop(8f)
                    }
                    Button {
                        attr {
                            width(140f)
                            height(40f)
                            backgroundColor(if (connected) Color(0xFF888888L) else Color(0xFF07C160L))
                            color(Color.WHITE)
                            fontSize(15f)
                            text(if (connected) "已连接" else "连接")
                        }
                        event { click { if (connected) disconnect() else connect() } }
                    }
                    Text {
                        attr {
                            marginLeft(12f)
                            fontSize(13f)
                            color(Color(0xFF888888L))
                            text(statusMsg)
                        }
                    }
                }

                sectionTitle("ASR 识别测试")
                Text {
                    attr {
                        fontSize(12f)
                        color(Color(0xFF999999L))
                        marginTop(4f)
                        text("语言由 SenseVoice 自动检测（zh/en/ja/ko/yue）")
                    }
                }
                View {
                    attr {
                        flexDirectionRow()
                        marginTop(8f)
                    }
                    Button {
                        attr {
                            width(140f)
                            height(40f)
                            backgroundColor(if (recording) Color(0xFFE64340L) else Color(0xFF07C160L))
                            color(Color.WHITE)
                            fontSize(15f)
                            text(if (recording) "停止录音" else "开始录音")
                        }
                        event { click { if (recording) stopAsr() else startAsr() } }
                    }
                }
                View {
                    attr {
                        marginTop(10f)
                        padding(12f)
                        borderWidth(1f)
                        borderColor(Color(0xFFEEEEEL))
                        borderRadius(6f)
                        minHeight(60f)
                    }
                    Text {
                        attr {
                            fontSize(15f)
                            color(Color(0xFF222222L))
                            text(if (asrText.isEmpty()) "识别结果将显示在此" else asrText)
                        }
                    }
                }

                sectionTitle("TTS 合成测试")
                Input {
                    attr {
                        height(80f)
                        marginTop(6f)
                        borderWidth(1f)
                        borderColor(Color(0xFFDDDDDDL))
                        borderRadius(6f)
                        paddingHorizontal(10f)
                        fontSize(14f)
                        color(Color(0xFF222222L))
                        text(ttsText)
                        placeholder("输入要合成的文字")
                    }
                    event { input { t -> ttsText = t } }
                }
                field("语言 (auto/zh/en/ja/ko/yue)", { ttsLang }) { ttsLang = it }
                field("语速 (0.5~2.0)", { ttsSpeed }) { ttsSpeed = it }
                field("语音角色 speaker (sid)", { ttsSpeaker }) { ttsSpeaker = it }
                Button {
                    attr {
                        marginTop(8f)
                        width(140f)
                        height(40f)
                        backgroundColor(Color(0xFF07C160L))
                        color(Color.WHITE)
                        fontSize(15f)
                        text("合成并播放")
                    }
                    event { click { speak() } }
                }
            }
        }
    }

    private fun xz(): XiaoZhiModule = acquireModule(XiaoZhiModule.MODULE_NAME)
    private fun router(): RouterModule = acquireModule(RouterModule.MODULE_NAME)

    private fun connect() {
        xz().connect(serverUrl, token) { result ->
            val ok = result?.optBoolean("success", false) ?: false
            connected = ok
            statusMsg = if (ok) "已连接" else "连接失败: ${result?.optString("error", "") ?: ""}"
        }
    }

    private fun disconnect() {
        xz().disconnect()
        connected = false
        statusMsg = "已断开"
    }

    private fun startAsr() {
        recording = true
        asrText = ""
        statusMsg = "录音中…"
        xz().startAsr { result ->
            asrText = result?.optString("text", "") ?: ""
            recording = false
            statusMsg = "识别完成"
        }
    }

    private fun stopAsr() {
        recording = false
        xz().stopAsr()
        statusMsg = "已停止，等待识别结果…"
    }

    private fun speak() {
        if (ttsText.isBlank()) {
            statusMsg = "请输入要合成的文字"
            return
        }
        statusMsg = "合成中…"
        val speed = ttsSpeed.toDoubleOrNull() ?: 1.0
        val speaker = ttsSpeaker.toIntOrNull() ?: 0
        xz().speak(ttsText, speaker, ttsLang, speed) { _ ->
            statusMsg = "TTS 播放完成"
        }
    }

    private fun openAdmin() {
        router().openPage("config", mutableMapOf())
    }

    private fun ViewBuilder.field(label: String, getValue: () -> String, onChange: (String) -> Unit) {
        Text {
            attr {
                fontSize(13f)
                color(Color(0xFF888888L))
                marginTop(12f)
                text(label)
            }
        }
        Input {
            attr {
                height(40f)
                marginTop(6f)
                borderWidth(1f)
                borderColor(Color(0xFFDDDDDDL))
                borderRadius(6f)
                paddingHorizontal(10f)
                fontSize(14f)
                color(Color(0xFF222222L))
                text(getValue())
                placeholder("")
            }
            event { input { t -> onChange(t) } }
        }
    }

    private fun ViewBuilder.sectionTitle(title: String) {
        Text {
            attr {
                fontSize(15f)
                fontWeight600()
                color(Color(0xFF1F1F1FL))
                marginTop(20f)
                marginBottom(4f)
                text(title)
            }
        }
    }
}
