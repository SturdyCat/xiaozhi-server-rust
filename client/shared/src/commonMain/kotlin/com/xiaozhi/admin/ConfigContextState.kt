package com.xiaozhi.admin


import com.tencent.kuikly.core.base.PagerScope
import com.tencent.kuikly.core.nvi.serialization.json.JSONObject
import com.tencent.kuikly.core.pager.Pager
import com.xiaozhi.admin.state.stateVar
import com.xiaozhi.admin.state.stateListOf

/**
 * 「灵魂」（`[soul]`）与「记忆」（`[memory]`）的表单状态 + 记忆动作。
 *
 * 为什么单独一个类（而不是继续往 `ConfigFormState` 里加字段）：
 * `AGENTS.md` §5.10 要求逻辑文件 ≤ 600 行，而这两个段合计约 40 个字段；
 * 拆成 `form.sm.*` 既守住规模约定，也让"上下文生产者"这一组概念在代码里成组出现。
 * 渲染见 `ConfigSoulMemoryCards.kt`；**JSON 往返**（`soulJson` / `putJson` / `fill`）在
 * `ConfigContextJson.kt`——本文件只保留字段声明、下拉选项与网络动作，两边共享同一个
 * `ContextSectionState` 实例（`form.sm`），拆的是文件、不是状态。
 *
 * ⚠️ 数组字段（traits/values/boundaries…）在表单里是**多行文本**（每行一条），
 * 读写时统一转换：写入按行拆分成 JSON 数组，读取按 \n 拼回。
 */
class ContextSectionState(private val scope: PagerScope) {

    // ===== [soul] 人格档案 =====
    var soulEnabled by stateVar("true")
    /// 内置人格预设（默认灵魂）：留空的字段由它补，填了的字段逐字段覆盖它；`none` = 完全自定义。
    var soulPreset by stateVar("xiaozhi")
    /// 预设/预览动作的反馈（载入结果、预览摘要、错误）。
    var soulPresetMsg by stateVar("")
    /// 「预览最终提示词」的正文（`POST /api/soul/preview` 的 `instructions`）。
    var soulPreviewText by stateVar("")
    var soulName by stateVar("小智")
    var soulSelfIntro by stateVar("")
    var soulForm by stateVar("")
    var soulAgeFeel by stateVar("")
    var soulWorldview by stateVar("")
    var soulBackstory by stateVar("")
    var soulRelationshipOrigin by stateVar("")
    /// 性格：每行一条「标签：遇到 X 会怎么做」。
    var soulTraits by stateVar("")
    var soulValues by stateVar("")
    var soulBoundaries by stateVar("")
    var soulTone by stateVar("")
    var soulColloquial by stateVar("true")
    var soulAddressUser by stateVar("你")
    var soulCatchphrases by stateVar("")
    /// ⚠️ 语音默认关：TTS 会把 emoji 念出来。
    var soulEmoji by stateVar("false")
    var soulMaxSentences by stateVar("2")
    var soulMaxChars by stateVar("40")
    var soulScenario by stateVar("")
    var soulDeviceHint by stateVar("")
    /// 风格范例：每条一组 Q/A（多行）。
    var soulExamples by stateVar("")

    // ===== [memory] 图记忆 =====
    var memoryEnabled by stateVar("true")
    var memoryBackend by stateVar("graph")
    var memoryDbPath by stateVar("/data/data.db")
    var memoryDbBusyTimeoutMs by stateVar("5000")
    var memoryRecallEnabled by stateVar("true")
    var memoryExtractionEnabled by stateVar("true")
    var memoryFreshTurnCount by stateVar("5")
    var memoryRecallMaxNodes by stateVar("6")
    var memoryRecallMaxTokens by stateVar("400")
    var memoryRecallBudgetMs by stateVar("300")
    var memoryMaintenanceInterval by stateVar("6")
    var memoryInjectPosition by stateVar("after_history")
    var memoryQuarantineMaxAttempts by stateVar("3")
    var memoryScopePrefix by stateVar("xiaozhi")

    var memoryRetentionKeep by stateVar("all")
    var memoryRetentionRecentTurns by stateVar("0")
    var memoryRetentionDays by stateVar("0")
    var memoryRetentionDryRun by stateVar("false")

