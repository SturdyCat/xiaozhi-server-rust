package com.xiaozhi.admin

import com.xiaozhi.admin.contracts.PluginSlotSnapshot
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNotNull
import kotlin.test.assertTrue

/**
 * 回归：宿主对非 Active 插件下发 `"registration": null`（停用/待注册都如此）。
 *
 * 早期实现用 `?.jsonObject` 取该字段，遇到 JsonNull 会抛 IllegalArgumentException；
 * 调用方 [com.xiaozhi.admin.contracts.PluginSlotSnapshot] 外层被 runCatching 吞成 null，
 * 于是一条停用插件就让「插件」整页与动态导航菜单全部变空。
 */
class PluginSlotsNullRegistrationTest {
    private val wasmJson = """
        {"plugins":[
          {"id":"asr.official","title":"ASR 策略","description":"","version":"0.1.0",
           "official":true,"phase":{"state":"active"},"lifecycle":"active",
           "implements":["asr.provider"],"permissions":["kv"],
           "registration":{"menu":[{"slot":"nav.main","id":"asr-policy","label":"ASR 策略","group":"插件","order":110,"icon":"settings"}],
                           "config_fields":[],"actions":[{"id":"ping","label":"连通性自检","danger":false,"help":""}],"ui":null}},
          {"id":"hello.official","title":"Hello 探针","description":"","version":"0.1.2",
           "official":true,"phase":{"state":"inactive"},"lifecycle":"registered",
           "implements":[],"permissions":[],"requires":[],"dependents":[],
           "registration":null}
        ],
         "extensions":{"asr.provider":{"state":"resolved","implementation":"asr.official"}}}
    """.trimIndent()

    @Test
    fun nullRegistrationDoesNotBlankWholePluginList() {
        val snap = assertNotNull(PluginSlotSnapshot.parse(wasmJson))
        assertEquals(2, snap.plugins.size)
        assertEquals(1, snap.navEntries.size)
        assertEquals("asr.official", snap.extensions["asr.provider"]?.implementation)

        val idle = assertNotNull(snap.plugin("hello.official"))
        assertEquals("registered", idle.lifecycle)
        assertTrue(idle.menu.isEmpty())
        assertTrue(idle.actions.isEmpty())
        assertTrue(idle.configFields.isEmpty())
    }
}
