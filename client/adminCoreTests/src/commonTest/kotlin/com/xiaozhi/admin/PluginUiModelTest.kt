package com.xiaozhi.admin

import com.xiaozhi.admin.contracts.PluginAction
import com.xiaozhi.admin.contracts.PluginConfigField
import com.xiaozhi.admin.contracts.PluginUiDocument
import com.xiaozhi.admin.contracts.PluginUiSlots
import com.xiaozhi.admin.contracts.PluginSlotSnapshot
import com.xiaozhi.admin.contracts.PluginUiNode
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.jsonObject
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertIs
import kotlin.test.assertNotNull
import kotlin.test.assertNull
import kotlin.test.assertTrue

/**
 * 动态插件 UI 文档（A2UI v0.1）纯模型回归：fail-closed 解析规则 + `PluginUiSlots` 对
 * `wasm` 键投影的一次性解析。零 Kuikly，全部回环 JSON 样本。
 *
 * 本文件钉住**旧扁平形态的兼容**（tree.form 引用配置字段、tree.panel 引用动作）与
 * `PluginUiSlots` 解析链；嵌套组件树（card/column/text/hint/field/actions/input/result、
 * 深度/预算/引用 fail-closed）的规则回归在 [PluginUiTreeTest]。
 */
class PluginUiModelTest {

    /** 与 PluginSlotsModelTest 同源的声明白名单（字段 + 动作）。 */
    private val fields = listOf(
        PluginConfigField(
            key = "greeting", label = "问候语", kind = "str",
            help = "唤醒后说的话", defaultHint = "你好", secret = false,
        ),
        PluginConfigField(
            key = "api_key", label = "密钥", kind = "password",
            help = "", defaultHint = "", secret = true,
        ),
    )
    private val actions = listOf(
        PluginAction(id = "greet", label = "打招呼", danger = false, help = ""),
        PluginAction(id = "reset", label = "重置数据", danger = true, help = "不可恢复"),
    )

    /** hello 夹具同源的双页样本（config + test；文档内 label 是展示副本，解析后以声明为准）。 */
    private val helloDoc: JsonObject = JsonObject(
        mapOf(
            "a2ui" to JsonPrimitive("0.1"),
            "pages" to JsonArray(
                listOf(
                    JsonObject(
                        mapOf(
                            "id" to JsonPrimitive("hello-config"),
                            "title" to JsonPrimitive("Hello 配置"),
                            "kind" to JsonPrimitive("config"),
                            "tree" to JsonObject(
                                mapOf(
                                    "type" to JsonPrimitive("form"),
                                    // 文档自带的 kind/label 是展示副本：解析结果必须取注册声明
                                    "fields" to JsonArray(
                                        listOf(
                                            JsonObject(
                                                mapOf(
                                                    "key" to JsonPrimitive("greeting"),
                                                    "kind" to JsonPrimitive("文档想伪装的类型"),
                                                    "label" to JsonPrimitive("文档想改的文案"),
                                                ),
                                            ),
                                        ),
                                    ),
                                ),
                            ),
                            "bindings" to JsonObject(
                                mapOf("config" to JsonArray(listOf(JsonPrimitive("greeting")))),
                            ),
                        ),
                    ),
                    JsonObject(
                        mapOf(
                            "id" to JsonPrimitive("hello-test"),
                            "title" to JsonPrimitive("Hello 连通性测试"),
                            "kind" to JsonPrimitive("test"),
                            "tree" to JsonObject(
                                mapOf(
                                    "type" to JsonPrimitive("panel"),
                                    "actions" to JsonArray(
                                        listOf(
                                            JsonObject(
                                                mapOf(
                                                    "action" to JsonPrimitive("greet"),
                                                    "label" to JsonPrimitive("文档想改的动作文案"),
                                                ),
                                            ),
                                        ),
                                    ),
                                ),
                            ),
                            "bindings" to JsonObject(
                                mapOf("actions" to JsonArray(listOf(JsonPrimitive("greet")))),
                            ),
                        ),
                    ),
                ),
            ),
        ),
    )

    // ============================================================
    // 整份作废（→ 声明式回退页）
    // ============================================================

    @Test
    fun malformedOrUnknownVersionFallsBackToNull() {
        assertNull(PluginUiDocument.parse(null, fields, actions))
        assertNull(PluginUiDocument.parse(JsonObject(emptyMap()), fields, actions))
        // 缺版本 / 版本不是 0.1：不猜形状，整份作废
        assertNull(
            PluginUiDocument.parse(
                JsonObject(mapOf("pages" to JsonArray(emptyList()))), fields, actions,
            ),
        )
        assertNull(
            PluginUiDocument.parse(
                JsonObject(
                    mapOf(
                        "a2ui" to JsonPrimitive("0.2"),
                        "pages" to JsonArray(emptyList()),
                    ),
                ),
                fields, actions,
            ),
        )
        assertNull(
            PluginUiDocument.parse(
                JsonObject(mapOf("a2ui" to JsonPrimitive("0.1"), "pages" to JsonArray(emptyList()))),
                fields, actions,
            ),
            "零有效页 = 没有可渲染内容",
        )
    }

