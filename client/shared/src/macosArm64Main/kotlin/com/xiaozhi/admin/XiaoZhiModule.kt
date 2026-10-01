package com.xiaozhi.admin

import com.tencent.kuikly.core.module.CallbackFn
import com.tencent.kuikly.core.module.Module
import com.tencent.kuikly.core.nvi.serialization.json.JSONObject

/**
 * XiaoZhi 测试 Module（macOS 专用，仅在 macosMain 编译）。
 *
 * 通过 Kuikly Module 机制桥接原生（macOS）侧实现的 [XiaoZhiModule]（OpenKuiklyIOSRender 下
 * 同名 KRBaseModule 子类），负责：
 * - 与 server 的 WebSocket 测试协议建连 / 收发（hello / asr_test / tts_test）
 * - 麦克风采集（Opus 编码后上行）与 TTS 音频播放
 *
 * 原生实现见 client/apps/macosApp/XiaoZhiModule.m。web/Android/iOS 不注册该模块，
 * 故 web 包根本不含测试功能（符合「仅 Mac App 使用测试」的约束）。
 */
class XiaoZhiModule : Module() {

    override fun moduleName(): String = MODULE_NAME

    /** 连接 server 并发送 hello。url 形如 ws://127.0.0.1:8000/api/ws */
    fun connect(url: String, token: String, callback: CallbackFn? = null) {
        toNative(
            keepCallbackAlive = false,
            methodName = "connect",
            param = JSONObject().apply {
                put("url", url)
                put("token", token)
            },
            callback = callback,
            syncCall = false,
        )
    }

    fun disconnect(callback: CallbackFn? = null) {
        toNative(
            keepCallbackAlive = false,
            methodName = "disconnect",
            param = null,
            callback = callback,
            syncCall = false,
        )
    }

    /**
     * 开始 ASR 测试录音。keepCallbackAlive=true：原生在 server 返回 stt 时再次回调（one-shot）。
     * 录音同时在原生侧累积缓冲，停止后可试听（stopRecording → playRecording）。
     */
    fun startAsr(callback: CallbackFn? = null) {
        toNative(
            keepCallbackAlive = true,
            methodName = "startAsr",
            param = null,
            callback = callback,
            syncCall = false,
        )
    }

    /** 仅停止采集（保留录音缓冲供试听/波形），不发 asr_test stop。回调 {seconds}。 */
    fun stopRecording(callback: CallbackFn? = null) {
        toNative(
            keepCallbackAlive = false,
            methodName = "stopRecording",
            param = null,
            callback = callback,
            syncCall = false,
        )
    }

    /** 发送 asr_test stop 触发服务端识别；识别结果经 startAsr 的 keepCallback 回调（stt）。 */
    fun sendAsr() {
        toNative(
            keepCallbackAlive = false,
            methodName = "sendAsr",
            param = null,
            callback = null,
            syncCall = false,
        )
    }

    /** 从头试听录音缓冲（原生 24k PCM）。回调 {duration}。 */
    fun playRecording(callback: CallbackFn? = null) {
        toNative(
            keepCallbackAlive = false,
            methodName = "playRecording",
            param = null,
            callback = callback,
            syncCall = false,
        )
    }

    /** 从头试听 TTS 合成缓冲。回调 {duration}。 */
    fun playTts(callback: CallbackFn? = null) {
        toNative(
            keepCallbackAlive = false,
            methodName = "playTts",
            param = null,
            callback = callback,
            syncCall = false,
        )
    }

    /** 停止试听播放。 */
    fun stopPlayback() {
        toNative(
            keepCallbackAlive = false,
            methodName = "stopPlayback",
            param = null,
            callback = null,
            syncCall = false,
        )
    }

    /**
     * 查询音频状态（试听进度可视化用，UI 100ms 轮询）。
     * @param source "recording" | "tts"
     * 回调 {wave: [64 桶峰值 0~1], duration, playing, position}
     */
    fun getAudioState(source: String, callback: CallbackFn? = null) {
        toNative(
            keepCallbackAlive = false,
            methodName = "getAudioState",
            param = JSONObject().apply { put("source", source) },
            callback = callback,
            syncCall = false,
        )
    }

    /** TTS 合成测试：text 必填，speaker/lang/speed 可空（留默认）。 */
    fun speak(text: String, speaker: Int, lang: String, speed: Double, callback: CallbackFn? = null) {
        toNative(
            keepCallbackAlive = false,
            methodName = "speak",
            param = JSONObject().apply {
                put("text", text)
                put("speaker", speaker)
                put("lang", lang)
                put("speed", speed)
            },
            callback = callback,
            syncCall = false,
        )
    }

    companion object {
        const val MODULE_NAME = "XiaoZhiModule"
    }
}
