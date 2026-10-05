package utils

/** 声明 JS 执行环境的 url decode 方法 */
external fun decodeURIComponent(encoded: String): String

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