    // ============================================================
    // 正常解析：引用解析到声明元数据（文档副本不改写声明）
    // ============================================================

    @Test
    fun helloSampleParsesWithRegistrationMetadata() {
        val document = assertNotNull(PluginUiDocument.parse(helloDoc, fields, actions))
        assertEquals("0.1", document.version)
        assertEquals(listOf("hello-config", "hello-test"), document.pages.map { it.id })
        val config = document.pages[0]
        assertEquals("Hello 配置", config.title)
        assertEquals("config", config.kind)
        val form = assertIs<PluginUiNode.Form>(config.tree)
        // 字段元数据来自注册声明：文档里伪装的 label/kind 不生效
        assertEquals("问候语", form.fields.single().label)
        assertEquals("str", form.fields.single().kind)
        val test = document.pages[1]
        assertEquals("test", test.kind)
        val panel = assertIs<PluginUiNode.Panel>(test.tree)
        // 动作元数据来自注册声明（label/danger）；文档只提供 id 引用
        assertEquals("打招呼", panel.actions.single().label)
        assertTrue(!panel.actions.single().danger)
    }

    @Test
    fun secretFieldResolvesWithSecretNote() {
        val document = assertNotNull(
            PluginUiDocument.parse(
                JsonObject(
                    mapOf(
                        "a2ui" to JsonPrimitive("0.1"),
                        "pages" to JsonArray(
                            listOf(
                                JsonObject(
                                    mapOf(
                                        "id" to JsonPrimitive("secret"),
                                        "tree" to JsonObject(
                                            mapOf(
                                                "type" to JsonPrimitive("form"),
                                                "fields" to JsonArray(
                                                    listOf(JsonObject(mapOf("key" to JsonPrimitive("api_key")))),
                                                ),
                                            ),
                                        ),
                                    ),
                                ),
                            ),
                        ),
                    ),
                ),
                fields, actions,
            ),
        )
        val form = assertIs<PluginUiNode.Form>(document.pages.single().tree)
        assertTrue(form.fields.single().secret)
    }

    // ============================================================
    // fail-closed：未声明引用丢弃、未知类型跳页、重复 id 去重
    // ============================================================

    @Test
    fun undeclaredReferencesAreDropped() {
        val document = assertNotNull(
            PluginUiDocument.parse(
                JsonObject(
                    mapOf(
                        "a2ui" to JsonPrimitive("0.1"),
                        "pages" to JsonArray(
                            listOf(
                                JsonObject(
                                    mapOf(
                                        "id" to JsonPrimitive("mixed"),
                                        "tree" to JsonObject(
                                            mapOf(
                                                "type" to JsonPrimitive("form"),
                                                "fields" to JsonArray(
                                                    listOf(
                                                        JsonObject(mapOf("key" to JsonPrimitive("not_declared"))),
                                                        JsonObject(mapOf("key" to JsonPrimitive("greeting"))),
                                                        JsonObject(mapOf("key" to JsonPrimitive("api_key"))),
                                                    ),
                                                ),
                                            ),
                                        ),
                                    ),
                                ),
                                JsonObject(
                                    mapOf(
                                        "id" to JsonPrimitive("bad-panel"),
                                        "tree" to JsonObject(
                                            mapOf(
                                                "type" to JsonPrimitive("panel"),
                                                "actions" to JsonArray(
                                                    listOf(JsonObject(mapOf("action" to JsonPrimitive("nope")))),
                                                ),
                                            ),
                                        ),
                                    ),
                                ),
                            ),
                        ),
                    ),
                ),
                fields, actions,
            ),
        )
        // 未声明字段被滤掉（保留声明过的两个，按文档顺序）；全未引用的 panel 页被丢弃
        assertEquals(listOf("mixed"), document.pages.map { it.id })
        val form = assertIs<PluginUiNode.Form>(document.pages.single().tree)
        assertEquals(listOf("greeting", "api_key"), form.fields.map { it.key })
    }

    @Test
    fun unknownTreeTypeAndDuplicateIdsAreSkipped() {
        val document = assertNotNull(
            PluginUiDocument.parse(
                JsonObject(
                    mapOf(
                        "a2ui" to JsonPrimitive("0.1"),
                        "pages" to JsonArray(
                            listOf(
                                JsonObject(
                                    mapOf(
                                        "id" to JsonPrimitive("video"),
                                        "tree" to JsonObject(mapOf("type" to JsonPrimitive("video"))),
                                    ),
                                ),
                                JsonObject(
                                    mapOf(
                                        "id" to JsonPrimitive("hint"),
                                        "tree" to JsonObject(
                                            mapOf(
                                                "type" to JsonPrimitive("panel"),
                                                "actions" to JsonArray(
                                                    listOf(JsonObject(mapOf("action" to JsonPrimitive("greet")))),
                                                ),
                                            ),
                                        ),
                                    ),
                                ),
                                // 与已有页同 id：丢弃后者
                                JsonObject(
                                    mapOf(
                                        "id" to JsonPrimitive("hint"),
                                        "tree" to JsonObject(
                                            mapOf(
                                                "type" to JsonPrimitive("panel"),
                                                "actions" to JsonArray(
                                                    listOf(JsonObject(mapOf("action" to JsonPrimitive("reset")))),
                                                ),
                                            ),
                                        ),
                                    ),
                                ),
                            ),
                        ),
                    ),
                ),
                fields, actions,
            ),
        )
        assertEquals(listOf("hint"), document.pages.map { it.id })
        assertEquals(
            listOf("greet"),
            assertIs<PluginUiNode.Panel>(document.pages.single().tree).actions.map { it.id },
        )
    }

