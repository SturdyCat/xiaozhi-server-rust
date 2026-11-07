package com.xiaozhi.admin

import com.xiaozhi.admin.contracts.PluginDetailModel
import com.xiaozhi.admin.contracts.PluginSlotSnapshot
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertNotNull
import kotlin.test.assertNull
import kotlin.test.assertTrue

/**
 * 插件生命周期契约的纯模型回归：解析缺键回退、requires/dependents 门控、
 * 仅非运行可移除、详情模型组装。全部 fail-closed 口径（服务端未就绪 = 旧服务端兼容）。
 */
class PluginLifecycleModelTest {

    /** 旧服务端形态：无 lifecycle/requires/dependents 键（回退 active、空依赖）。 */
    private val legacyJson = """
        {"plugins":[{"id":"hello.official","title":"Hello","description":"","version":"0.1.0",
         "official":true,"phase":{"state":"active"},"implements":[],"permissions":[],
         "registration":{"menu":[],"config_fields":[],"actions":[]}}]}
    """.trimIndent()

    /** staged：无 registration 投影（null）。 */
    private val stagedJson = """
        {"plugins":[{"id":"up.new","title":"新插件","description":"","version":"0.1.0","api_version":"0.1",
         "official":false,"phase":{"state":"unknown"},"implements":[],"permissions":[],
         "lifecycle":"staged","requires":[],"dependents":[]}]}
    """.trimIndent()

    /** registered：一条依赖满足、一条未满足（active_version null = 未装载）。 */
    private val registeredJson = """
        {"plugins":[{"id":"dep.b","title":"依赖方","description":"","version":"1.0.0","api_version":"0.1",
         "official":false,"phase":{"state":"unknown"},"implements":[],"permissions":[],
         "lifecycle":"registered",
         "requires":[{"id":"dep.a","min_version":"0.2.0","satisfied":true,"active_version":"0.2.1"},
                     {"id":"dep.c","min_version":"0.3.0","satisfied":false,"active_version":null}],
         "dependents":[]}]}
    """.trimIndent()

    /** active：被 dashboard.x 依赖（禁用门控应卡住）。 */
    private val activeDependentJson = """
        {"plugins":[{"id":"dep.a","title":"被依赖者","description":"","version":"0.2.1","api_version":"0.1",
         "official":true,"phase":{"state":"active"},"implements":[],"permissions":[],
         "lifecycle":"active","requires":[],"dependents":["dashboard.x"],
         "registration":{"menu":[{"slot":"nav.main","id":"a","label":"A","group":"插件","order":1,"icon":"settings"}],
                         "config_fields":[{"key":"src","label":"源","kind":"str","default_hint":"","help":"站点","hot":"live","secret":false,"required":false,"enum_values":[]}],
                         "actions":[{"id":"run","label":"运行","danger":false,"help":"点击运行"}]}}]}
    """.trimIndent()

    @Test
    fun missingLifecycleFallsBackToActiveLegacyCompat() {
        val snap = assertNotNull(PluginSlotSnapshot.parse(legacyJson))
        val p = assertNotNull(snap.plugin("hello.official"))
        assertNull(p.lifecycle, "缺键 = 旧服务端投影")
        assertTrue(p.isActive, "旧服务端缺 lifecycle → 视为 active 兼容")
        assertTrue(p.requires.isEmpty())
        assertTrue(p.dependents.isEmpty())
        // 旧服务端没有 deactivate/remove 端点：UI 走旧 unload 通道，不冒充新生命周期按钮
        assertFalse(p.canDeactivate, "lifecycle 缺键 ≠ 显式 active：禁用按钮不出现")
        assertFalse(p.canRemove, "旧服务端 active 插件不可走 remove 通道")
        assertFalse(p.canActivate)
    }

    @Test
    fun stagedHidesRegistrationSurfaceAndAllowsRegisterRemove() {
        val snap = assertNotNull(PluginSlotSnapshot.parse(stagedJson))
        val p = assertNotNull(snap.plugin("up.new"))
        assertEquals("staged", p.lifecycle)
        assertFalse(p.isActive)
        // registration 为 null：功能入口/字段/动作均为空（服务端仅 Active 投影）
        assertTrue(p.menu.isEmpty())
        assertTrue(p.actions.isEmpty())
        assertTrue(p.configFields.isEmpty())
        assertEquals("待注册", p.lifecycleLabel)
        assertTrue(p.canRemove, "staged 可移除")
        assertFalse(p.canDeactivate)
        assertFalse(p.canActivate, "staged 只显示注册按钮；激活门控只在 registered 态生效")
    }

