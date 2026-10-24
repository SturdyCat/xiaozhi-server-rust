package com.xiaozhi.admin

import com.xiaozhi.admin.ui.*

import com.tencent.kuikly.core.base.PagerScope
import com.tencent.kuikly.core.nvi.serialization.json.JSONObject
import com.tencent.kuikly.core.pager.Pager
import com.tencent.kuikly.core.timer.setTimeout

/** 配置持久化与预设动作；所有草稿仍由 form 唯一持有。 */
internal class ConfigFormActions(private val form: ConfigFormState, private val scope: PagerScope) {
    // 基线只在加载全局配置后、套用 overlay 前抓取；保存成功后不能推进。
    private var deviceBaseline: JSONObject? = null
    private var saveResultSeq = 0

    /** 拉取配置来源元信息（GET /api/config/meta）→ 展示"配置文件路径 + 是否持久化"。 */
    fun loadConfigMeta(ctx: Pager, baseUrl: String = ""): Unit = with(form) {
        network(ctx).requestGet("${baseUrl}/api/config/meta", JSONObject()) { data, success, _, _ ->
            if (!success) return@requestGet
            val path = data?.optString("config_path", "") ?: ""
            val persistent = data?.optBoolean("persistent", true) ?: true
            val hint = data?.optString("hint", "") ?: ""
            configMetaMsg = when {
                path.isEmpty() -> "配置来源：内置默认（未指定配置文件，保存不会持久化）"
                !persistent -> "配置来源：$path ⚠️ 不在挂载卷上，重建容器会丢失！$hint"
                else -> "配置来源：$path（持久化 ✓）"
            }
        }
    }

    fun load(ctx: Pager, baseUrl: String = ""): Unit = with(form) {
        invalidateReadOnlyTests()
        serverBase = baseUrl
        // 设备个性化目标（P1-5）：全局基线 + overlay 套用，与全局加载分道
        if (editDeviceId.isNotEmpty()) {
            loadDeviceTarget(ctx, baseUrl)
            return@with
        }
        network(ctx).requestGet("${baseUrl}/api/config", JSONObject()) { data, success, errorMsg, _ ->
            if (success) {
                fill(data)
                refreshNodeTests(ctx)
                loadVoices(ctx, baseUrl)
                loadConfigMeta(ctx, baseUrl)
                // 字段热生效元数据（用于"这个字段多久生效"提示；失败静默）
                schema.load(ctx, baseUrl)
                conflict = false
                saveImpactNote = ""
                statusMsg = "已加载配置"
                statusLevel = "ok"
            } else {
                statusMsg = "加载失败: $errorMsg"
                statusLevel = "error"
            }
        }
    }

    /**
     * 加载设备个性化目标：先拉全局配置填表（基线），抓取白名单快照，再拉该设备
     * overlay 套用到表单（只覆盖白名单字段，其余字段保持全局值 → 表单即「设备生效值」）。
     * 保存时按 [deviceBaseline] 求差，只把相对全局改动的白名单字段写回 overlay。
     */
    private fun loadDeviceTarget(ctx: Pager, baseUrl: String): Unit = with(form) {
        val id = editDeviceId
        network(ctx).requestGet("${baseUrl}/api/config", JSONObject()) { data, success, errorMsg, _ ->
            if (!success) {
                statusMsg = "加载失败: $errorMsg"
                statusLevel = "error"
                return@requestGet
            }
            fill(data)
            refreshNodeTests(ctx)
            loadVoices(ctx, baseUrl)
            loadConfigMeta(ctx, baseUrl)
            schema.load(ctx, baseUrl)
            // 全局基线快照（必须在套用 overlay 之前抓）
            deviceBaseline = buildOverlayDraft()
            network(ctx).requestGet("${baseUrl}/api/devices/$id/overlay", JSONObject()) { odata, osuccess, oerr, _ ->
                if (osuccess) {
                    val overlay = odata?.optJSONObject("overlay")
                    val has = overlay != null && overlay.length() > 0
                    if (has) fill(overlay!!) // overlay 只含白名单字段 → fill 缺键保留当前（全局）值
                    conflict = false
                    saveImpactNote = ""
                    dirty = false
                    statusMsg = if (has) {
                        "已加载设备 $id 的个性化配置（改动保存后仅对该设备生效，其余字段沿用全局）"
                    } else {
                        "设备 $id 暂无个性化配置：当前显示全局值，保存后差异字段仅对该设备生效"
                    }
                    statusLevel = "ok"
                } else {
                    statusMsg = "设备 overlay 加载失败: $oerr"
                    statusLevel = "error"
                }
            }
        }
    }

