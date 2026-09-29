package com.xiaozhi.admin

import com.tencent.kuikly.core.annotations.Page
import com.tencent.kuikly.core.base.Border
import com.tencent.kuikly.core.base.BorderStyle
import com.tencent.kuikly.core.base.Color
import com.tencent.kuikly.core.base.ViewBuilder
import com.tencent.kuikly.core.base.ViewContainer
import com.tencent.kuikly.core.module.NetworkModule
import com.tencent.kuikly.core.nvi.serialization.json.JSONObject
import com.tencent.kuikly.core.pager.Pager
import com.tencent.kuikly.core.reactive.handler.observable
import com.tencent.kuikly.core.views.Input
import com.tencent.kuikly.core.views.Scroller
import com.tencent.kuikly.core.views.Text
import com.tencent.kuikly.core.views.View

/**
 * 管理后台 · 配置页（config），位于 shared 公共层，跨端复用（web / Android / iOS / OHOS）。
 * 通过 server 的 GET /api/config 拉取全部参数并渲染表单；保存时 POST /api/config 写回 config.toml。
 * 网络统一走 Kuikly NetworkModule（跨平台，无需平台分支）。
 */
@Page("config")
class ConfigPage : Pager() {

    // ===== [server] =====
    var listen by observable("")
    var expectedToken by observable("")
    var workerThreads by observable("2")
    var adminDir by observable("")

    // ===== [audio] =====
    var downlinkSampleRate by observable("24000")
    var downlinkFrameMs by observable("60")
    var channels by observable("1")
    var binaryProtocolVersion by observable("1")

    // ===== [asr] =====
    var asrBackend by observable("mock")
    var asrModel by observable("")
    var asrTokens by observable("")
    var asrLanguage by observable("auto")
    var asrUseItn by observable("true")
    var asrNumThreads by observable("2")
    var asrProvider by observable("cpu")

    // ===== [vad] =====
    var vadModel by observable("")
    var vadThreshold by observable("0.5")
    var vadMinSilence by observable("0.25")
    var vadMinSpeech by observable("0.25")

    // ===== [tts] =====
    var ttsBackend by observable("mock")
    var ttsModel by observable("")
    var ttsVoices by observable("")
    var ttsTokens by observable("")
    var ttsDataDir by observable("")
    var ttsDictDir by observable("")
    var ttsLexicon by observable("")
    var ttsLang by observable("zh")
    var ttsSpeaker by observable("0")
    var ttsSpeed by observable("1.0")
    var ttsNumThreads by observable("1")

    // ===== [llm] =====
    var llmBackend by observable("mock")
    var llmApiBase by observable("")
    var llmApiKey by observable("")
    var llmModel by observable("")
    var llmSystemPrompt by observable("")
    var llmMaxHistory by observable("10")
    var llmTemperature by observable("0.7")

    var statusMsg by observable("")

