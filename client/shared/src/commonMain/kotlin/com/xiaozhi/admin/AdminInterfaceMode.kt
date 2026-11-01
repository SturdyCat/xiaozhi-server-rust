package com.xiaozhi.admin

/** Client presentation preference only; never part of config JSON, revision or draft. */
enum class AdminInterfaceMode(val storedValue: String, val label: String) {
    Simple("simple", "简洁"), Advanced("advanced", "高级");

    companion object {
        fun fromStored(value: String): AdminInterfaceMode =
            entries.firstOrNull { it.storedValue == value } ?: Simple
    }
}

/** Preferences are client-local (browser origin / native application).
 * Include the server, never credentials, so native server switches are independent.
 */
fun interfaceModePreferenceKey(baseUrl: String): String {
    val address = baseUrl.trim().substringBefore('#').substringBefore('?').trimEnd('/')
    val schemeEnd = address.indexOf("://")
    val server = if (schemeEnd >= 0) {
        val scheme = address.substring(0, schemeEnd).lowercase()
        val rest = address.substring(schemeEnd + 3)
        val authority = rest.substringBefore('/').substringAfterLast('@').lowercase()
        val path = rest.substringAfter('/', "").let { if (it.isEmpty()) "" else "/$it" }
        "$scheme://$authority$path"
    } else {
        // Also sanitize scheme-relative / bare-host addresses without changing base-path case.
        val prefix = if (address.startsWith("//")) "//" else ""
        val rest = address.removePrefix(prefix)
        val authority = rest.substringBefore('/').substringAfterLast('@').lowercase()
        val path = rest.substringAfter('/', "").let { if (it.isEmpty()) "" else "/$it" }
        "$prefix$authority$path"
    }
    return "xz.admin.interfaceMode.v1:" + server.ifEmpty { "same-origin" }
}
