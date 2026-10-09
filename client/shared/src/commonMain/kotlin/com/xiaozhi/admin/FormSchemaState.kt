package com.xiaozhi.admin

import com.tencent.kuikly.core.base.PagerScope
import com.tencent.kuikly.core.module.NetworkModule
import com.tencent.kuikly.core.nvi.serialization.json.JSONArray
import com.tencent.kuikly.core.nvi.serialization.json.JSONObject
import com.tencent.kuikly.core.pager.Pager
import com.tencent.kuikly.core.reactive.handler.observable

/**
 * 字段元数据（`GET /api/config/schema`）的**只读缓存**：目前只用于「多久生效」提示。
 *
 * 为什么要有它（`docs/plugin-architecture-unification.md` §5.4 第 6 条 / §6.5）：
 * 字段级热生效语义的**唯一权威**是服务端注册表（`FieldSchema.hot`）。此前管理页把
 * "保存后新会话生效"这类承诺**硬写在卡片文案里**——注册表改了档位，文案不会跟着变，
 * 正是 §1.7 bug-2（文档说生效、实际没生效）的同一类漂移。现在提示由 schema 驱动：
 * 服务端把某字段改成 `restart_only`，界面提示自动出现，无需改客户端。
 *
 * 呈现口径：
 * - **`live`（保存即生效）不标注**——那是用户的默认预期，逐字段标注只会制造噪音；
 * - 字段级 [fieldNote] 用于"同段内档位不一致"的少数例外（如 `[tts].cache_entries`）；
 * - 段级 [sectionNote] 用于整段档位一致的卡片（`[server]` / `[audio]` / `[aiui]`），
 *   这样一张卡只出现一句提示，而不是 12 个字段各来一遍；
 * - 拉取失败**静默**（提示是增强信息，不能因此干扰表单）。
 *
 * 拆成独立文件：`AGENTS.md` §5.10 文件规模约定（`ConfigFormState` 已接近上限），
 * 且它是"字段元数据"而非"表单字段值"，职责不同。
 */
class FormSchemaState(private val scope: PagerScope) {

    /** `"<段.子段.字段>" -> 提示文案`（只收 `hot != live` 的字段）。 */
    private val hots = mutableMapOf<String, String>()

    /** `"<段>" -> 该段整体档位提示`（段内字段取最严格）。 */
    private val sectionHints = mutableMapOf<String, String>()

    /** `"<段>" -> 已记录档位序`（取最严格时比较用；文案本身分不出严格序）。 */
    private val sectionRanks = mutableMapOf<String, Int>()

    /**
     * 元数据版本号：schema 到达后自增。
     *
     * UI 必须读它才能对"元数据晚于首帧到达"保持响应（见 [fieldNote]）——Kuikly 只对
     * **闭包内读到的 observable** 做响应式跟踪，普通 Map 的写入不会触发任何重建。
     */
    var version by scope.observable(0)

    private fun network(ctx: Pager): NetworkModule = ctx.acquireModule(NetworkModule.MODULE_NAME)