    override fun body(): ViewBuilder {
        // ⚠️ DSL 容器带 @ScopeMarker（@DslMarker）：嵌套容器内**无法**经隐式接收者访问
        // 外部 Pager 的成员（属性 / 私有扩展），官方 demo 口径是先 `val ctx = this` 再显式 ctx.xxx。
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
                        fontSize(18f)
                        color(Color.WHITE)
                        text("XiaoZhi 管理后台 · 配置")
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

                ctx.sectionTitle("Server")
                ctx.field("listen", { ctx.listen }) { ctx.listen = it }
                ctx.field("expected_token", { ctx.expectedToken }) { ctx.expectedToken = it }
                ctx.field("worker_threads", { ctx.workerThreads }) { ctx.workerThreads = it }
                ctx.field("admin_dir", { ctx.adminDir }) { ctx.adminDir = it }

                ctx.sectionTitle("Audio")
                ctx.field("downlink_sample_rate", { ctx.downlinkSampleRate }) { ctx.downlinkSampleRate = it }
                ctx.field("downlink_frame_duration_ms", { ctx.downlinkFrameMs }) { ctx.downlinkFrameMs = it }
                ctx.field("channels", { ctx.channels }) { ctx.channels = it }
                ctx.field("binary_protocol_version", { ctx.binaryProtocolVersion }) { ctx.binaryProtocolVersion = it }

                ctx.sectionTitle("ASR")
                ctx.field("backend", { ctx.asrBackend }) { ctx.asrBackend = it }
                ctx.field("model", { ctx.asrModel }) { ctx.asrModel = it }
                ctx.field("tokens", { ctx.asrTokens }) { ctx.asrTokens = it }
                ctx.field("language", { ctx.asrLanguage }) { ctx.asrLanguage = it }
                ctx.field("use_itn", { ctx.asrUseItn }) { ctx.asrUseItn = it }
                ctx.field("num_threads", { ctx.asrNumThreads }) { ctx.asrNumThreads = it }
                ctx.field("provider", { ctx.asrProvider }) { ctx.asrProvider = it }

                ctx.sectionTitle("VAD")
                ctx.field("model", { ctx.vadModel }) { ctx.vadModel = it }
                ctx.field("threshold", { ctx.vadThreshold }) { ctx.vadThreshold = it }
                ctx.field("min_silence_duration", { ctx.vadMinSilence }) { ctx.vadMinSilence = it }
                ctx.field("min_speech_duration", { ctx.vadMinSpeech }) { ctx.vadMinSpeech = it }

                ctx.sectionTitle("TTS")
                ctx.field("backend", { ctx.ttsBackend }) { ctx.ttsBackend = it }
                ctx.field("model", { ctx.ttsModel }) { ctx.ttsModel = it }
                ctx.field("voices", { ctx.ttsVoices }) { ctx.ttsVoices = it }
                ctx.field("tokens", { ctx.ttsTokens }) { ctx.ttsTokens = it }
                ctx.field("data_dir", { ctx.ttsDataDir }) { ctx.ttsDataDir = it }
                ctx.field("dict_dir", { ctx.ttsDictDir }) { ctx.ttsDictDir = it }
                ctx.field("lexicon", { ctx.ttsLexicon }) { ctx.ttsLexicon = it }
                ctx.field("lang", { ctx.ttsLang }) { ctx.ttsLang = it }
                ctx.field("speaker", { ctx.ttsSpeaker }) { ctx.ttsSpeaker = it }
                ctx.field("speed", { ctx.ttsSpeed }) { ctx.ttsSpeed = it }
                ctx.field("num_threads", { ctx.ttsNumThreads }) { ctx.ttsNumThreads = it }

                ctx.sectionTitle("LLM")
                ctx.field("backend", { ctx.llmBackend }) { ctx.llmBackend = it }
                ctx.field("api_base", { ctx.llmApiBase }) { ctx.llmApiBase = it }
                ctx.field("api_key", { ctx.llmApiKey }) { ctx.llmApiKey = it }
                ctx.field("model", { ctx.llmModel }) { ctx.llmModel = it }
                ctx.field("system_prompt", { ctx.llmSystemPrompt }) { ctx.llmSystemPrompt = it }
                ctx.field("max_history", { ctx.llmMaxHistory }) { ctx.llmMaxHistory = it }
                ctx.field("temperature", { ctx.llmTemperature }) { ctx.llmTemperature = it }
            }

