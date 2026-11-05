package com.xiaozhi.admin.contracts

import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive

/**
 * 运行时 WASM 插件槽位投影（P2e）。
 *
 * 服务端未编译 wasm、服务端不可达、旧服务端没有 `wasm` 键时统一返回 null；
 * 调用方必须保留 [AdminNav] 的静态基线，不能把动态投影当成唯一导航来源。
 * 本文件零 Kuikly 依赖（kotlinx JSON 纯解析），所有合并规则可在 adminCoreTests 纯 JVM 回归。
 */
data class PluginSlotSnapshot(
    val plugins: List<PluginSlotPlugin>,
    val extensions: Map<String, PluginExtensionState>,
    val navEntries: List<PluginNavEntry>,
) {
    fun plugin(id: String): PluginSlotPlugin? = plugins.firstOrNull { it.id == id }

    companion object {
        private val json = Json { ignoreUnknownKeys = true }

        /**
         * 从 `GET /api/plugins` 回包的 `wasm` 键解析。
         * 缺少该键、结构不完整或插件数组缺失 = null（静态基线回退）。
         */
        fun parse(wasmJsonText: String?): PluginSlotSnapshot? {
            val text = wasmJsonText?.takeIf { it.isNotBlank() } ?: return null
            val wasm = runCatching { json.parseToJsonElement(text).jsonObject }.getOrNull()
                ?: return null
            val rows = runCatching { wasm["plugins"]?.jsonArray }.getOrNull() ?: return null
            val plugins = mutableListOf<PluginSlotPlugin>()
            val nav = mutableListOf<PluginNavEntry>()
            for (row in rows) {
                val obj = runCatching { row.jsonObject }.getOrNull() ?: continue
                val id = obj.str("id") ?: continue
                val registration = obj["registration"]?.jsonObject
                val plugin = PluginSlotPlugin(
                    id = id,
                    title = obj.str("title") ?: id,
                    description = obj.str("description") ?: "",
                    version = obj.str("version") ?: "",
                    official = obj.bool("official"),
                    phase = obj["phase"]?.jsonObject?.str("state") ?: "unknown",
                    implements = obj.strings("implements"),
                    permissions = obj.strings("permissions"),
                    actions = parseActions(registration?.get("actions")?.jsonArray),
                    configFields = parseFields(registration?.get("config_fields")?.jsonArray),
                )
                plugins += plugin
                val menu = registration?.get("menu")?.jsonArray ?: continue
                for (entryEl in menu) {
                    val entry = runCatching { entryEl.jsonObject }.getOrNull() ?: continue
                    if (entry.str("slot") != "nav.main") continue
                    val entryId = entry.str("id") ?: continue
                    nav += PluginNavEntry(
                        pluginId = id,
                        entryId = entryId,
                        label = entry.str("label") ?: plugin.title,
                        group = entry.str("group") ?: "插件",
                        order = entry.int("order") ?: 0,
                        icon = entry.str("icon") ?: "settings",
                    )
                }
            }
            val extensions = mutableMapOf<String, PluginExtensionState>()
            val ext = wasm["extensions"]?.jsonObject
            if (ext != null) {
                for ((key, value) in ext) {
                    val row = runCatching { value.jsonObject }.getOrNull() ?: continue
                    extensions[key] = PluginExtensionState(
                        state = row.str("state") ?: "unresolved",
                        implementation = row.str("implementation") ?: "",
                        preferred = row.str("preferred") ?: "",
                    )
                }
            }
            return PluginSlotSnapshot(plugins, extensions, nav)
        }

        private fun JsonObject.str(key: String): String? =
            (this[key] as? kotlinx.serialization.json.JsonPrimitive)?.takeIf { it.isString }
                ?.content

        private fun JsonObject.int(key: String): Int? =
            (this[key] as? kotlinx.serialization.json.JsonPrimitive)?.content?.toIntOrNull()

        private fun JsonObject.bool(key: String): Boolean =
            (this[key] as? kotlinx.serialization.json.JsonPrimitive)?.content == "true"

        private fun JsonObject.strings(key: String): List<String> {
            val array = this[key]?.jsonArray ?: return emptyList()
            return array.mapNotNull { el ->
                (el as? kotlinx.serialization.json.JsonPrimitive)?.takeIf { it.isString }?.content
            }.filter { it.isNotBlank() }
        }

        private fun parseActions(array: kotlinx.serialization.json.JsonArray?): List<PluginAction> {
            if (array == null) return emptyList()
            return array.mapNotNull { el ->
                val row = (el as? JsonObject) ?: return@mapNotNull null
                row.str("id")?.takeIf { it.isNotBlank() }?.let { id ->
                    PluginAction(
                        id = id,
                        label = row.str("label") ?: id,
                        danger = row.bool("danger"),
                        help = row.str("help") ?: "",
                    )
                }
            }
        }

        private fun parseFields(array: kotlinx.serialization.json.JsonArray?): List<PluginConfigField> {
            if (array == null) return emptyList()
            return array.mapNotNull { el ->
                val row = (el as? JsonObject) ?: return@mapNotNull null
                row.str("key")?.takeIf { it.isNotBlank() }?.let { key ->
                    PluginConfigField(
                        key = key,
                        label = row.str("label") ?: key,
                        kind = row.str("kind") ?: "str",
                        help = row.str("help") ?: "",
                        defaultHint = row.str("default_hint") ?: "",
                        secret = row.bool("secret"),
                    )
                }
            }
        }
    }
}

