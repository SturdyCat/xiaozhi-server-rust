package com.xiaozhi.admin

import com.tencent.kuikly.core.base.PagerScope
import com.tencent.kuikly.core.nvi.serialization.json.JSONArray
import com.tencent.kuikly.core.nvi.serialization.json.JSONObject
import com.tencent.kuikly.core.reactive.collection.ObservableList
import com.tencent.kuikly.core.reactive.handler.observable
import com.tencent.kuikly.core.reactive.handler.observableList

/**
 * 「指令闸门」（`[command]`）的表单状态。
 *
 * 为什么单独一个类（而不是继续往 `ConfigFormState` 里加字段）：`AGENTS.md` §5.10
 * 要求逻辑文件 ≤ 600 行，而 `ConfigFormState` 已在上限附近；这里与 `ContextSectionState`
 * 同一做法——`form.cmd.*` 成组出现，JSON 映射放在本文件的 `putJson` / `fill`，
 * 渲染见 `ConfigCommandCard.kt`。
 *
 * 语义（服务端为唯一权威，见 `server/src/plugins/command/mod.rs`）：
 * ASR 识别文本在送进 LLM **之前**先过两道闸：① **长度闸门**——归一化后超过 `maxChars`
 * 个字（默认 5，0 = 不限制）的长句携带信息，一律放行给大模型；② 按 `keywords` 匹配
 * （`matchMode`：exact/contains）。两者都通过才命中，命中则先说 `reply` 再把本次会话断开。
 * `enabled = false`（默认）时行为与没有这段完全一致。
 *
 * ⚠️ `keywords` 是数组字段，在表单里是**多行文本**（每行一个词），读写时统一转换
 * （与 `[soul].traits` 等数组字段同一约定）。
 */
class CommandSectionState(private val scope: PagerScope) {

    var enabled by scope.observable("false")
    /// 指令词表：每行一个词（归一化后比较；「退下吧。」等价于「退下」）。
    var keywords by scope.observable("退下\n闭嘴\n关闭")
    /// exact（整句相等，默认、安全）| contains（句中出现即命中，⚠️ 会让「关闭闹钟」也断线）。
    var matchMode by scope.observable("exact")
    /// 长度闸门：归一化后超过这么多字就不做指令判断（0 = 不限制）；留空/非法按默认 5 处理。
    var maxChars by scope.observable("5")
    /// 命中后先说的告别语；留空 = 直接断开、不出声。
    var reply by scope.observable("好的，我先退下了。")

    /// 匹配方式下拉选项（官方 AlertDialog 要求 ObservableList）。
    val matchModeOptions: ObservableList<Pair<String, String>> by scope.observableList()

    init {
        matchModeOptions.addAll(
            listOf(
                "exact" to "整句相等（推荐，安全）",
                "contains" to "句中出现即命中（谨慎）",
            ),
        )
    }

    /** 多行文本 → JSON 数组（每行一个词；空行/空白项丢弃）。 */
    private fun lines(s: String): JSONArray {
        val arr = JSONArray()
        s.split("\n").map { it.trim() }.filter { it.isNotEmpty() }.forEach { arr.put(it) }
        return arr
    }

    /** JSON 数组 → 多行文本（空白项丢弃）。 */
    private fun joinLines(arr: JSONArray?): String {
        if (arr == null) return ""
        val out = mutableListOf<String>()
        for (i in 0 until arr.length()) {
            val v = (arr.optString(i) ?: "").trim()
            if (v.isNotEmpty()) out.add(v)
        }
        return out.joinToString("\n")
    }

    /**
     * 写入保存请求体（`[command]` 整段回传）。
     *
     * 与其它段一样**必须整段回传**：服务端虽是部分更新语义，但前端不回传就等于
     * "这个段在 UI 里不可配置"（旧客户端漏传整段曾被静默重置过，见
     * `docs/plugin-architecture-unification.md` §1.7 bug-1）。
     */
    fun putJson(root: JSONObject) {
        root.put(
            "command",
            JSONObject().apply {
                put("enabled", enabled.toBooleanStrictOrNull() ?: false)
                put("keywords", lines(keywords))
                put("match_mode", matchMode)
                // 留空/非数字 = 回落到默认 5；负数钳到 0（服务端 0 = 不限制）
                put("max_chars", (maxChars.trim().toIntOrNull() ?: 5).coerceAtLeast(0))
                put("reply", reply)
            },
        )
    }

    /** 用服务端 `/api/config` 回包填充（缺键保留当前值；`keywords` 为空数组时如实显示空）。 */
    fun fill(obj: JSONObject) {
        obj.optJSONObject("command")?.let { c ->
            enabled = c.optBoolean("enabled", enabled.toBooleanStrictOrNull() ?: false).toString()
            // 数组缺键 = 老服务端（保留当前草稿）；空数组 = 用户确实清空了词表（如实显示）
            c.optJSONArray("keywords")?.let { keywords = joinLines(it) }
            matchMode = c.optString("match_mode", matchMode)
            // 数字缺键 = 老服务端（保留当前草稿，不静默改成默认值）
            maxChars = c.optInt("max_chars", maxChars.trim().toIntOrNull() ?: 5).toString()
            reply = c.optString("reply", reply)
        }
    }
}
