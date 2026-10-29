package com.xiaozhi.admin

import com.xiaozhi.admin.contracts.*

import com.tencent.kuikly.core.nvi.serialization.json.JSONObject
import com.tencent.kuikly.core.pager.Pager

/** 只读测试请求；不是 BLE 操作保护，不改变远端已发送请求的执行状态。 */
internal class ConfigFormTests(private val form: ConfigFormState) {
    private val gate = NodeRequestGate()
    private val credentialGate = NodeRequestGate()

    // base / target / saved revision setter 推进代际；切回同一目标也不接受旧代回包。
    // 管理鉴权仍走既有请求契约；expectedToken 是配置草稿，不用作请求 Bearer。
    fun invalidate() {
        gate.invalidate()
        form.testBusyLlm = false
        form.testBusyTts = false
        form.testBusyVad = false
        form.testMsgLlm = ""
        form.testMsgTts = ""
        form.testMsgVad = ""
        invalidateCredentials()
    }

    // 仅凭据/音色草稿变化时推进自己的 epoch，不重置无关节点测试；ticket 不保存密钥。
    fun invalidateCredentials() {
        credentialGate.invalidate()
        form.credsTestMsg = ""
        form.credsTestOk = false
    }

    /**
     * 内联配置测试：服务端按 SQLite 已保存配置快照 ad-hoc 构建引擎（不是未保存草稿），
     * Web 也可直接测 LLM 连通性 / TTS 与 VAD 延迟。结果行内展示首包延迟与总耗时。
     */
    fun runInlineTest(ctx: Pager, kind: String): Unit = with(form) {
        if (kind !in listOf("llm", "tts", "vad")) return@with
        val busy = when (kind) {
            "llm" -> testBusyLlm
            "tts" -> testBusyTts
            else -> testBusyVad
        }
        if (busy) return@with
        val ticket = gate.begin(kind) ?: return@with
        when (kind) {
            "llm" -> { testBusyLlm = true; testMsgLlm = "" }
            "tts" -> { testBusyTts = true; testMsgTts = "" }
            else -> { testBusyVad = true; testMsgVad = "" }
        }
        val body = JSONObject().apply { put("text", "你好") }
        val accessRevision = ctx.adminAccessRevision()
        val requestToken = ctx.adminRequestToken()
        val headers = adminRequestHeaders(requestToken)
        network(ctx).httpRequest("$serverBase/api/test/$kind", true, body, headers) { data, success, errorMsg, resp ->
            if (!gate.finish(ticket)) return@httpRequest
            if (!ctx.isAdminAccessCurrent(accessRevision, requestToken)) {
                when (kind) {
                    "llm" -> testBusyLlm = false
                    "tts" -> testBusyTts = false
                    else -> testBusyVad = false
                }
                return@httpRequest
            }
            val ok = success && (resp.statusCode == null || resp.statusCode in 200..299)
            val state = data?.optString("state", "") ?: ""
            val first = data?.optLong("first_packet_ms", -1) ?: -1
            val elapsed = data?.optLong("elapsed_ms", -1) ?: -1
            val err = data?.optString("error", "")?.takeIf { it.isNotEmpty() } ?: (errorMsg ?: "")
            val line = when {
                !ok -> "测试失败：${err.ifEmpty { "HTTP ${resp.statusCode}" }}"
                state == "ok" -> buildString {
                    append("✓ 通过")
                    if (first >= 0) append(" · 首包 ${first}ms")
                    if (elapsed >= 0) append(" · 总耗时 ${elapsed}ms")
                }
                else -> "不可用：$err"
            }
            when (kind) {
                "llm" -> { testBusyLlm = false; testMsgLlm = line }
                "tts" -> { testBusyTts = false; testMsgTts = line }
                else -> { testBusyVad = false; testMsgVad = line }
            }
        }
    }

    /**
     * 测试当前表单里的讯飞三要素（POST /api/tts/voices/test-credentials，**只测不存**）：
     * 服务端用传入凭据真实发起一次短合成，验证密钥/服务/发音人是否可用。
     * 结果以提示显示在按钮旁（成功含耗时；失败含可行动原因，如 11200 发音人未授权）。
     */
    fun testXfyunCreds(ctx: Pager): Unit = with(form) {
        if (!xfyunCredsReady()) {
            credsTestOk = false
            credsTestMsg = "请先填写 app_id / api_key / api_secret"
            return@with
        }
        val ticket = credentialGate.begin("xfyun-credentials") ?: return@with
        credsTestMsg = "测试中…"
        credsTestOk = false
        val body = JSONObject().apply {
            put("app_id", xfyunAppId.trim())
            put("api_key", xfyunApiKey.trim())
            put("api_secret", xfyunApiSecret.trim())
            put("voice", xfyunVoice.trim())
        }
        val accessRevision = ctx.adminAccessRevision()
        val requestToken = ctx.adminRequestToken()
        val headers = adminRequestHeaders(requestToken)
        network(ctx).httpRequest("${serverBase}/api/tts/voices/test-credentials", true, body, headers) { data, success, errorMsg, resp ->
            if (!credentialGate.finish(ticket)) return@httpRequest
            if (!ctx.isAdminAccessCurrent(accessRevision, requestToken)) {
                credsTestMsg = ""
                credsTestOk = false
                return@httpRequest
            }
            val code = resp.statusCode
            if (success && (code == null || code in 200..299)) {
                val ok = data?.optBoolean("ok", false) ?: false
                credsTestOk = ok
                credsTestMsg = if (ok) {
                    "凭据可用（合成耗时 ${data?.optLong("elapsed_ms") ?: 0}ms）"
                } else {
                    "凭据不可用: ${data?.optString("error", "未知错误")}"
                }
            } else {
                credsTestOk = false
                credsTestMsg = "测试失败: ${if (errorMsg.isNotEmpty()) errorMsg else "HTTP $code"}"
            }
        }
    }
}
