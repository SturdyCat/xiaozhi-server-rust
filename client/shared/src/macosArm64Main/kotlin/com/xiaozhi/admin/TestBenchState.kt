package com.xiaozhi.admin

import com.tencent.kuikly.core.base.ViewContainer
import com.tencent.kuikly.core.pager.Pager
import com.tencent.kuikly.core.reactive.handler.observable
import com.tencent.kuikly.core.views.Text
import com.tencent.kuikly.core.views.View

/**
 * ASR / TTS 测试台状态（macOS 专用，macosArm64Main）。
 *
 * 持有连接 / 录音 / 合成 / 识别结果等状态，方法 connect/disconnect/startAsr/stopAsr/speak
 * 照 TestPage.kt 现有实现调用 XiaoZhiModule（web/Android/iOS 不注册该模块，故测试功能仅 Mac App）。
 *
 * ⚠️ XiaoZhiModule 由持有它的 Pager（AdminShell）经 createExternalModules() 注册；
 *   本类不是 Pager，故方法都接收 ctx: Pager 来 acquireModule。
 *   渲染见文件底部的 ViewContainer.renderBench(bench, ctx) 扩展。
 */
class TestBenchState {

    var serverUrl by observable("ws://127.0.0.1:8000/api/ws")
    var token by observable("")
    var connected by observable(false)
    var connectionState by observable("idle") // idle / connecting / connected / error
    var recording by observable(false)
    var speaking by observable(false)
    var asrText by observable("")
    var ttsText by observable("你好，小智")
    var ttsLang by observable("auto") // 参考 SenseVoice：auto 自动检测 zh/en/ja/ko/yue
    var ttsSpeed by observable("1.0") // 参考 kokoro.js demo：语速
    var ttsSpeaker by observable("0") // Kokoro 语音角色 sid
    var statusMsg by observable("")

    // ============================================================
    // 桥接：经 Pager 取 XiaoZhiModule
    // ============================================================

    private fun xz(ctx: Pager): XiaoZhiModule = ctx.acquireModule(XiaoZhiModule.MODULE_NAME)

    fun connect(ctx: Pager) {
        connectionState = "connecting"
        statusMsg = "连接中…"
        xz(ctx).connect(serverUrl, token) { result ->
            val ok = result?.optBoolean("success", false) ?: false
            connected = ok
            connectionState = if (ok) "connected" else "error"
            statusMsg = if (ok) "已连接" else "连接失败: ${result?.optString("error", "") ?: ""}"
        }
    }

    fun disconnect(ctx: Pager) {
        xz(ctx).disconnect()
        connected = false
        connectionState = "idle"
        statusMsg = "已断开"
    }

    fun startAsr(ctx: Pager) {
        recording = true
        asrText = ""
        statusMsg = "录音中…"
        xz(ctx).startAsr { result ->
            asrText = result?.optString("text", "") ?: ""
            recording = false
            statusMsg = "识别完成"
        }
    }

    fun stopAsr(ctx: Pager) {
        recording = false
        xz(ctx).stopAsr()
        statusMsg = "已停止，等待识别结果…"
    }

    fun speak(ctx: Pager) {
        if (ttsText.isBlank()) {
            statusMsg = "请输入要合成的文字"
            return
        }
        speaking = true
        statusMsg = "合成中…"
        val speed = ttsSpeed.toDoubleOrNull() ?: 1.0
        val speaker = ttsSpeaker.toIntOrNull() ?: 0
        xz(ctx).speak(ttsText, speaker, ttsLang, speed) { _ ->
            speaking = false
            statusMsg = "TTS 播放完成"
        }
    }

    // ============================================================
    // 渲染：见文件底部 ViewContainer.renderBench(bench, ctx) 扩展
    // ============================================================
}

/**
 * 渲染测试台三个分组卡片（连接 / ASR 识别测试 / TTS 合成测试）。
 * 宽屏（wide()=true）：连接 + ASR 一行两列，TTS（奇数张）独占整行；窄屏：纵向单列。
 *
 * ⚠️ 必须在目标容器（Scroller）闭包内**非限定**调用 `renderBench(bench, ctx, wide)`：
 * Kuikly 的 View{} DSL 静态绑定到词法作用域最近的 ViewContainer 接收者，
 * 以 `ctx.groupedCard(...)` 方式调用会把卡片挂到 Pager 根容器，导致布局逃逸。
 * AdminCard 的 content（ViewBuilder，带接收者）在 groupedCard 卡片容器内执行，节点挂进卡片。
 * ⚠️ wide 须为 lambda（如 `{ pagerData.pageViewWidth >= 900f }`），在 cardGrid 的
 * vif 闭包内读取才能随窗口 resize 响应式重排。
 */
fun ViewContainer<*, *>.renderBench(bench: TestBenchState, ctx: Pager, wide: () -> Boolean) {
    cardGrid(
        wide = wide,
        cards = listOf(
            AdminCard("连接") {
                labeledField("server url", { bench.serverUrl }, { bench.serverUrl = it }, "ws://127.0.0.1:8000/api/ws")
                labeledField("token（可选）", { bench.token }, { bench.token = it })
                actionRow {
                    primaryButton(if (bench.connected) "断开" else "连接") {
                        if (bench.connected) bench.disconnect(ctx) else bench.connect(ctx)
                    }
                    View { attr { width(AdminSpace.md) } }
                    statusBadge(bench.connectionState, connectionLabel(bench.connectionState))
                }
            },
            AdminCard("ASR 识别测试") {
                Text {
                    attr {
                        fontSize(AdminType.caption)
                        color(AdminColors.textSecondary)
                        marginTop(AdminSpace.xs)
                        text("语言由 SenseVoice 自动检测（zh/en/ja/ko/yue）")
                    }
                }
                actionRow {
                    // 录音时禁用 TTS 相关（合成按钮在下方 TTS 组按 recording 控制），
                    // 录音按钮本身在录音时变 danger 红并停止；合成中禁用录音。
                    primaryButton(
                        if (bench.recording) "停止录音" else "开始录音",
                        enabled = !bench.speaking,
                    ) {
                        if (bench.recording) bench.stopAsr(ctx) else bench.startAsr(ctx)
                    }
                }
                View { attr { height(AdminSpace.md) } }
                groupedCard("识别结果", withDivider = false) {
                    if (bench.asrText.isEmpty()) {
                        Text {
                            attr {
                                fontSize(AdminType.body)
                                color(AdminColors.textTertiary)
                                text("识别结果将显示在此")
                            }
                        }
                    } else {
                        Text {
                            attr {
                                fontSize(AdminType.body)
                                color(AdminColors.textPrimary)
                                text(bench.asrText)
                            }
                        }
                        View { attr { height(AdminSpace.sm) } }
                        secondaryButton("复制") { /* 剪贴板需平台模块，从略，见交付备注 */ }
                    }
                }
            },
            AdminCard("TTS 合成测试") {
                labeledField("合成文字", { bench.ttsText }, { bench.ttsText = it }, "输入要合成的文字", height = 100f)
                labeledField("语言 (auto/zh/en/ja/ko/yue)", { bench.ttsLang }, { bench.ttsLang = it })
                labeledField("语速 (0.5~2.0)", { bench.ttsSpeed }, { bench.ttsSpeed = it })
                labeledField("语音角色 speaker (sid)", { bench.ttsSpeaker }, { bench.ttsSpeaker = it })
                actionRow {
                    // 录音中禁用 TTS 按钮
                    primaryButton("合成并播放", enabled = !bench.recording) {
                        bench.speak(ctx)
                    }
                }
            },
        ),
    )
}
