package com.xiaozhi.admin

import com.xiaozhi.admin.ui.*

import com.tencent.kuikly.core.nvi.serialization.json.JSONObject
import com.tencent.kuikly.core.pager.Pager
import com.tencent.kuikly.core.reactive.collection.ObservableList
import com.tencent.kuikly.core.timer.setTimeout

// 目录只操作 ConfigFormState 的原 observableList；不复制当前选择或配置密钥为状态。
/** Transient request closure only; never serialized, rendered, logged or saved as form state. */
private class CatalogueRequestScope(
    val base: String,
    val catalogueEpoch: Long,
    val requestToken: String,
    val accessRevision: Long,
)

private fun ConfigFormState.captureCatalogueScope(ctx: Pager) = CatalogueRequestScope(
    serverBase, catalogueScopeEpoch, ctx.adminRequestToken(), ctx.adminAccessRevision(),
)

private fun ConfigFormState.isCatalogueScopeCurrent(ctx: Pager, requestScope: CatalogueRequestScope): Boolean =
    serverBase == requestScope.base && catalogueScopeEpoch == requestScope.catalogueEpoch &&
        ctx.isAdminAccessCurrent(requestScope.accessRevision, requestScope.requestToken)

internal fun ConfigFormState.fetchProviderLlmModels(ctx: Pager) {
    if (llmModelsBusy) return
    llmModelsBusy = true
    llmModelsMsg = ""
    val body = JSONObject()
    if (llmApiBase.isNotBlank()) body.put("api_base", llmApiBase.trim())
    if (llmApiKey.isNotBlank()) body.put("api_key", llmApiKey.trim())
    val requestScope = captureCatalogueScope(ctx)
    val headers = adminRequestHeaders(requestScope.requestToken)
    network(ctx).httpRequest("${requestScope.base}/api/test/llm/models", true, body, headers) { data, success, errorMsg, resp ->
        // Scope invalidation releases the old busy flag. An old callback cannot release
        // a fresh scope's request, even after generic base or credential A→B→A changes.
        if (!isCatalogueScopeCurrent(ctx, requestScope)) return@httpRequest
        llmModelsBusy = false
        val ok = success && (resp.statusCode == null || resp.statusCode in 200..299)
        val state = data?.optString("state", "") ?: ""
        val err = data?.optString("error", "")?.takeIf { it.isNotEmpty() } ?: (errorMsg ?: "")
        if (!ok || state != "ok") {
            llmModelOptions.clear()
            llmModelsMsg = "获取失败：${err.ifEmpty { "HTTP ${resp.statusCode}" }}"
            return@httpRequest
        }
        val arr = data?.optJSONArray("models")
        llmModelOptions.clear()
        val n = arr?.length() ?: 0
        for (i in 0 until n) {
            val m = arr?.optString(i, "") ?: ""
            if (m.isNotEmpty()) llmModelOptions.add(m to m)
        }
        llmModelsMsg = if (llmModelOptions.isEmpty()) {
            "上游未返回模型（检查 api_base 是否为 OpenAI 兼容端点）"
        } else {
            "✓ ${llmModelOptions.size} 个模型，可在下方下拉选择或直接手输"
        }
    }
}

internal fun ConfigFormState.initializeProviderCatalogue() {
    ttsModeOptions.addAll(
        listOf(
            "local" to "本地模型（离线合成）",
            "remote" to "远程服务（在线 API）",
        ),
    )
    ttsLocalEngines.addAll(listOf("kokoro" to "本地 Kokoro INT8（离线）"))
    xfyunVoiceGroupOptions.addAll(
        listOf(
            "female" to "女声",
            "male" to "男声",
            "custom" to "自定义（手填 vcn）",
        ),
    )
    xfyunVoiceTypeOptions.addAll(
        listOf(
            "all" to "全部发音人",
            "classic" to "普通发音人",
            "x6" to "极速拟人",
        ),
    )
    reloadXfyunVoiceOptions()
    ttsRemoteEngines.addAll(
        listOf(
            "xfyun" to "科大讯飞（在线）",
            // 未来供应商在此追加：如 "azure" to "Azure TTS（在线）"
        ),
    )
}