    /** 退出设备编辑，回到全局配置（清基线并重新加载全局）。 */
    fun backToGlobal(ctx: Pager): Unit = with(form) {
        editDeviceId = ""
        deviceBaseline = null
        load(ctx, serverBase)
    }

    /**
     * 清除设备 overlay（恢复完全使用全局配置）并退回全局编辑。
     * Kuikly 无 DELETE：走服务端 POST `.../overlay/delete` 别名（同 OTA 删除惯例）。
     */
    fun clearDeviceOverlay(ctx: Pager, onResult: ((String, String) -> Unit)? = null): Unit = with(form) {
        val id = editDeviceId
        if (id.isEmpty()) return@with
        val headers = JSONObject().apply { put("Content-Type", "application/json") }
        network(ctx).httpRequest("${serverBase}/api/devices/$id/overlay/delete", true, JSONObject(), headers) { data, success, errorMsg, resp ->
            val code = resp.statusCode
            val serverMsg = data?.optString("message", "") ?: ""
            val ok = success && (code == null || code in 200..299)
            val msg = if (ok) {
                serverMsg.ifEmpty { "已清除设备 $id 的个性化配置（恢复使用全局配置）" }
            } else {
                "清除失败: ${serverMsg.ifEmpty { errorMsg.ifEmpty { "HTTP $code" } }}"
            }
            onResult?.invoke(msg, if (ok) "ok" else "error")
            if (ok) backToGlobal(ctx)
        }
    }

    /// 「重新加载」：409 冲突后按服务端现值重填表单（放弃本地草稿）。
    fun reload(ctx: Pager, baseUrl: String = ""): Unit = with(form) {
        dirty = false
        conflict = false
        load(ctx, baseUrl)
    }

    /**
     * 保存配置（POST /api/config），**异步等待接口完成**：
     * - 进入时 [saving]=true → 顶栏按钮切「保存中…」并拦截重复点击（三态按钮第一态）；
     * - 完成后 [saving]=false 且 [saveResult] 置 "ok"/"error" → 按钮短暂显示「已保存 / 保存失败」（第三态）；
     * - [onResult] 在**请求返回之后**回调（msg, level），由页面弹顶部 toast——
     *   ⚠️ 不能在调用处紧随 save() 之后取 statusMsg：那时请求还没回来，拿到的是上一条旧消息。
     */
    fun save(ctx: Pager, baseUrl: String = "", onResult: ((String, String) -> Unit)? = null): Unit = with(form) {
        saving = true
        // 进入"保存中"：清掉上一次的完成态，并作废旧的回落定时器
        saveResult = ""
        saveResultSeq++
        // 设备个性化目标（P1-5）：白名单差量 → 设备 overlay，不走全局 /api/config
        if (editDeviceId.isNotEmpty()) {
            saveDeviceOverlay(ctx, baseUrl, onResult)
            return@with
        }
        val body = buildConfigDraft()
        // ⚠️ 必须显式带 Content-Type: application/json：
        // Kuikly 的 requestPost 默认 headers=null，原生 KRHttpRequestTool 在非 JSON
        // Content-Type 下会把 body 编码成 form-urlencoded —— 服务端 Json<Config> 提取器
        // 直接 415（实测报 "Expected request with `Content-Type: application/json`"，
        // macApp 表现为「保存配置」失败）。同时按 HTTP 状态码判定成功：
        // 传输层 success 且 statusCode 非 2xx（如 415/400）时给出明确错误。
        val headers = JSONObject().apply { put("Content-Type", "application/json") }
        network(ctx).httpRequest("${baseUrl}/api/config", true, body, headers) { data, success, errorMsg, resp ->
            saving = false
            val code = resp.statusCode
            // 服务端返回 JSON {ok, message, config_path}：优先透传 message（含**实际写入路径**
            // 与持久化告警）——"配置怎么没生效/丢哪了"直接可见。
            val serverMsg = data?.optString("message", "") ?: ""
            if (success && (code == null || code in 200..299)) {
                dirty = false
                conflict = false
                lastSavedAt = "已保存"
                // 服务端回传新指纹：无需再 GET 即可继续保存
                revision = data?.optString("revision", revision) ?: revision
                refreshNodeTests(ctx)
                // 三档生效文案由**服务端**判定（字段级 hot 取最严格，见 docs §5.4-6）：
                // 客户端只做翻译，避免"说立即生效、其实要重启"这类误导。
                val hotText = when (data?.optString("hot", "") ?: "") {
                    "live" -> "已保存并立即生效"
                    "next_session" -> "已保存，新会话生效"
                    "restart_only" -> "已保存，下次启动生效"
                    else -> ""
                }
                // 改动影响高亮：把全局 hot 投影到配置 section 头部（仅确有非即时影响时才提示）
                saveImpactNote = when (data?.optString("hot", "") ?: "") {
                    "next_session" -> "本次改动需：新会话生效"
                    "restart_only" -> "本次改动需：重启 server 生效"
                    else -> ""
                }
                statusMsg = if (hotText.isEmpty()) {
                    serverMsg.ifEmpty { "保存成功（新会话生效）" }
                } else {
                    "$hotText。$serverMsg"
                }
                statusLevel = "ok"
                // 三要素已填齐时：保存即自动触发音色探测（目录为空或想更新时最省事；失败不影响保存）
                if (xfyunCredsReady()) {
                    refreshVoices(ctx)
                }
            } else if (code == 409) {
                // 过期写入：**保留草稿**（dirty 不动），顶栏出现「重新加载」入口
                markDirty()
                conflict = true
                saveImpactNote = ""
                statusMsg = serverMsg.ifEmpty { "配置已被其他窗口修改，请重新加载后再保存" }
                statusLevel = "error"
            } else {
                saveImpactNote = ""
                statusMsg = "保存失败: ${serverMsg.ifEmpty { if (errorMsg.isNotEmpty()) errorMsg else "HTTP $code" }}"
                statusLevel = "error"
            }
            // ===== 接口已完成：三态按钮切「完成态」并安排回落，再交给调用方弹顶部 toast =====
            // 完成态直接镜像本次结果级别（三个分支只会是 ok / error），展示 SAVE_RESULT_HOLD_MS 后复位。
            saveResult = statusLevel
            val resultSeq = ++saveResultSeq
            this@ConfigFormActions.scope.setTimeout(SAVE_RESULT_HOLD_MS) { if (resultSeq == saveResultSeq) saveResult = "" }
            onResult?.invoke(statusMsg, statusLevel)
        }
    }