    var memoryExtractorApiBase by stateVar("")
    var memoryExtractorApiKey by stateVar("")
    /// 密钥 presence（服务端打码，只回 `has_api_key`）；留空 = 不修改盘上已存值。
    var memoryExtractorApiKeyConfigured by stateVar(false)
    var memoryExtractorModel by stateVar("")
    var memoryExtractorTemperature by stateVar("0.1")

    // ===== [memory.rag] 本地文档 RAG（P1-9，仅 engine=rag 时生效）=====
    var memoryRagDocsDir by stateVar("/data/rag-docs")
    var memoryRagChunkSize by stateVar("512")
    var memoryRagChunkOverlap by stateVar("64")
    var memoryRagTopK by stateVar("4")
    /// 「重建索引」动作的状态消息（构建中 / 完成提示）。
    var memoryRagReindexMsg by stateVar("")

    // ===== 记忆动作的 UI 状态（状态卡 / 测试召回 / 维护 / 清空）=====
    var memoryStatusMsg by stateVar("")
    var memoryTestText by stateVar("")
    /// 试召回的作用域（留空 = 跨全部设备检索）。
    var memoryTestScope by stateVar("")
    var memoryTestMsg by stateVar("")
    var memoryTestOk by stateVar(false)
    var memoryMaintainMsg by stateVar("")
    var memoryClearScope by stateVar("")
    var memoryClearAll by stateVar(false)
    /// 两段式强确认：第一次点击只置位并提示，第二次才真正执行（避免误删长期记忆）。
    var memoryClearConfirm by stateVar(false)
    var memoryClearMsg by stateVar("")

    // 下拉选项（可变列表；T05 起为 Compose 快照列表）
    val soulPresetOptions: MutableList<Pair<String, String>> = stateListOf()
    val memoryBackendOptions: MutableList<Pair<String, String>> = stateListOf()
    val memoryInjectPositionOptions: MutableList<Pair<String, String>> = stateListOf()
    val retentionKeepOptions: MutableList<Pair<String, String>> = stateListOf()

    init {
        // 与服务端 `presets::PRESETS` 对应；`loadDefaultSoul` 会按服务端回答补上未知项
        // （新增服务端预设时客户端不必同步发版，但下拉文案仍以本表为准）
        soulPresetOptions.addAll(
            listOf(
                "xiaozhi" to "小智（内置默认人格）",
                "none" to "不使用预设（完全自定义）",
            ),
        )
        memoryBackendOptions.addAll(
            listOf(
                "graph" to "内置图记忆（本地 SQLite）",
                "store" to "纯存储（只存原文，先积累再决定）",
                "rag" to "本地文档 RAG（扫描目录建索引）",
                "none" to "不生效（保留配置）",
            ),
        )
        memoryInjectPositionOptions.addAll(
            listOf(
                "after_history" to "历史之后（默认，护住前缀缓存）",
                "before_history" to "历史之前（离当前问题更近）",
            ),
        )
        retentionKeepOptions.addAll(
            listOf(
                "all" to "全部保留（默认）",
                "referenced" to "只留被记忆引用的原文",
                "recent" to "最近 N 轮之外的未引用原文",
            ),
        )
    }

    // ============================================================
    // 灵魂动作（内置默认灵魂的载入 / 清空 / 预览）
    // ============================================================
    // 设计依据：人格是"看不见的"——不把最终提示词和字段来源摊开，用户就只会得到
    // "配了没用 / 不敢改"的结论（与记忆的「测试召回」同一个道理）。
    // 合并语义（留空=用预设、填写=覆盖）**只实现在服务端**（`soul::effective`），
    // 客户端只做两件事：把当前草稿发过去、把回答显示出来——不在这里复制一份合并规则。

