package com.xiaozhi.admin

import com.xiaozhi.admin.contracts.AdminNav
import com.xiaozhi.admin.contracts.DevicePluginNav
import com.xiaozhi.admin.contracts.DevicePluginNavState
import com.xiaozhi.admin.contracts.PluginSlots
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertTrue

/**
 * QA 独立穿透性回归：按**真实渲染路径**复算三处「成品菜单集合」，而不是只直驱纯函数。
 *
 * 与 PluginSlotsModelTest 的区别：
 * - 这里复刻 AdminShell.kt:133-139（hiddenNavIds + grouped + 过滤）、
 *   ConfigWorkspaceViews.kt:257-261（globalGroups 分组过滤 + 空组跳过）、
 *   ConfigWorkspaceViews.kt:132-134（start 位置节点按 sectionId 过滤）三处入口的调用链；
 * - 先用前置断言证明静态基线里**确实存在**待隐藏项，再断言空壳下「不含」，
 *   并用「明确注册 / 未知态均按注册门控」做反例，
 *   避免「对本来就不含目标项的集合断言不含」这类恒真测试；
 * - 用「静态基线 - 白名单」做严格相等，任何连带隐藏核心项都会让断言失败。
 *
 * 覆盖缺口说明：PluginMetaState.apply() 依赖 Kuikly 的 PagerScope / JSONObject，且
 * PluginMetaState.kt 未纳入 adminCoreTests 的 kotlin.include 白名单（见
 * adminCoreTests/build.gradle.kts:15-58），因此 wire 层「device_plugins 缺键 vs 明确 []」
 * 的解析无法在纯 JVM 回归，本文件只覆盖纯契约与三处渲染入口的过滤结果。
 */
class DevicePluginNavPenetrationTest {

    /** 服务端明确下发 device_plugins: [] 的纯底座空壳。 */
    private val emptyShell = DevicePluginNavState(
        loaded = true, declared = true, declaredIds = emptySet(), registeredIds = emptySet(),
    )

    /** 加载中 / 不可达 / 旧服务端缺键。 */
    private val unknown = DevicePluginNavState.UNKNOWN

    /** 常规构建：两个设备插件都声明且都注册。 */
    private val bothRegistered = DevicePluginNavState(
        loaded = true, declared = true,
        declaredIds = setOf("echomuse", "tuya"), registeredIds = setOf("echomuse", "tuya"),
    )

    /** 常规构建但用户把两个插件都关了：运行分区隐藏，配置段必须保留。 */
    private val bothDisabled = DevicePluginNavState(
        loaded = true, declared = true,
        declaredIds = setOf("echomuse", "tuya"), registeredIds = emptySet(),
    )

    /** 复刻 AdminShell.kt:133-139：hiddenNavIds 的判据即 anyDevicePluginRegistered() 的委托实现。 */
    private fun sidebarIds(state: DevicePluginNavState): List<String> {
        val hiddenNavIds = buildSet {
            if (!DevicePluginNav.showDeviceHub(state)) add(DevicePluginNav.DEVICE_HUB_NAV_ID)
        }
        return PluginSlots.mergeGrouped(AdminNav.grouped(macos = false), null)
            .flatMap { (_, items) -> items.filter { it.id !in hiddenNavIds } }
            .map { it.id }
    }

    /** 复刻 ConfigWorkspaceViews.kt:257-261：按组过滤 + 空组跳过。 */
    private fun globalMenuIds(state: DevicePluginNavState): List<String> =
        ConfigSections.globalGroups
            .map { (_, sections) -> DevicePluginNav.filterItems(sections, { it.id }, state) }
            .filter { it.isNotEmpty() }
            .flatMap { sections -> sections.map { it.id } }

    /** 复刻 ConfigWorkspaceViews.kt:132-134：按 node.sectionId 过滤。 */
    private fun startNodeIds(state: DevicePluginNavState): List<String> =
        DevicePluginNav.filterItems(
            PipelineWorkflow.positions.first { it.id == "start" }.nodes,
            { it.sectionId }, state,
        ).map { it.id }

    private val staticSidebarIds: List<String> get() = AdminNav.visible(macos = false).map { it.id }
    private val staticGlobalIds: List<String>
        get() = ConfigSections.globalGroups.flatMap { (_, sections) -> sections.map { it.id } }
    private val staticStartNodes
        get() = PipelineWorkflow.positions.first { it.id == "start" }.nodes