data class PluginSlotPlugin(
    val id: String,
    val title: String,
    val description: String,
    val version: String,
    val official: Boolean,
    val phase: String,
    val implements: List<String>,
    val permissions: List<String>,
    val actions: List<PluginAction>,
    val configFields: List<PluginConfigField>,
)

data class PluginAction(val id: String, val label: String, val danger: Boolean, val help: String)

data class PluginConfigField(
    val key: String,
    val label: String,
    val kind: String,
    val help: String,
    val defaultHint: String,
    val secret: Boolean,
)

data class PluginNavEntry(
    val pluginId: String,
    val entryId: String,
    val label: String,
    val group: String,
    val order: Int,
    val icon: String,
) {
    val navigationId: String get() = "plugin:$pluginId:$entryId"
}

data class PluginExtensionState(val state: String, val implementation: String, val preferred: String)

/** 服务端不可达/旧服务端时，动态槽位不应污染静态菜单。 */
object PluginSlots {
    const val PLUGIN_GROUP = "插件"
    private const val ORDER_BASE = 9000

    fun mergeNav(staticItems: List<NavDescriptor>, snapshot: PluginSlotSnapshot?): List<NavDescriptor> {
        if (snapshot == null) return staticItems
        val existing = staticItems.mapTo(mutableSetOf()) { it.id }
        val dynamic = snapshot.navEntries
            .sortedWith(compareBy<PluginNavEntry> { it.order }.thenBy { it.pluginId }.thenBy { it.entryId })
            .mapNotNull { entry ->
                if (entry.navigationId in existing) return@mapNotNull null
                existing += entry.navigationId
                NavDescriptor(
                    id = entry.navigationId,
                    order = ORDER_BASE + entry.order.coerceIn(0, 999),
                    label = entry.label,
                    group = PLUGIN_GROUP,
                )
            }
        return staticItems + dynamic
    }

    fun mergeGrouped(
        staticGroups: List<Pair<String, List<NavDescriptor>>>,
        snapshot: PluginSlotSnapshot?,
    ): List<Pair<String, List<NavDescriptor>>> {
        if (snapshot == null) return staticGroups
        val dynamic = mergeNav(emptyList(), snapshot)
        if (dynamic.isEmpty()) return staticGroups
        return staticGroups + (PLUGIN_GROUP to dynamic)
    }

    fun pluginIdFromNavigation(id: String): String? {
        if (!id.startsWith("plugin:")) return null
        return id.removePrefix("plugin:").substringBefore(':').takeIf { it.isNotBlank() }
    }
}
