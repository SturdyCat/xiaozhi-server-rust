package com.xiaozhi.admin

import com.tencent.kuikly.core.base.PagerScope
import com.tencent.kuikly.core.nvi.serialization.json.JSONObject
import com.tencent.kuikly.core.reactive.handler.observable

/**
 * 「唤醒问候语」（`[wakeup]`）的表单状态。
 *
 * 为什么单独一个类（而不是继续往 `ConfigFormState` 里加字段）：`AGENTS.md` §5.10
 * 要求逻辑文件 ≤ 600 行，而 `ConfigFormState` 已在上限附近；这里与 `CommandSectionState`
 * 同一做法——`form.wake.*` 成组出现，JSON 映射放在本文件的 `putJson` / `fill`，
 * 渲染见 `ConfigWakeupCard.kt`。
 *
 * 语义（服务端为唯一权威，见 `server/src/plugins/wakeup/mod.rs`）：
 * 设备唤醒词命中后固件**立刻**上报 `listen state=detect`（需固件 `CONFIG_SEND_WAKE_WORD_DATA=y`），
 * 服务器收到即说 [greeting]（默认「我在」），**跳过 LLM 与正常回复**。成品音频按音色三级复用：
 * 内存 TTS 缓存 → 磁盘（[persist]）→ 真合成一次后落盘；稳态下每次唤醒零 LLM、零 TTS。
 * `enabled = false`（默认）时行为与没有这段完全一致。
 */
class WakeupSectionState(private val scope: PagerScope) {

    var enabled by scope.observable("false")
    /// 唤醒后立刻说的这句话。建议短：播放期间上行音频在缓冲区排队，太长会推迟用户第一句话。
    var greeting by scope.observable("我在")
    /// 是否把成品音频落盘（跨进程重启复用）。⚠️ 需要 [tts].cache_entries > 0。
    var persist by scope.observable("true")

    /**
     * 写入保存请求体（`[wakeup]` 整段回传）。
     *
     * 与其它段一样**必须整段回传**：服务端虽是部分更新语义，但前端不回传就等于
     * "这个段在 UI 里不可配置"（旧客户端漏传整段曾被静默重置过，见
     * `docs/plugin-architecture-unification.md` §1.7 bug-1）。
     */
    fun putJson(root: JSONObject) {
        root.put(
            "wakeup",
            JSONObject().apply {
                put("enabled", enabled.toBooleanStrictOrNull() ?: false)
                put("greeting", greeting)
                put("persist", persist.toBooleanStrictOrNull() ?: true)
            },
        )
    }

    /** 用服务端 `/api/config` 回包填充（缺键保留当前值，不静默改成默认值）。 */
    fun fill(obj: JSONObject) {
        obj.optJSONObject("wakeup")?.let { w ->
            enabled = w.optBoolean("enabled", enabled.toBooleanStrictOrNull() ?: false).toString()
            greeting = w.optString("greeting", greeting)
            persist = w.optBoolean("persist", persist.toBooleanStrictOrNull() ?: true).toString()
        }
    }
}