/** 按性别分组 + 音色类型筛选重建下拉选项（列表来自 /api/tts/voices 拉取结果）。 */
internal fun ConfigFormState.reloadProviderVoiceOptions() {
    xfyunVoiceOptions.clear()
    if (xfyunVoiceGroup == "custom") return
    val classic = if (xfyunVoiceGroup == "male") voicesMaleClassic else voicesFemaleClassic
    val x6 = if (xfyunVoiceGroup == "male") voicesMaleX6 else voicesFemaleX6
    when (xfyunVoiceType) {
        "classic" -> xfyunVoiceOptions.addAll(classic)
        "x6" -> xfyunVoiceOptions.addAll(x6)
        else -> {
            xfyunVoiceOptions.addAll(classic)
            xfyunVoiceOptions.addAll(x6)
        }
    }
}

/** 切换音色类型筛选：若当前 vcn 不在筛选结果里则回落第一个。 */
internal fun ConfigFormState.selectProviderVoiceType(t: String) {
    xfyunVoiceType = t
    reloadXfyunVoiceOptions()
    val ids = xfyunVoiceOptions.map { it.first }
    if (xfyunVoice !in ids) {
        xfyunVoice = ids.firstOrNull() ?: xfyunVoice
        markDirty()
    }
}

/** 切换音色分组：非自定义分组时若当前 vcn 不在该组，回落到该组第一个。 */
internal fun ConfigFormState.selectProviderVoiceGroup(g: String) {
    xfyunVoiceGroup = g
    reloadXfyunVoiceOptions()
    if (g != "custom") {
        val ids = xfyunVoiceOptions.map { it.first }
        if (xfyunVoice !in ids) {
            xfyunVoice = ids.firstOrNull() ?: xfyunVoice
            markDirty()
        }
    }
}

/** 依已拉取目录反推当前 vcn 的性别分组与音色类型；目录未拉取时按前缀启发。 */
internal fun ConfigFormState.deriveProviderVoiceGroupAndType() {
    val v = xfyunVoice
    val inList: (ObservableList<Pair<String, String>>) -> Boolean = { l -> l.any { it.first == v } }
    when {
        inList(voicesFemaleClassic) -> { xfyunVoiceGroup = "female"; xfyunVoiceType = "classic" }
        inList(voicesFemaleX6) -> { xfyunVoiceGroup = "female"; xfyunVoiceType = "x6" }
        inList(voicesMaleClassic) -> { xfyunVoiceGroup = "male"; xfyunVoiceType = "classic" }
        inList(voicesMaleX6) -> { xfyunVoiceGroup = "male"; xfyunVoiceType = "x6" }
        !voicesLoaded -> {
            // 目录未就绪：前缀启发，避免把极速拟人音色误落「自定义」
            if (v.startsWith("x5_") || v.startsWith("x6_")) {
                xfyunVoiceGroup = "female"; xfyunVoiceType = "x6"
            }
        }
        else -> xfyunVoiceGroup = "custom"
    }
}

/**
 * 触发服务端**真实探测**发音人目录（POST /api/tts/voices/refresh，异步后台任务：
 * 对候选池逐项调用合成 API 探测可用性），随后轮询 `GET /api/tts/voices` 直到
 * `probing=false` 再刷新下拉。需先保存三要素（音色列表由真实 API 探测生成）。
 */