    // ============================================================
    // 预算截断
    // ============================================================

    @Test
    fun pageAndLeafBudgetsTruncateInDocumentOrder() {
        fun page(id: String): JsonObject = JsonObject(
            mapOf(
                "id" to JsonPrimitive(id),
                "tree" to JsonObject(
                    mapOf(
                        "type" to JsonPrimitive("panel"),
                        "actions" to JsonArray(
                            listOf(JsonObject(mapOf("action" to JsonPrimitive("greet")))),
                        ),
                    ),
                ),
            ),
        )
        val many = (0 until PluginUiDocument.MAX_PAGES + 3).map { page("p$it") }
        val document = assertNotNull(
            PluginUiDocument.parse(
                JsonObject(
                    mapOf("a2ui" to JsonPrimitive("0.1"), "pages" to JsonArray(many)),
                ),
                fields, actions,
            ),
        )
        assertEquals(PluginUiDocument.MAX_PAGES, document.pages.size)
        assertEquals("p0", document.pages.first().id)
    }

    // ============================================================
    // PluginUiSlots：wasm 键投影的一次性解析
    // ============================================================

    /** 带 ui 的 wasm 清单（hello.official Active 带 ui；greeter.usermade Degraded 投 null）。 */
    private val wasmJson = """
        {"plugins":[
          {"id":"hello.official","title":"Hello","description":"","version":"0.1.0",
           "official":true,"phase":{"state":"active"},"implements":[],"permissions":[],
           "registration":{"menu":[],
                           "config_fields":[{"key":"greeting","label":"问候语","kind":"str","default_hint":"","help":"","hot":"live","secret":false,"required":false,"enum_values":[]}],
                           "actions":[{"id":"greet","label":"打招呼","danger":false,"help":""}],
                           "ui":{"a2ui":"0.1","pages":[{"id":"hello-config","title":"Hello 配置","kind":"config","tree":{"type":"form","fields":[{"key":"greeting"}]}}]}}},
          {"id":"greeter.usermade","title":"问候（用户）","description":"","version":"0.2.0",
           "official":false,"phase":{"state":"degraded"},"implements":[],"permissions":[],
           "registration":{"menu":[],"config_fields":[],"actions":[],
                           "ui":null}}
        ]}
    """.trimIndent()

    @Test
    fun slotsParseOnlyActiveDeclaredDocuments() {
        val snapshot = assertNotNull(PluginSlotSnapshot.parse(wasmJson))
        val docs = PluginUiSlots.parse(wasmJson, snapshot)
        // Active + 声明了合法 ui → 解析成功；Degraded 投影 null → 缺席（回退页）
        assertEquals(setOf("hello.official"), docs.keys)
        val form = assertIs<PluginUiNode.Form>(docs["hello.official"]!!.pages.single().tree)
        assertEquals("greeting", form.fields.single().key)
    }

    @Test
    fun slotsParseFailsClosedOnBrokenInputs() {
        val snapshot = assertNotNull(PluginSlotSnapshot.parse(wasmJson))
        assertTrue(PluginUiSlots.parse(null, snapshot).isEmpty())
        assertTrue(PluginUiSlots.parse("", snapshot).isEmpty())
        assertTrue(PluginUiSlots.parse("not json", snapshot).isEmpty())
        assertTrue(PluginUiSlots.parse(wasmJson, null).isEmpty(), "无快照 = 无声明白名单，动态 UI 一并不可用")
        // 文档形状不合规（缺 a2ui 版本）→ 该插件缺席，不影响其他键
        val badUi = wasmJson.replace("\"ui\":{\"a2ui\":\"0.1\",", "\"ui\":{")
        assertTrue(PluginUiSlots.parse(badUi, assertNotNull(PluginSlotSnapshot.parse(badUi))).isEmpty())
    }

    @Test
    fun documentParsesFromRealWasmTextProjection() {
        // 端到端口径：清单文本 → registration.ui 对象 → 文档（模拟 PluginMetaState 单次解析链）
        val wasm = Json.parseToJsonElement(wasmJson).jsonObject
        val registration = (wasm["plugins"] as JsonArray)[0]
            .let { it as JsonObject }["registration"]!!
            .let { it as JsonObject }
        val document = assertNotNull(
            PluginUiDocument.parse(
                registration["ui"] as? JsonObject,
                fields, actions,
            ),
        )
        assertEquals(1, document.pages.size)
    }
}
