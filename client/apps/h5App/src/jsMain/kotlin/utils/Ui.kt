package utils

import com.tencent.kuikly.core.render.web.nvi.serialization.json.JSONObject
import kotlinx.browser.document
import kotlinx.browser.window

/** 声明 JS 执行环境的 url decode 方法 */
external fun decodeURIComponent(encoded: String): String

/** 统一封装 UI 类操作（在页面上展示 toast 等） */
object Ui {
    internal fun showToast(message: JSONObject) {
        val content = message.optString("content")
        if (content != "") {
            val wrapDiv = document.createElement("div")
            val contentDiv = document.createElement("div")
            wrapDiv.classList.add("toast-wrapper")
            contentDiv.classList.add("toast-content")
            contentDiv.innerHTML = content
            wrapDiv.appendChild(contentDiv)
            document.body?.appendChild(wrapDiv)
            window.setTimeout({ document.body?.removeChild(wrapDiv) }, 3000)
        }
    }
}

/** 统一封装 URL 类操作 */
object URL {
    internal fun parseParams(url: String): Map<String, String> {
        val params = mutableMapOf<String, String>()
        if (url.contains("?")) {
            val query = url.substringAfter("?")
            if (query != "") {
                query.split("&").forEach { param ->
                    val (name, value) = param.split("=")
                    params[name] = decodeURIComponent(value)
                }
            }
        }
        return params
    }
}