    /**
     * 保存设备个性化 overlay（PUT /api/devices/{id}/overlay 的差量）。
     *
     * - 差量为空 → 清除 overlay（设备恢复完全使用全局配置）；
     * - Kuikly NetworkModule 只有 GET/POST：跟随 OTA 删除惯例走服务端 POST 别名
     *   （POST = PUT、POST `.../overlay/delete` = DELETE）。
     * - 基线在保存成功后**不更新**：增量始终相对全局，连续保存不会把上一次的差量
     *   误判为"无改动"而清掉 overlay（基线只在 [loadDeviceTarget] 抓取 / [backToGlobal] 清除）。
     */
    private fun saveDeviceOverlay(ctx: Pager, baseUrl: String, onResult: ((String, String) -> Unit)?): Unit = with(form) {
        val id = editDeviceId
        val diff = computeOverlayDraftDiff(deviceBaseline)
        val clear = diff.length() == 0
        val url = "${baseUrl}/api/devices/$id/overlay" + if (clear) "/delete" else ""
        val body = if (clear) JSONObject() else diff
        val headers = JSONObject().apply { put("Content-Type", "application/json") }
        network(ctx).httpRequest(url, true, body, headers) { data, success, errorMsg, resp ->
            saving = false
            val code = resp.statusCode
            val serverMsg = data?.optString("message", "") ?: ""
            if (success && (code == null || code in 200..299)) {
                dirty = false
                conflict = false
                lastSavedAt = "已保存"
                statusMsg = serverMsg.ifEmpty {
                    if (clear) "已清除设备 $id 的个性化配置（恢复使用全局配置）"
                    else "设备 $id 的个性化配置已保存（新会话生效）"
                }
                statusLevel = "ok"
            } else {
                statusMsg = "保存失败: ${serverMsg.ifEmpty { if (errorMsg.isNotEmpty()) errorMsg else "HTTP $code" }}"
                statusLevel = "error"
            }
            // 三态按钮完成态 + 回落定时器（与全局保存同一口径）
            saveResult = statusLevel
            val resultSeq = ++saveResultSeq
            this@ConfigFormActions.scope.setTimeout(SAVE_RESULT_HOLD_MS) { if (resultSeq == saveResultSeq) saveResult = "" }
            onResult?.invoke(statusMsg, statusLevel)
        }
    }