    /**
     * `POST /api/soul/preview`（草稿 = 当前表单），把**最终提示词**展开到界面。
     *
     * 顺带承担"保存前校验器"的角色：preset 拼错、变量写错都会在这里直接报出来。
     */
    fun previewSoul(ctx: Pager, baseUrl: String) {
        soulPresetMsg = "生成预览…"
        soulPreviewText = ""
        val body = JSONObject().apply { put("soul", soulJson()) }
        network(ctx).httpRequest("${baseUrl}/api/soul/preview", true, body, jsonHeaders()) { data, success, errorMsg, resp ->
            val code = resp.statusCode
            if (!success || (code != null && code !in 200..299)) {
                soulPresetMsg = "预览失败: ${serverMessage(data, errorMsg, code)}"
                return@httpRequest
            }
            val text = data?.optString("instructions", "") ?: ""
            val sb = StringBuilder()
            val source = data?.optString("source", "") ?: ""
            sb.append(if (source == "soul") "来源：人格档案" else "来源：[llm].system_prompt（人格未启用）")
            val presetName = data?.optString("preset_name", "") ?: ""
            if (presetName.isNotEmpty()) sb.append(" · 预设：").append(presetName)
            sb.append("\n").append(data?.optInt("chars", 0) ?: 0).append(" 字符 · 约 ")
            sb.append(data?.optLong("approx_tokens", 0L) ?: 0L).append(" tokens")
            val from = joinNames(data?.optJSONArray("from_preset"))
            if (from.isNotEmpty()) sb.append("\n来自预设（你留空的字段）：").append(from)
            val over = joinNames(data?.optJSONArray("overridden"))
            if (over.isNotEmpty()) sb.append("\n你已覆盖的字段：").append(over)
            soulPresetMsg = sb.toString()
            soulPreviewText = if (text.length > 2000) text.substring(0, 2000) + "\n…（已截断）" else text
        }
    }

    /**
     * 「载入预设内容到表单」：把服务端合并出的**完整档案**写回表单，于是"默认人格"变成
     * 可逐条修改的显式值（此前它只存在于运行时合并结果里）。
     *
     * 草稿 = 当前表单（含开关/数值），因此不会冲掉你已经调过的项：
     * 非空字段本来就会覆盖预设，空字段才被预设填上。
     */
    fun loadDefaultSoul(ctx: Pager, baseUrl: String, onChanged: () -> Unit) {
        if (soulPreset == "none") {
            soulPresetMsg = "当前预设为「不使用预设（完全自定义）」，没有可载入的内容；" +
                "想用内置默认人格请先把预设改成「小智」。"
            return
        }
        soulPresetMsg = "载入中…"
        val body = JSONObject().apply { put("soul", soulJson()) }
        network(ctx).httpRequest("${baseUrl}/api/soul/preview", true, body, jsonHeaders()) { data, success, errorMsg, resp ->
            val code = resp.statusCode
            if (!success || (code != null && code !in 200..299)) {
                soulPresetMsg = "载入失败: ${serverMessage(data, errorMsg, code)}"
                return@httpRequest
            }
            val eff = data?.optJSONObject("effective")
            if (eff == null) {
                soulPresetMsg = "载入失败：服务端未返回 effective 档案（版本不匹配？）"
                return@httpRequest
            }
            // ⚠️ 必须整段喂 `fill`：它按"服务端回包"语义映射（数组键按数组处理），
            // 手写部分字段会在缺失的数组键上把用户原文冲成空。
            fill(JSONObject().apply { put("soul", eff) })
            onChanged()
            val name = data.optString("preset_name", soulPreset)
            soulPresetMsg = "已把「$name」的人格内容展开到表单（可直接改，改完点保存）。" +
                "留空的字段运行时会自动用预设补。"
        }
    }

    /**
     * 清空人格填写字段 → 回到"只用预设"的状态（留空 = 运行时由预设补）。
     *
     * 连同 `name`/`address_user` 一起清空是有意的：预设提供它们；
     * 若 preset 为 `none`，提示里会说明必须自己填回去（服务端校验也会报出怎么修）。
     */
    fun clearSoulProfile(onChanged: () -> Unit) {
        soulName = ""
        soulAddressUser = ""
        soulSelfIntro = ""
        soulForm = ""
        soulAgeFeel = ""
        soulWorldview = ""
        soulBackstory = ""
        soulRelationshipOrigin = ""
        soulTraits = ""
        soulValues = ""
        soulBoundaries = ""
        soulTone = ""
        soulCatchphrases = ""
        soulScenario = ""
        soulDeviceHint = ""
        soulExamples = ""
        soulPreviewText = ""
        onChanged()
        soulPresetMsg = if (soulPreset == "none") {
            "已清空人格字段。当前预设是「不使用预设」，因此 name / address_user 必须自己填，否则保存后人格不生效。"
        } else {
            "已清空人格字段：留空的字段运行时会自动用预设「$soulPreset」补。"
        }
    }

