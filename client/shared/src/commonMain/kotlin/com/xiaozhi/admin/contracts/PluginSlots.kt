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

// ============================================================
// 设备插件注册门控（GET /api/plugins 的 device_plugins 键）
// ============================================================

/**
 * 设备插件注册投影的**三态快照**（零 Kuikly 依赖，adminCoreTests 纯 JVM 可回归）。
 *
 * 为什么不能只用一个布尔：同一个「空数组」在 wire 上代表两件不同的事——
 * - **缺 `device_plugins` 键**（旧服务端 / 回包异常）→ 语义未知，必须保持编译期静态基线，
 *   既不误删菜单，也不在加载间隙闪隐；
 * - **明确下发 `device_plugins: []`**（`pure-base` 纯底座空壳）→ 服务端明确声明
 *   「一个设备插件都没有」，此时才隐藏由设备插件驱动的菜单；
 * - **明确下发非空数组**（常规构建）→ 按 `registered` 决定「设备管理」的运行分区；
 *   **配置段始终保留**，否则用户把插件关掉后就再也找不到重新开启的入口。
 *
 * 唯一权威数据源是 `device_plugins[]`。`capabilities[]`（编译期 `ALL_CAPABILITIES` 注册表）
 * 与 `/api/config/schema` 的 `app_sections`（编译期热生效档位表）在纯底座里也会原样下发，
 * **不能**用来判断「服务端有没有插件」。
 */
data class DevicePluginNavState(
    /** 本次回包是否已成功解析；false = 加载中 / 请求失败，保持静态基线。 */
    val loaded: Boolean,
    /** 回包是否明确含 `device_plugins` 数组（缺键 / 值非数组 = false）。 */
    val declared: Boolean,
    /** 服务端明确声明的设备插件 id 全集；`declared && isEmpty()` 即空壳。 */
    val declaredIds: Set<String>,
    /** 其中 `registered=true` 的 id。 */
    val registeredIds: Set<String>,
    /** `/api/plugins.ui.mode == pure-base`：静态业务 UI 不属于底座。 */
    val pureBase: Boolean = false,
) {
    companion object {
        /** 未知态：加载中 / 不可达 / 旧服务端缺键 —— 一律保持静态基线。 */
        val UNKNOWN = DevicePluginNavState(
            loaded = false,
            declared = false,
            declaredIds = emptySet(),
            registeredIds = emptySet(),
        )
    }
}

/**
 * 设备插件驱动的静态菜单门控。
 *
 * 只认 [DEVICE_HUB_NAV_ID] 与 [SECTION_PLUGIN_IDS] 里的 id，其余核心导航
 * （概览 / 语音流程 / 全局配置 / 文件 / 插件）一律不动——这是「最小、可回退」的边界。
 * 所有函数都是纯函数（不读 Kuikly 状态），因此能在 adminCoreTests 直驱回归。
 */
object DevicePluginNav {
    /** 侧栏一级项「设备管理」。 */
    const val DEVICE_HUB_NAV_ID = "devicehub"

    /** 纯底座仍提供的静态运维入口；业务 UI 必须来自 WASM registration.menu。 */
    private val PURE_BASE_NAV_IDS = setOf("overview", "devices", "console", "plugins", "files")

    /** 纯底座保留的部署级配置；业务 section 不再由编译期客户端常量渲染。 */
    private val PURE_BASE_SECTION_IDS = setOf("overview", "server")

    /**
     * 由业务插件驱动的静态 section。底座不为这些 id 提供任何 UI；未知/未加载状态
     * 也必须先隐藏，不能以“兼容旧服务端”为由把尚未注册的业务误显示出来。
     * 注意口径：`nlu`（模型唯一识别链）、`audio`（下行音频参数）、`mcp`（设备工具注册表）
     * 属底座/系统级契约，不算业务插件；pure-base 下它们由 [PURE_BASE_SECTION_IDS] 收口。
     */
    private val BUSINESS_SECTION_IDS = setOf(
        "presets", "command", "wakeup", "soul", "memory", "aiui",
        "echomuse", "tuya", "ota",
    )

    /** 由设备插件驱动的配置 section → 插件 id（兼容旧配置路径）。 */
    val SECTION_PLUGIN_IDS: Map<String, String> = mapOf(
        "echomuse" to "echomuse",
        "tuya" to "tuya",
    )

    /** 语义未知：不隐藏、不闪隐（加载中 / 不可达 / 旧服务端缺键）。 */
    fun isUnknown(state: DevicePluginNavState): Boolean = !state.loaded || !state.declared

    /** 服务端明确声明零个设备插件（纯底座空壳）。缺键不算空壳。 */
    fun isEmptyShell(state: DevicePluginNavState): Boolean =
        !isUnknown(state) && state.declaredIds.isEmpty()

    /** 侧栏「设备管理」：只有服务端明确声明并至少注册一个设备插件才显示。 */
    fun showDeviceHub(state: DevicePluginNavState): Boolean =
        !state.pureBase && !isUnknown(state) && state.registeredIds.isNotEmpty()

    /** 设备插件的**运行分区**（设备管理页里的设备卡 / 齿轮配置块）：按注册态逐个判定。 */
    fun showPluginPartition(state: DevicePluginNavState, pluginId: String): Boolean =
        !state.pureBase && !isUnknown(state) && pluginId in state.registeredIds

    /**
     * 配置 section 是否显示。
     *
     * 业务 section（含设备插件 section）只在服务端**明确证明**其存在时出现：
     * pure-base 只保留部署级配置；明确空壳与未知态一律隐藏业务；设备 section 还要求
     * 对应插件 `registered=true`。「未注册就没有对应业务 UI」是本门控的唯一语义。
     */
    fun showSection(state: DevicePluginNavState, sectionId: String): Boolean {
        // 底座只包含核心配置；任何业务 section 都必须由动态插件菜单承载。
        if (state.pureBase) return sectionId in PURE_BASE_SECTION_IDS
        // 明确空壳没有任何业务插件；业务 section 不能因 legacy 编译期常量重新出现。
        if (isEmptyShell(state) && sectionId in BUSINESS_SECTION_IDS) return false
        // 首次加载、旧服务端或回包异常时，宁可隐藏业务，也不能把编译期业务误当成已安装插件。
        if (isUnknown(state)) return sectionId !in BUSINESS_SECTION_IDS
        // 设备业务 section 也必须有对应的注册 id；仅存在配置键或产物文件不算注册。
        SECTION_PLUGIN_IDS[sectionId]?.let { pluginId -> return pluginId in state.registeredIds }
        // legacy/full 构建的核心非设备 section 由其 feature 自身决定；pure-base 已在上面收口。
        return true
    }

    /** 单个静态菜单项（导航 id 或配置 section id）是否可见。 */
    fun showEntry(state: DevicePluginNavState, id: String): Boolean {
        if (id.startsWith("plugin:")) return true
        if (state.pureBase) return id in PURE_BASE_NAV_IDS || showSection(state, id)
        return if (id == DEVICE_HUB_NAV_ID) showDeviceHub(state) else showSection(state, id)
    }

    /** 过滤静态菜单列表；空组由调用方跳过（与侧栏 / 全局配置菜单现有渲染一致）。 */
    fun <T> filterItems(
        items: List<T>,
        idOf: (T) -> String,
        state: DevicePluginNavState,
    ): List<T> = items.filter { item -> showEntry(state, idOf(item)) }
}
