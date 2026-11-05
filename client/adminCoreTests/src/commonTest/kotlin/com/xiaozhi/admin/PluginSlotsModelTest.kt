package com.xiaozhi.admin

import com.xiaozhi.admin.contracts.AdminNav
import com.xiaozhi.admin.contracts.NavDescriptor
import com.xiaozhi.admin.contracts.PluginSlots
import com.xiaozhi.admin.contracts.PluginSlotSnapshot
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNotNull
import kotlin.test.assertNull
import kotlin.test.assertTrue

/**
 * P2e：wasm 插件槽位投影与导航合并的纯模型回归（零 Kuikly，回环 JSON 样本）。
 * 服务端不可达 / 无 wasm 键 = 静态基线；有键 = 追加「插件」组且不污染既有项。
 */
class PluginSlotsModelTest {
    private val wasmJson = """
        {"plugins":[
          {"id":"hello.official","title":"Hello 演示插件","description":"演示","version":"0.1.0",
           "official":true,"phase":{"state":"active"},"implements":["demo.greeter"],"permissions":["kv"],
           "registration":{"menu":[{"slot":"nav.main","id":"hello","label":"Hello","group":"插件","order":100,"icon":"settings"},
                                    {"slot":"config.section","id":"hello-cfg","label":"Hello 配置","group":"插件","order":1,"icon":"settings"}],
                          "config_fields":[{"key":"greeting","label":"问候语","kind":"str","default_hint":"你好","help":"","hot":"live","secret":false,"required":false,"enum_values":[]}],
                          "actions":[{"id":"greet","label":"打招呼","danger":false,"help":""}]}},
          {"id":"greeter.usermade","title":"问候（用户）","description":"","version":"0.2.0",
           "official":false,"phase":{"state":"degraded"},"implements":[],"permissions":[],
           "registration":{"menu":[{"slot":"nav.main","id":"user-greeter","label":"问候(用户)","group":"插件","order":101,"icon":"settings"}],
                          "config_fields":[],"actions":[]}}
        ],
         "extensions":{"demo.greeter":{"state":"resolved","implementation":"hello.official"}}}
    """.trimIndent()

    @Test
    fun missingWasmKeyFallsBackToNull() {
        assertNull(PluginSlotSnapshot.parse(null))
        assertNull(PluginSlotSnapshot.parse(""))
        assertNull(PluginSlotSnapshot.parse("{}"))
        assertNull(PluginSlotSnapshot.parse("not json"))
    }

    @Test
    fun parseExtractsPluginsNavExtensions() {
        val snap = assertNotNull(PluginSlotSnapshot.parse(wasmJson))
        assertEquals(2, snap.plugins.size)
        assertEquals("active", snap.plugin("hello.official")?.phase)
        assertEquals(1, snap.plugin("hello.official")?.actions?.size)
        assertEquals("问候语", snap.plugin("hello.official")?.configFields?.first()?.label)
        // nav.main 槽位 2 项；config.section 不进主导航
        assertEquals(2, snap.navEntries.size)
        assertEquals("resolved", snap.extensions["demo.greeter"]?.state)
    }

    @Test
    fun mergeNavAppendsPluginGroupWithoutPollutingStatics() {
        val staticItems = AdminNav.visible(macos = false)
        val snap = assertNotNull(PluginSlotSnapshot.parse(wasmJson))
        val merged = PluginSlots.mergeNav(staticItems, snap)
        // 静态项原样在前，动态项追加为「插件」组
        assertEquals(staticItems, merged.take(staticItems.size))
        val dynamic = merged.drop(staticItems.size)
        assertEquals(2, dynamic.size)
        assertTrue(dynamic.all { it.group == PluginSlots.PLUGIN_GROUP })
        // order 9000 带宽：插件项排在所有静态项之后，组内按 order 升序
        assertTrue(dynamic.all { it.order >= 9000 })
        assertEquals(
            listOf("Hello", "问候(用户)"),
            dynamic.map { it.label },
        )
        // 动态 id 可反查插件
        assertEquals("hello.official", PluginSlots.pluginIdFromNavigation(dynamic[0].id))
        assertNull(PluginSlots.pluginIdFromNavigation("overview"))
    }

    @Test
    fun mergeGroupedKeepsPluginEntriesContiguousAndNullKeepsBaseline() {
        val staticGroups = AdminNav.grouped(macos = false)
        val snap = assertNotNull(PluginSlotSnapshot.parse(wasmJson))
        val groups = PluginSlots.mergeGrouped(staticGroups, snap)
        // 静态组原样在前；「插件」组单独成段（组内相邻）
        assertEquals(staticGroups, groups.take(staticGroups.size))
        assertEquals(PluginSlots.PLUGIN_GROUP, groups.last().first)
        assertEquals(2, groups.last().second.size)
        // 无快照 = 静态基线逐项一致
        assertEquals(staticGroups, PluginSlots.mergeGrouped(staticGroups, null))
    }

    @Test
    fun duplicateNavigationIdsAreDroppedAndOrdersCoerced() {
        val snapshotJson = """
            {"plugins":[
              {"id":"p.a","title":"A","description":"","version":"","official":false,"phase":{"state":"active"},"implements":[],"permissions":[],
               "registration":{"menu":[{"slot":"nav.main","id":"dup","label":"A1","group":"插件","order":100000,"icon":"settings"}],"config_fields":[],"actions":[]}},
              {"id":"p.b","title":"B","description":"","version":"","official":false,"phase":{"state":"active"},"implements":[],"permissions":[],
               "registration":{"menu":[{"slot":"nav.main","id":"dup","label":"B1","group":"插件","order":2,"icon":"settings"}],"config_fields":[],"actions":[]}}
            ]}
        """.trimIndent()
        val snap = assertNotNull(PluginSlotSnapshot.parse(snapshotJson))
        // navigationId = plugin:<pluginId>:<entryId>：与静态项精确同 id 才去重
        //（跨插件的同名 entryId 不是冲突）；越界 order 钳到 9000..9999 带宽
        val staticItems = listOf(NavDescriptor("plugin:p.a:dup", 0, "既有同名入口", "存储"))
        val merged = PluginSlots.mergeNav(staticItems, snap)
        assertEquals(staticItems, merged.take(staticItems.size))
        assertEquals(1, merged.size - staticItems.size, "p.a 撞静态被丢，p.b 保留")
        assertEquals("B1", merged.last().label)
        val both = PluginSlots.mergeNav(emptyList(), snap)
        assertEquals(2, both.size, "跨插件不同 id 都保留")
        assertEquals(listOf(9002, 9999), both.map { it.order }, "钳到 9000 带宽且升序")
    }
}