    /** `GET /api/soul/presets`：把服务端预设补进下拉（新增服务端预设时客户端无需发版）。 */
    fun syncSoulPresets(ctx: Pager, baseUrl: String) {
        network(ctx).requestGet("${baseUrl}/api/soul/presets", JSONObject()) { data, success, _, _ ->
            if (!success) return@requestGet
            val arr = data.optJSONArray("presets") ?: return@requestGet
            for (i in 0 until arr.length()) {
                val p = arr.optJSONObject(i) ?: continue
                val id = p.optString("id", "")
                if (id.isEmpty()) continue
                if (soulPresetOptions.none { it.first == id }) {
                    soulPresetOptions.add(id to p.optString("name", id))
                }
            }
        }
    }

    // ============================================================
    // 记忆动作（状态卡 / 测试召回 / 立即维护 / 清空）
    // ============================================================

    /** `GET /api/memory/status`：把状态拼成一段可读文本（状态卡直接展示）。 */
    fun refreshStatus(ctx: Pager, baseUrl: String) {
        memoryStatusMsg = "查询中…"
        network(ctx).requestGet("${baseUrl}/api/memory/status", JSONObject()) { data, success, errorMsg, _ ->
            if (!success) {
                memoryStatusMsg = "状态查询失败: $errorMsg"
                return@requestGet
            }
            val sb = StringBuilder()
            val kb = data.optLong("db_bytes", 0L) / 1024
            sb.append("引擎 ").append(data.optString("engine", data.optString("backend", "?")))
            sb.append("（").append(data.optString("db_path", "?")).append("，").append(kb).append(" KB）")
            sb.append("\n记忆 ").append(data.optInt("memories", 0)).append(" 条 · 原文 ")
            sb.append(data.optInt("messages", 0)).append(" 条 · 待抽取 ")
            sb.append(data.optInt("pending", 0)).append(" · 隔离 ").append(data.optInt("quarantined", 0))
            val cap = data.optJSONObject("capability")
            val phase = cap?.optString("phase", "") ?: ""
            if (phase.isNotEmpty()) {
                val err = cap?.optString("last_error", "") ?: ""
                sb.append("\n状态: ").append(phase)
                if (err.isNotEmpty()) sb.append(" —— ").append(err)
            }
            val extractor = data.optString("extractor", "")
            if (extractor.isNotEmpty()) sb.append("\n抽取模型: ").append(extractor)
            val aiui = data.optString("aiui_note", "")
            if (aiui.isNotEmpty()) sb.append("\n").append(aiui)
            val scopes = data.optJSONArray("scopes")
            if (scopes != null && scopes.length() > 0) {
                sb.append("\n作用域: ")
                for (i in 0 until scopes.length()) {
                    val s = scopes.optJSONObject(i) ?: continue
                    if (i > 0) sb.append("；")
                    sb.append(s.optString("scope", "")).append(" ").append(s.optInt("memories", 0)).append(" 条")
                }
            }
            memoryStatusMsg = sb.toString()
        }
    }

    /** `POST /api/memory/recall`：当场验证"记忆到底有没有在工作"。 */
    fun testRecall(ctx: Pager, baseUrl: String) {
        val text = memoryTestText.trim()
        if (text.isEmpty()) {
            memoryTestOk = false
            memoryTestMsg = "请先输入一句用于试召回的话"
            return
        }
        memoryTestMsg = "召回中…"
        memoryTestOk = false
        val body = JSONObject().apply {
            put("text", text)
            if (memoryTestScope.isNotEmpty()) put("scope", memoryTestScope.trim())
        }
        network(ctx).httpRequest("${baseUrl}/api/memory/recall", true, body, jsonHeaders()) { data, success, errorMsg, resp ->
            val code = resp.statusCode
            if (!success || (code != null && code !in 200..299)) {
                memoryTestOk = false
                memoryTestMsg = "召回失败: ${if (errorMsg.isNotEmpty()) errorMsg else "HTTP $code"}"
                return@httpRequest
            }
            val hits = data.optInt("hits", 0)
            val ms = data.optLong("elapsed_ms", 0L)
            val err = data.optString("error", "")
            val preview = data.optString("preview", "")
            memoryTestOk = hits > 0
            val sb = StringBuilder()
            if (err.isNotEmpty()) sb.append("降级: ").append(err).append("\n")
            sb.append("命中 ").append(hits).append(" 条 · 匹配词项 ").append(data.optInt("matched_terms", 0))
            sb.append(" · 耗时 ").append(ms).append("ms")
            if (preview.isNotEmpty()) {
                sb.append("\n\n将注入的正文（作为背景资料，不是指令）：\n")
                sb.append(if (preview.length > 900) preview.substring(0, 900) + "…" else preview)
            } else if (hits == 0) {
                sb.append("\n\n没有命中：可能还没聊过相关内容，或这轮属于「近况窗口」（原文已在本轮上下文里）。")
            }
            memoryTestMsg = sb.toString()
        }
    }