    @Test
    fun registeredRequiresGateActivation() {
        val snap = assertNotNull(PluginSlotSnapshot.parse(registeredJson))
        val p = assertNotNull(snap.plugin("dep.b"))
        assertEquals("registered", p.lifecycle)
        assertEquals(2, p.requires.size)
        assertEquals(1, p.missingRequirements.size)
        val missing = p.missingRequirements.single()
        assertEquals("dep.c", missing.id)
        assertNull(missing.activeVersion, "active_version JSON null → null = 未激活/未装载")
        assertEquals("未激活/未装载", missing.currentState)
        assertFalse(p.canActivate, "存在未满足依赖 → 激活不可用")
        assertTrue(p.canRemove)
        assertFalse(p.canDeactivate)
    }

    @Test
    fun activeWithDependentsBlocksDeactivate() {
        val snap = assertNotNull(PluginSlotSnapshot.parse(activeDependentJson))
        val p = assertNotNull(snap.plugin("dep.a"))
        assertTrue(p.isActive)
        assertEquals(listOf("dashboard.x"), p.dependents)
        assertFalse(p.canDeactivate, "被激活插件依赖 → 禁用不可用")
        assertFalse(p.canRemove, "active 不可移除")
        assertFalse(p.canActivate)
        assertEquals("运行中", p.lifecycleLabel)
    }

    @Test
    fun activeWithoutDependentsAllowsDeactivate() {
        val json = activeDependentJson.replace("\"dependents\":[\"dashboard.x\"]", "\"dependents\":[]")
        val p = assertNotNull(PluginSlotSnapshot.parse(json)?.plugin("dep.a"))
        assertTrue(p.canDeactivate)
    }

    @Test
    fun detailModelAssemblesActiveSectionsAndStagedFallback() {
        val active = assertNotNull(PluginSlotSnapshot.parse(activeDependentJson)?.plugin("dep.a"))
        val activeModel = PluginDetailModel.from(active, listOf("A 页（config）"))
        assertTrue(activeModel.sections.any { it.title == "功能入口" && it.lines.contains("A（nav.main）") })
        assertTrue(activeModel.sections.any { it.title == "配置项" && it.lines.single().contains("源") })
        assertTrue(activeModel.sections.any { it.title == "动作与用法" && it.lines.single().contains("点击运行") })
        assertTrue(activeModel.sections.any { it.title == "动态页面" })
        assertTrue(activeModel.sections.none { it.title == "注册状态" })

        val staged = assertNotNull(PluginSlotSnapshot.parse(stagedJson)?.plugin("up.new"))
        val stagedModel = PluginDetailModel.from(staged, emptyList())
        assertTrue(stagedModel.sections.none { it.title == "功能入口" }, "未激活不投影 registration 摘要")
        assertTrue(
            stagedModel.sections.any { it.title == "注册状态" },
            "未激活时明确说明摘要缺席原因，不伪造空节",
        )
        assertTrue(stagedModel.lifecycleLine.contains("待注册"))
    }

    @Test
    fun detailModelExplainsUnsatisfiedDependencyAndDependents() {
        val registered = assertNotNull(PluginSlotSnapshot.parse(registeredJson)?.plugin("dep.b"))
        val model = PluginDetailModel.from(registered, emptyList())
        val dep = assertNotNull(model.sections.firstOrNull { it.title == "依赖" })
        assertTrue(dep.lines.any { it.contains("dep.a") && it.contains("已满足") && it.contains("0.2.1") })
        assertTrue(dep.lines.any { it.contains("dep.c") && it.contains("未满足") && it.contains("未激活/未装载") })

        val active = assertNotNull(PluginSlotSnapshot.parse(activeDependentJson)?.plugin("dep.a"))
        val activeModel = PluginDetailModel.from(active, emptyList())
        val depLine = assertNotNull(activeModel.sections.firstOrNull { it.title == "依赖" })
        assertTrue(depLine.lines.any { it.contains("dashboard.x") && it.contains("先停用依赖者") })
    }

    @Test
    fun apiVersionParsedForDetailHeader() {
        val staged = assertNotNull(PluginSlotSnapshot.parse(stagedJson)?.plugin("up.new"))
        assertEquals("0.1", staged.apiVersion)
        val legacy = assertNotNull(PluginSlotSnapshot.parse(legacyJson)?.plugin("hello.official"))
        assertEquals("", legacy.apiVersion, "旧服务端缺键 = 空串（详情头省略契约段）")
    }
}
