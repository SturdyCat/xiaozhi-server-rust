package com.xiaozhi.admin.compose

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import com.tencent.kuikly.core.nvi.serialization.json.JSONObject
import com.tencent.kuikly.core.pager.Pager
import com.xiaozhi.admin.doubleText
import com.xiaozhi.admin.getJson
import com.xiaozhi.admin.intText
import com.xiaozhi.admin.network
import com.xiaozhi.admin.pagerBaseUrl

/**
 * 「配置段草稿」通用基类：一个 Compose 段 = 一份草稿 + 读 `GET /api/config` + 写 `POST /api/config`。
 *
 * ## 为什么要有它
 * 迁移是逐段进行的，每个段都要处理同样的四件事：加载态、保存态、结果文案、`expected_revision`
 * 冲突（409）。若每段各写一遍（旧代码 `AiuiDraft` 就是全手写），三十多个配置段会出现三十多份
 * 逐字相同的请求骨架，任何一处行为调整都要扫全量。
 *
 * ## 与旧 `ConfigFormState` 的关系
 * 旧表单是**单个巨型 observable 状态**（几百个字段 + schema 驱动 + 设备 overlay + 测试台联动），
 * 且是 Kuikly DSL 的观察系统；Compose 的快照系统读它不会触发重组。因此 Compose 侧只能"每段
 * 一份草稿"，按段提交（服务端 `POST /api/config` 是部分更新语义，只提交出现的键）。
 *
 * ## 提交契约（与旧版逐字一致）
 * - 只提交本段涉及的键；不碰其他段、不碰密钥以外的字段。
 * - 必须带 `Content-Type: application/json`，否则 Kuikly 会把 body 编成 form-urlencoded（服务端 415）。
 * - 带 `expected_revision`：期间被别的窗口改过则服务端回 409，这里提示重新加载而不是覆盖。
 */
internal abstract class AdminConfigDraft {

    var loading by mutableStateOf(false)
        private set
    var saving by mutableStateOf(false)
        private set
    var message by mutableStateOf("")
        private set
    var isError by mutableStateOf(false)
        private set

    /** 已成功加载过一次（切换段时用来避免重复拉整份配置）。 */
    var loadedOnce by mutableStateOf(false)
        private set

    private var revision by mutableStateOf("")

    /** 用 `GET /api/config` 的整份回包填充本段字段（子类只取自己关心的段）。 */
    abstract fun fill(config: JSONObject)

    /** 组装要提交的部分更新（只含本段的键）。 */
    abstract fun collect(): JSONObject

    /**
     * 本段的界面描述（见 [SettingRow]）。
     *
     * 放在草稿里而不是单独写一个 Composable：段的字段与它的展示顺序天然是一件事，
     * 拆成两个文件后每次加字段都要在两处对齐；且行模型是纯数据，不引入 Compose 依赖。
     */
    abstract fun rows(): List<SettingRow>

    /** 变更后调用：清掉上一次的结果文案，避免"已保存"挂在新的改动上误导。 */
    fun markDirty() {
        if (message.isNotEmpty()) message = ""
        isError = false
    }

    fun load(pager: Pager, force: Boolean = false) {
        if (loading || saving) return
        if (loadedOnce && !force) return
        loading = true
        getJson(pager, "${pagerBaseUrl(pager)}/api/config") { ok, data, reason ->
            loading = false
            if (!ok) {
                message = if (reason.isEmpty()) "加载失败" else "加载失败：$reason"
                isError = true
                return@getJson
            }
            revision = data.optString("revision", revision)
            fill(data)
            loadedOnce = true
            message = "已加载"
            isError = false
        }
    }

    fun save(pager: Pager) {
        if (saving || loading) return
        saving = true
        val body = collect()
        if (revision.isNotEmpty()) body.put("expected_revision", revision)
        val headers = JSONObject().apply { put("Content-Type", "application/json") }
        network(pager).httpRequest(
            "${pagerBaseUrl(pager)}/api/config",
            true,
            body,
            headers,
        ) { data, success, errorMsg, resp ->
            saving = false
            val code = resp.statusCode
            when {
                success && (code == null || code in 200..299) -> {
                    revision = data?.optString("revision", revision) ?: revision
                    message = data?.optString("message", "已保存").orEmpty().ifEmpty { "已保存" }
                    isError = false
                }
                code == 409 -> {
                    message = "配置已被其他窗口修改，请重新加载后再改"
                    isError = true
                }
                else -> {
                    message = "保存失败：${if (errorMsg.isNotEmpty()) errorMsg else "HTTP $code"}"
                    isError = true
                }
            }
        }
    }
}

/** 段草稿字段的声明辅助：`var x by draftState("")`，与 Compose 快照联动。 */
internal fun draftState(initial: String) = mutableStateOf(initial)

/** 布尔字段的声明辅助：`var x by draftBool(false)`。 */
internal fun draftBool(initial: Boolean) = mutableStateOf(initial)

/** 段 JSON 的读写小工具：统一"文本草稿 ⇄ 服务端数字/布尔"的转换与兜底。 */
internal object DraftFields {

    fun text(obj: JSONObject, key: String, current: String, fallback: String = current): String =
        obj.optString(key, "").takeIf { it.isNotEmpty() } ?: current.ifEmpty { fallback }

    fun int(obj: JSONObject, key: String, current: String, fallback: Int): String =
        obj.intText(key, current, fallback)

    fun double(obj: JSONObject, key: String, current: String, fallback: Double): String =
        obj.doubleText(key, current, fallback)

    fun bool(obj: JSONObject, key: String, current: Boolean): Boolean = obj.optBoolean(key, current)

    /** 文本 → Int，非法输入回落到兜底（服务端只接受数字，别把空串发过去）。 */
    fun toInt(text: String, fallback: Int): Int = text.trim().toIntOrNull() ?: fallback
}