    @Test
    fun baselineActuallyContainsGatedItems() {
        // 前置断言：先证明三处静态基线里确实存在待隐藏项，否则后续「不含」断言会恒真。
        assertTrue("devicehub" in staticSidebarIds, "侧栏基线必须含 devicehub")
        assertTrue("tuya" in staticGlobalIds, "全局配置基线必须含 tuya")
        assertTrue(
            staticStartNodes.any { it.id == "echomuse.input" && it.sectionId == "echomuse" },
            "start 位置基线必须含 echomuse.input(sectionId=echomuse)",
        )
        assertTrue(staticStartNodes.any { it.id == "input" }, "start 位置基线必须含核心 input 节点")
    }

    @Test
    fun emptyShellHidesBusinessItemsAndKeepsCoreManagement() {
        // 严格相等：空壳只保留底座入口，所有业务 section 均须由插件声明。
        assertEquals(staticSidebarIds - "devicehub", sidebarIds(emptyShell))
        assertEquals(listOf("overview", "server"), globalMenuIds(emptyShell))
        assertEquals(
            staticStartNodes
                .filter { it.sectionId != "echomuse" && it.sectionId != "wakeup" }
                .map { it.id },
            startNodeIds(emptyShell),
        )
        // 显式点名核心项仍在（验收口径）；注意 echomuse 配置段本就不在全局配置菜单里，
        // 它由流程节点承载，故不对 globalMenuIds 断言「不含 echomuse」以免恒真。
        assertTrue(
            sidebarIds(emptyShell).containsAll(
                listOf("overview", "config", "settings", "plugins", "files"),
            ),
        )
        assertFalse("devicehub" in sidebarIds(emptyShell))
        assertFalse("tuya" in globalMenuIds(emptyShell))
        assertFalse("ota" in globalMenuIds(emptyShell))
        assertFalse("aiui" in globalMenuIds(emptyShell))
        assertFalse("presets" in globalMenuIds(emptyShell))
        assertFalse("echomuse.input" in startNodeIds(emptyShell))
    }

    @Test
    fun registeredBuildsExposeOnlyRegisteredPluginItems() {
        // 未知态 fail-closed：服务端尚未证明插件存在时不显示业务入口。
        assertFalse("devicehub" in sidebarIds(unknown))
        assertFalse("tuya" in globalMenuIds(unknown))
        assertFalse("echomuse.input" in startNodeIds(unknown))
        // 明确注册后，动态/兼容入口才可见。
        assertTrue("devicehub" in sidebarIds(bothRegistered))
        assertTrue("tuya" in globalMenuIds(bothRegistered))
        assertTrue("echomuse.input" in startNodeIds(bothRegistered))
        // 常规构建全关：未注册即不显示任何对应业务 UI。
        assertFalse("devicehub" in sidebarIds(bothDisabled))
        assertFalse("tuya" in globalMenuIds(bothDisabled))
        assertFalse("echomuse.input" in startNodeIds(bothDisabled))
    }

    @Test
    fun gatingIsStrictlyLimitedToBusinessSections() {
        // 白名单严格性：空壳下除「设备管理 + 业务 section」外，所有静态导航 id / 配置
        // section id / 流程 sectionId 的 showEntry 都必须为 true，避免误伤核心导航。
        val gated = setOf(
            "devicehub", "presets", "command", "wakeup", "soul", "memory",
            "aiui", "echomuse", "tuya", "ota",
        )
        val allIds = staticSidebarIds + staticGlobalIds + staticStartNodes.map { it.sectionId }
        for (id in allIds.distinct()) {
            assertEquals(id !in gated, DevicePluginNav.showEntry(emptyShell, id), "空壳下 $id 的门控判定")
        }
        // 系统级契约（nlu 识别链 / 下行音频 / MCP 工具注册表）不属于业务插件。
        assertTrue(DevicePluginNav.showEntry(emptyShell, "mcp"))
        assertTrue(DevicePluginNav.showEntry(emptyShell, "audio"))
        assertTrue(DevicePluginNav.showEntry(emptyShell, "nlu"))
    }
}