    /** `POST /api/memory/maintain`：立即维护（`dry_run` = 只演练不删除）。 */
    fun maintain(ctx: Pager, baseUrl: String, dryRun: Boolean) {
        memoryMaintainMsg = if (dryRun) "演练中…" else "维护中…"
        val body = JSONObject().apply {
            put("force", true)
            put("dry_run", dryRun)
        }
        network(ctx).httpRequest("${baseUrl}/api/memory/maintain", true, body, jsonHeaders()) { data, success, errorMsg, resp ->
            val code = resp.statusCode
            if (!success || (code != null && code !in 200..299)) {
                memoryMaintainMsg = "维护失败: ${if (errorMsg.isNotEmpty()) errorMsg else "HTTP $code"}"
                return@httpRequest
            }
            val verb = if (data?.optBoolean("dry_run", false) == true) "演练（未删除）" else "已处理"
            memoryMaintainMsg = "$verb：隔离释放 ${data?.optInt("released", 0) ?: 0} 条，保留策略处理 ${data?.optInt("deleted", 0) ?: 0} 条"
        }
    }

    /** `POST /api/memory/rag/reindex`：触发后台重建文档索引（P1-9，仅 engine=rag 有效）。 */
    fun reindexRag(ctx: Pager, baseUrl: String) {
        memoryRagReindexMsg = "重建中…"
        network(ctx).httpRequest("${baseUrl}/api/memory/rag/reindex", true, JSONObject(), jsonHeaders()) { data, success, errorMsg, resp ->
            val code = resp.statusCode
            if (!success || (code != null && code !in 200..299)) {
                memoryRagReindexMsg =
                    "重建触发失败: ${data?.optString("message", "")?.ifEmpty { null } ?: if (errorMsg.isNotEmpty()) errorMsg else "HTTP $code"}"
                return@httpRequest
            }
            memoryRagReindexMsg =
                data?.optString("message", "")?.ifEmpty { null } ?: "已开始后台重建（结果查状态卡）"
        }
    }

    /** `POST /api/memory/clear`：**两段式强确认**（第一次调用只置位）。 */
    fun clear(ctx: Pager, baseUrl: String) {
        if (!memoryClearConfirm) {
            memoryClearConfirm = true
            memoryClearMsg = if (memoryClearAll) {
                "⚠️ 再点一次将清空**全部设备**的记忆（不可恢复）"
            } else {
                "⚠️ 再点一次将清空作用域「${memoryClearScope.trim()}」的记忆（不可恢复）"
            }
            return
        }
        memoryClearConfirm = false
        val body = JSONObject().apply {
            if (memoryClearAll) {
                put("all", true)
            } else {
                put("scope", memoryClearScope.trim())
            }
        }
        memoryClearMsg = "清空中…"
        network(ctx).httpRequest("${baseUrl}/api/memory/clear", true, body, jsonHeaders()) { data, success, errorMsg, resp ->
            val code = resp.statusCode
            if (!success || (code != null && code !in 200..299)) {
                val serverMsg = data?.optString("message", "") ?: ""
                memoryClearMsg = "清空失败: " + serverMsg.ifEmpty { errorMsg.ifEmpty { "HTTP $code" } }
                return@httpRequest
            }
            memoryClearMsg = data?.optString("message", "已清空") ?: "已清空"
        }
    }
}