    // ============================================================
    // 配置预设（P1-5）：快照当前全局配置 / 应用 / 删除
    // ============================================================
    // 「保存为预设」只上报预设名：服务端快照 SQLite 已保存配置（不是客户端未保存草稿）。
    // 预设仅作用于**全局**配置；设备个性化模式下预设卡不显示。

    /** 拉取预设名列表（GET /api/config/presets；失败静默，列表保持原样）。 */
    fun loadPresets(ctx: Pager, baseUrl: String = ""): Unit = with(form) {
        network(ctx).requestGet("${baseUrl}/api/config/presets", JSONObject()) { data, success, _, _ ->
            if (!success) return@requestGet
            presetNames.clear()
            val arr = data?.optJSONArray("presets") ?: return@requestGet
            for (i in 0 until arr.length()) {
                val n = arr.optString(i) ?: ""
                if (n.isNotEmpty()) presetNames.add(n)
            }
        }
    }

    /** 把 SQLite 已保存的全局配置快照为预设（POST /api/config/presets {name}）。 */
    fun savePreset(ctx: Pager, baseUrl: String = ""): Unit = with(form) {
        val name = presetNameDraft.trim()
        if (name.isEmpty()) {
            presetMsg = "请先填写预设名（字母数字与 - _ .）"
            return@with
        }
        presetBusy = true
        presetMsg = ""
        val body = JSONObject().apply { put("name", name) }
        val headers = JSONObject().apply { put("Content-Type", "application/json") }
        network(ctx).httpRequest("${baseUrl}/api/config/presets", true, body, headers) { data, success, errorMsg, resp ->
            presetBusy = false
            val code = resp.statusCode
            val serverMsg = data?.optString("message", "") ?: ""
            if (success && (code == null || code in 200..299)) {
                presetMsg = serverMsg.ifEmpty { "已把当前配置保存为预设「$name」" }
                presetNameDraft = ""
                loadPresets(ctx, baseUrl)
            } else {
                presetMsg = "保存预设失败: ${serverMsg.ifEmpty { errorMsg.ifEmpty { "HTTP $code" } }}"
            }
        }
    }

    /** 应用预设到全局配置（POST /api/config/presets/{name}/apply），成功后重载表单反映新值。 */
    fun applyPreset(ctx: Pager, baseUrl: String = "", name: String): Unit = with(form) {
        presetBusy = true
        presetMsg = ""
        val headers = JSONObject().apply { put("Content-Type", "application/json") }
        network(ctx).httpRequest("${baseUrl}/api/config/presets/$name/apply", true, JSONObject(), headers) { data, success, errorMsg, resp ->
            presetBusy = false
            val code = resp.statusCode
            val serverMsg = data?.optString("message", "") ?: ""
            if (success && (code == null || code in 200..299)) {
                presetMsg = serverMsg.ifEmpty { "已应用预设「$name」（新会话生效）" }
                // 全局配置已被预设覆盖：重载表单（预设仅作用于全局；设备目标下预设卡不显示）
                load(ctx, baseUrl)
            } else {
                presetMsg = "应用预设失败: ${serverMsg.ifEmpty { errorMsg.ifEmpty { "HTTP $code" } }}"
            }
        }
    }

    /** 删除预设（POST /api/config/presets/{name}/delete 别名；Kuikly 无 DELETE）。 */
    fun deletePreset(ctx: Pager, baseUrl: String = "", name: String): Unit = with(form) {
        presetBusy = true
        presetMsg = ""
        network(ctx).requestPost("${baseUrl}/api/config/presets/$name/delete", JSONObject()) { data, success, errorMsg, resp ->
            presetBusy = false
            val code = resp.statusCode
            val serverMsg = data?.optString("message", "") ?: ""
            if (success && (code == null || code in 200..299)) {
                presetMsg = serverMsg.ifEmpty { "已删除预设「$name」" }
                loadPresets(ctx, baseUrl)
            } else {
                presetMsg = "删除预设失败: ${serverMsg.ifEmpty { errorMsg.ifEmpty { "HTTP $code" } }}"
            }
        }
    }
}

private const val SAVE_RESULT_HOLD_MS = 2000