            // 底部保存栏（传统 DSL 无 Button 组件：View + click 事件模拟）
            View {
                attr {
                    height(60f)
                    flexDirectionRow()
                    alignItemsCenter()
                    paddingLeft(16f)
                    paddingRight(16f)
                    backgroundColor(Color(0xFFF7F7F7L))
                }
                View {
                    attr {
                        width(140f)
                        height(40f)
                        allCenter()
                        backgroundColor(Color(0xFF07C160L))
                    }
                    event { click { ctx.saveConfig() } }
                    Text {
                        attr {
                            fontSize(15f)
                            color(Color.WHITE)
                            text("保存配置")
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
        }
    }

    override fun pageDidAppear() {
        super.pageDidAppear()
        loadConfig()
    }

    private fun network(): NetworkModule = acquireModule(NetworkModule.MODULE_NAME)

    private fun loadConfig() {
        network().requestGet("/api/config", JSONObject()) { data, success, errorMsg, _ ->
            if (success) {
                fill(data)
                statusMsg = "已加载配置"
            } else {
                statusMsg = "加载失败: $errorMsg"
            }
        }
    }

    private fun saveConfig() {
        val body = JSONObject().apply {
            put("server", JSONObject().apply {
                put("listen", listen)
                put("expected_token", expectedToken)
                put("worker_threads", workerThreads.toIntOrNull() ?: 2)
                put("admin_dir", adminDir)
            })
            put("audio", JSONObject().apply {
                put("downlink_sample_rate", downlinkSampleRate.toIntOrNull() ?: 24000)
                put("downlink_frame_duration_ms", downlinkFrameMs.toIntOrNull() ?: 60)
                put("channels", channels.toIntOrNull() ?: 1)
                put("binary_protocol_version", binaryProtocolVersion.toIntOrNull() ?: 1)
            })
            put("asr", JSONObject().apply {
                put("backend", asrBackend)
                put("model", asrModel)
                put("tokens", asrTokens)
                put("language", asrLanguage)
                put("use_itn", asrUseItn.toBooleanStrictOrNull() ?: true)
                put("num_threads", asrNumThreads.toIntOrNull() ?: 2)
                put("provider", asrProvider)
            })
            put("vad", JSONObject().apply {
                put("model", vadModel)
                put("threshold", vadThreshold.toDoubleOrNull() ?: 0.5)
                put("min_silence_duration", vadMinSilence.toDoubleOrNull() ?: 0.25)
                put("min_speech_duration", vadMinSpeech.toDoubleOrNull() ?: 0.25)
            })
            put("tts", JSONObject().apply {
                put("backend", ttsBackend)
                put("model", ttsModel)
                put("voices", ttsVoices)
                put("tokens", ttsTokens)
                put("data_dir", ttsDataDir)
                put("dict_dir", ttsDictDir)
                put("lexicon", ttsLexicon)
                put("lang", ttsLang)
                put("speaker", ttsSpeaker.toIntOrNull() ?: 0)
                put("speed", ttsSpeed.toDoubleOrNull() ?: 1.0)
                put("num_threads", ttsNumThreads.toIntOrNull() ?: 1)
            })
            put("llm", JSONObject().apply {
                put("backend", llmBackend)
                put("api_base", llmApiBase)
                put("api_key", llmApiKey)
                put("model", llmModel)
                put("system_prompt", llmSystemPrompt)
                put("max_history", llmMaxHistory.toIntOrNull() ?: 10)
                put("temperature", llmTemperature.toDoubleOrNull() ?: 0.7)
            })
        }
        network().requestPost("/api/config", body) { _, success, errorMsg, _ ->
            statusMsg = if (success) "保存成功（引擎参数需重启生效）" else "保存失败: $errorMsg"
        }
    }

    private fun fill(obj: JSONObject) {
        obj.optJSONObject("server")?.let { s ->
            listen = s.optString("listen", listen)
            expectedToken = s.optString("expected_token", expectedToken)
            workerThreads = s.optInt("worker_threads", workerThreads.toIntOrNull() ?: 2).toString()
            adminDir = s.optString("admin_dir", adminDir)
        }
        obj.optJSONObject("audio")?.let { a ->
            downlinkSampleRate = a.optInt("downlink_sample_rate", downlinkSampleRate.toIntOrNull() ?: 24000).toString()
            downlinkFrameMs = a.optInt("downlink_frame_duration_ms", downlinkFrameMs.toIntOrNull() ?: 60).toString()
            channels = a.optInt("channels", channels.toIntOrNull() ?: 1).toString()
            binaryProtocolVersion = a.optInt("binary_protocol_version", binaryProtocolVersion.toIntOrNull() ?: 1).toString()
        }
        obj.optJSONObject("asr")?.let { a ->
            asrBackend = a.optString("backend", asrBackend)
            asrModel = a.optString("model", asrModel)
            asrTokens = a.optString("tokens", asrTokens)
            asrLanguage = a.optString("language", asrLanguage)
            asrUseItn = a.optBoolean("use_itn", asrUseItn.toBooleanStrictOrNull() ?: true).toString()
            asrNumThreads = a.optInt("num_threads", asrNumThreads.toIntOrNull() ?: 2).toString()
            asrProvider = a.optString("provider", asrProvider)
        }
        obj.optJSONObject("vad")?.let { v ->
            vadModel = v.optString("model", vadModel)
            vadThreshold = v.optDouble("threshold", vadThreshold.toDoubleOrNull() ?: 0.5).toString()
            vadMinSilence = v.optDouble("min_silence_duration", vadMinSilence.toDoubleOrNull() ?: 0.25).toString()
            vadMinSpeech = v.optDouble("min_speech_duration", vadMinSpeech.toDoubleOrNull() ?: 0.25).toString()
        }
        obj.optJSONObject("tts")?.let { t ->
            ttsBackend = t.optString("backend", ttsBackend)
            ttsModel = t.optString("model", ttsModel)
            ttsVoices = t.optString("voices", ttsVoices)
            ttsTokens = t.optString("tokens", ttsTokens)
            ttsDataDir = t.optString("data_dir", ttsDataDir)
            ttsDictDir = t.optString("dict_dir", ttsDictDir)
            ttsLexicon = t.optString("lexicon", ttsLexicon)
            ttsLang = t.optString("lang", ttsLang)
            ttsSpeaker = t.optInt("speaker", ttsSpeaker.toIntOrNull() ?: 0).toString()
            ttsSpeed = t.optDouble("speed", ttsSpeed.toDoubleOrNull() ?: 1.0).toString()
            ttsNumThreads = t.optInt("num_threads", ttsNumThreads.toIntOrNull() ?: 1).toString()
        }
        obj.optJSONObject("llm")?.let { l ->
            llmBackend = l.optString("backend", llmBackend)
            llmApiBase = l.optString("api_base", llmApiBase)
            llmApiKey = l.optString("api_key", llmApiKey)
            llmModel = l.optString("model", llmModel)
            llmSystemPrompt = l.optString("system_prompt", llmSystemPrompt)
            llmMaxHistory = l.optInt("max_history", llmMaxHistory.toIntOrNull() ?: 10).toString()
            llmTemperature = l.optDouble("temperature", llmTemperature.toDoubleOrNull() ?: 0.7).toString()
        }
    }
}

/**
 * 单个配置字段：标签 + 输入框（value 经 getter 传入以保持响应式）。
 * ⚠️ 必须是**文件级**扩展（官方 demo 口径）：Kuikly DSL 容器带 @ScopeMarker（@DslMarker），
 * 类内私有成员扩展在嵌套容器里既无法经隐式接收者解析（'cannot be called in this context'），
 * 显式 ctx.field(...) 也不行；文件级扩展无 dispatch receiver，走隐式接收者链天然可用。
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
