package com.xiaozhi.admin

import com.tencent.kuikly.core.annotations.Page
import com.tencent.kuikly.core.base.Border
import com.tencent.kuikly.core.base.BorderStyle
import com.tencent.kuikly.core.base.Color
import com.tencent.kuikly.core.base.ViewBuilder
import com.tencent.kuikly.core.base.ViewContainer
import com.tencent.kuikly.core.module.Module
import com.tencent.kuikly.core.module.RouterModule
import com.tencent.kuikly.core.nvi.serialization.json.JSONObject
import com.tencent.kuikly.core.pager.Pager
import com.tencent.kuikly.core.reactive.handler.observable
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
        // ⚠️ DSL 容器带 @ScopeMarker（@DslMarker）：嵌套容器内无法隐式访问 Pager 成员，
        // 官方 demo 口径是先 `val ctx = this` 再显式 ctx.xxx（ConfigPage 同款约定）。
        val ctx = this
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
                    paddingLeft(16f)
                    paddingRight(16f)
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
                // 传统 DSL 无 Button 组件（只有 Compose DSL 的 ButtonView）：View + click 模拟
                View {
                    attr {
                        height(36f)
                        paddingLeft(12f)
                        paddingRight(12f)
                        allCenter()
                        backgroundColor(Color(0xFF3A3A3AL))
                    }
                    event { click { ctx.openAdmin() } }
                    Text {
                        attr {
                            fontSize(13f)
                            color(Color.WHITE)
                            text("管理后台")
                        }
                    }
                }
            }

            Scroller {
                attr {
                    flex(1f)
                    paddingLeft(16f)
                    paddingRight(16f)
                    paddingTop(12f)
                    paddingBottom(12f)
                }

                ctx.sectionTitle("连接")
                ctx.field("server url", { ctx.serverUrl }) { ctx.serverUrl = it }
                ctx.field("token（可选）", { ctx.token }) { ctx.token = it }
                View {
                    attr {
                        flexDirectionRow()
                        marginTop(8f)
                    }
                    View {
                        attr {
                            width(140f)
                            height(40f)
                            allCenter()
                            backgroundColor(if (ctx.connected) Color(0xFF888888L) else Color(0xFF07C160L))
                        }
                        event { click { if (ctx.connected) ctx.disconnect() else ctx.connect() } }
                        Text {
                            attr {
                                fontSize(15f)
                                color(Color.WHITE)
                                text(if (ctx.connected) "已连接" else "连接")
                            }
                        }
                    }
                    Text {
                        attr {
                            marginLeft(12f)
                            fontSize(13f)
                            color(Color(0xFF888888L))
                            text(ctx.statusMsg)
                        }
                    }
                }

                ctx.sectionTitle("ASR 识别测试")
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
                    View {
                        attr {
                            width(140f)
                            height(40f)
                            allCenter()
                            backgroundColor(if (ctx.recording) Color(0xFFE64340L) else Color(0xFF07C160L))
                        }
                        event { click { if (ctx.recording) ctx.stopAsr() else ctx.startAsr() } }
                        Text {
                            attr {
                                fontSize(15f)
                                color(Color.WHITE)
                                text(if (ctx.recording) "停止录音" else "开始录音")
                            }
                        }
                    }
                }
                View {
                    attr {
                        marginTop(10f)
                        padding(12f)
                        border(Border(1f, BorderStyle.SOLID, Color(0xFFEEEEEL)))
                        borderRadius(6f)
                        minHeight(60f)
                    }
                    Text {
                        attr {
                            fontSize(15f)
                            color(Color(0xFF222222L))
                            text(if (ctx.asrText.isEmpty()) "识别结果将显示在此" else ctx.asrText)
                        }
                    }
                }

                ctx.sectionTitle("TTS 合成测试")
                Input {
                    attr {
                        height(80f)
                        marginTop(6f)
                        border(Border(1f, BorderStyle.SOLID, Color(0xFFDDDDDDL)))
                        borderRadius(6f)
                        fontSize(14f)
                        color(Color(0xFF222222L))
                        text(ctx.ttsText)
                        placeholder("输入要合成的文字")
                    }
                    event { textDidChange { params -> ctx.ttsText = params.text } }
                }
                ctx.field("语言 (auto/zh/en/ja/ko/yue)", { ctx.ttsLang }) { ctx.ttsLang = it }
                ctx.field("语速 (0.5~2.0)", { ctx.ttsSpeed }) { ctx.ttsSpeed = it }
                ctx.field("语音角色 speaker (sid)", { ctx.ttsSpeaker }) { ctx.ttsSpeaker = it }
                View {
                    attr {
                        marginTop(8f)
                        width(140f)
                        height(40f)
                        allCenter()
                        backgroundColor(Color(0xFF07C160L))
                    }
                    event { click { ctx.speak() } }
                    Text {
                        attr {
                            fontSize(15f)
                            color(Color.WHITE)
                            text("合成并播放")
                        }
                    }
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
        // RouterModule.openPage(pageName, pageData: JSONObject? = null)——第二参是 JSONObject，
        // 不能传 mutableMapOf()。
        router().openPage("config")
    }
}

/**
 * 单个配置字段：标签 + 输入框（value 经 getter 传入以保持响应式）。
 * ⚠️ 必须是**文件级**扩展（同 ConfigPage.kt 的约定）：类内私有成员扩展在 @ScopeMarker
 * 限定的嵌套容器里无法解析（'cannot be called in this context'）。
 */
private fun ViewContainer<*, *>.field(label: String, getValue: () -> String, onChange: (String) -> Unit) {
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
            border(Border(1f, BorderStyle.SOLID, Color(0xFFDDDDDDL)))
            borderRadius(6f)
            fontSize(14f)
            color(Color(0xFF222222L))
            text(getValue())
            placeholder("")
        }
        event { textDidChange { params -> onChange(params.text) } }
    }
}

/** 分组小标题 */
private fun ViewContainer<*, *>.sectionTitle(title: String) {
    Text {
        attr {
            fontSize(15f)
            fontWeightMedium()
            color(Color(0xFF1F1F1FL))
            marginTop(20f)
            marginBottom(4f)
            text(title)
        }
    }
}