internal fun ConfigFormState.refreshProviderVoices(ctx: Pager) {
    val requestScope = captureCatalogueScope(ctx)
    voicesRefreshMsg = "探测中…（对候选音色逐个真实合成验证，约需 1 分钟）"
    val headers = adminRequestHeaders(requestScope.requestToken)
    network(ctx).httpRequest("${requestScope.base}/api/tts/voices/refresh", true, JSONObject(), headers) { _, success, errorMsg, resp ->
        if (!isCatalogueScopeCurrent(ctx, requestScope)) return@httpRequest
        val code = resp.statusCode
        if (success && (code == null || code in 200..299)) {
            pollProviderVoices(ctx, attempts = 0, requestScope = requestScope)
        } else {
            voicesRefreshMsg = "探测启动失败: ${if (errorMsg.isNotEmpty()) errorMsg else "HTTP $code"}"
        }
    }
}

/** 同一个捕获作用域贯穿 POST→GET→timer→最终目录读取；失效只丢弃回包，不取消远端任务。 */
private fun ConfigFormState.pollProviderVoices(ctx: Pager, attempts: Int, requestScope: CatalogueRequestScope) {
    if (!isCatalogueScopeCurrent(ctx, requestScope)) return
    if (attempts > 90) {
        voicesRefreshMsg = "探测超时（请查看服务端日志）"
        return
    }
    val headers = adminRequestHeaders(requestScope.requestToken)
    network(ctx).httpRequest("${requestScope.base}/api/tts/voices", false, JSONObject(), headers) { data, success, _, _ ->
        if (!isCatalogueScopeCurrent(ctx, requestScope)) return@httpRequest
        if (success && !data.optBoolean("probing", false)) {
            loadCapturedProviderVoices(ctx, requestScope)
        } else {
            ctx.setTimeout(2000) {
                if (isCatalogueScopeCurrent(ctx, requestScope)) {
                    pollProviderVoices(ctx, attempts + 1, requestScope)
                }
            }
        }
    }
}

/** 拉取服务端音色目录（GET /api/tts/voices）并按性别×类型重建下拉。 */
internal fun ConfigFormState.loadProviderVoices(ctx: Pager, baseUrl: String = "") {
    serverBase = baseUrl // 保留连接流程的基址写入；先完成失效，再捕获新作用域。
    loadCapturedProviderVoices(ctx, captureCatalogueScope(ctx))
}

private fun ConfigFormState.loadCapturedProviderVoices(ctx: Pager, requestScope: CatalogueRequestScope) {
    if (!isCatalogueScopeCurrent(ctx, requestScope)) return
    val headers = adminRequestHeaders(requestScope.requestToken)
    network(ctx).httpRequest("${requestScope.base}/api/tts/voices", false, JSONObject(), headers) { data, success, _, _ ->
        if (!isCatalogueScopeCurrent(ctx, requestScope)) return@httpRequest
        if (!success) return@httpRequest
        val arr = data.optJSONArray("voices") ?: return@httpRequest
        voicesFemaleClassic.clear()
        voicesFemaleX6.clear()
        voicesMaleClassic.clear()
        voicesMaleX6.clear()
        for (i in 0 until arr.length()) {
            val v = arr.optJSONObject(i) ?: continue
            val vcn = v.optString("vcn")
            val name = v.optString("name")
            val tag = v.optString("tag")
            val label = if (tag.isEmpty()) name else "$name（$tag）"
            val type = v.optString("type")
            val pair = vcn to label
            when (v.optString("gender") to type) {
                "female" to "classic" -> voicesFemaleClassic.add(pair)
                "female" to "x6" -> voicesFemaleX6.add(pair)
                "male" to "classic" -> voicesMaleClassic.add(pair)
                else -> voicesMaleX6.add(pair)
            }
        }
        voicesLoaded = true
        // 目录到位后反推当前配置音色的分组与类型（fill 时目录可能尚未拉取）
        deriveVoiceGroupAndType()
        reloadXfyunVoiceOptions()
        // 空目录（尚未探测过）：给出可行动指引（非错误，避免误以为服务端坏了）
        if (arr.length() == 0) {
            voicesRefreshMsg = "音色列表为空：保存三要素后点「探测音色目录」（服务端会对候选音色逐个真实合成验证）"
        }
    }
}