    /** 拉取并展开 schema；失败静默保留旧缓存（服务端不可达时表单仍可用）。 */
    fun load(ctx: Pager, baseUrl: String = "") {
        network(ctx).requestGet("${baseUrl}/api/config/schema", JSONObject()) { data, success, _, _ ->
            if (!success) return@requestGet
            val nextHots = mutableMapOf<String, String>()
            val nextSections = mutableMapOf<String, String>()
            val nextRanks = mutableMapOf<String, Int>()
            // ① 非能力段（[server] / [audio]）：服务端 `APP_SECTION_HOT` 的投影，无字段表
            val appSections = data.optJSONArray("app_sections")
            if (appSections != null) {
                for (i in 0 until appSections.length()) {
                    val s = appSections.optJSONObject(i) ?: continue
                    val id = s.optString("id", "")
                    val hot = s.optString("hot", "")
                    if (id.isEmpty() || rank(hot) == 0) continue
                    nextSections[id] = s.optString("hot_hint", hot)
                    nextRanks[id] = rank(hot)
                }
            }
            // ② 能力字段表（公共字段 + 各实现私有字段）
            val caps = data.optJSONArray("capabilities")
            if (caps != null) {
                for (i in 0 until caps.length()) {
                    val cap = caps.optJSONObject(i) ?: continue
                    val common = cap.optJSONArray("common_fields")
                    if (common != null) {
                        for (j in 0 until common.length()) {
                            val seg = common.optJSONObject(j) ?: continue
                            collect(nextHots, nextSections, nextRanks, seg)
                        }
                    }
                    val impls = cap.optJSONArray("implementations")
                    if (impls != null) {
                        for (j in 0 until impls.length()) {
                            val im = impls.optJSONObject(j) ?: continue
                            collect(nextHots, nextSections, nextRanks, im)
                        }
                    }
                }
            }
            // 整批替换：避免"拉了一半"的中间态被读到
            hots.clear(); hots.putAll(nextHots)
            sectionHints.clear(); sectionHints.putAll(nextSections)
            sectionRanks.clear(); sectionRanks.putAll(nextRanks)
            version++
        }
    }

    /** 把一段（含 `fields_path` / `fields`）并入结果表；段级取该段**最严格**档位。 */
    private fun collect(
        into: MutableMap<String, String>,
        sectionHints: MutableMap<String, String>,
        sectionRanks: MutableMap<String, Int>,
        seg: JSONObject,
    ) {
        val path = seg.optJSONArray("fields_path") ?: return
        val fields = seg.optJSONArray("fields") ?: return
        val prefix = (0 until path.length()).joinToString(".") { path.optString(it) ?: "" }
        val section = path.optString(0) ?: ""
        for (i in 0 until fields.length()) {
            val f = fields.optJSONObject(i) ?: continue
            val key = f.optString("key", "")
            if (key.isEmpty()) continue
            val hot = f.optString("hot", "")
            val strict = rank(hot)
            if (strict == 0) continue // live 不标注
            into[if (prefix.isEmpty()) key else "$prefix.$key"] = f.optString("hot_hint", hot)
            if (section.isNotEmpty() && strict > (sectionRanks[section] ?: -1)) {
                sectionRanks[section] = strict
                sectionHints[section] = f.optString("hot_hint", hot)
            }
        }
    }

    /** 档位严格序（与服务端 `HotReload::rank` 对齐：live < next_session < restart_only）。 */
    private fun rank(hot: String): Int = when (hot) {
        "next_session" -> 1
        "restart_only" -> 2
        else -> 0
    }

    /**
     * 字段的生效语义提示（空串 = 无需提示）。
     * [path] 是字段所在配置段（如 `"server"`、`"tts.xfyun"`），[key] 是字段名。
     *
     * ⚠️ 必须在 `attr` / `vif` **闭包内**调用（而不是构建期求值成 String 传下去）：
     * 函数内部先读了 [version] 这个 observable，Kuikly 会因此把该闭包注册到元数据版本上，
     * 于是"schema 晚于首帧到达"时提示仍会出现。构建期快照会让你永远看不到提示。
     */
    fun fieldNote(path: String, key: String): String {
        val stamp = version // 建立响应式依赖（勿删，见上方注释）
        return if (stamp >= 0) hots["$path.$key"] ?: "" else ""
    }

    /**
     * 整段档位提示（空串 = 无需提示）：用于"整段同一档位"的卡片（如 `[server]` / `[audio]` / `[aiui]`）。
     *
     * ⚠️ 段内档位不一致时（如 `[tts]`：`engine` 新会话、`cache_entries` 需重启）**不要**用它——
     * 那会把整段说成"需重启"。这种情况请用 [fieldNote] 标注例外字段。
     */
    fun sectionNote(section: String): String {
        val stamp = version // 同上：建立响应式依赖
        return if (stamp >= 0) sectionHints[section] ?: "" else ""
    }
}
