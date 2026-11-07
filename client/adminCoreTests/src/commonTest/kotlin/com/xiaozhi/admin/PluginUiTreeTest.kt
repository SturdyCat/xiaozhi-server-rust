package com.xiaozhi.admin

import com.xiaozhi.admin.contracts.PluginAction
import com.xiaozhi.admin.contracts.PluginConfigField
import com.xiaozhi.admin.contracts.PluginUiDocument
import com.xiaozhi.admin.contracts.PluginUiNode
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertIs
import kotlin.test.assertNotNull
import kotlin.test.assertNull
import kotlin.test.assertTrue

/**
 * 动态插件 UI 文档**嵌套组件树**（A2UI v0.1 超集语法）纯模型回归：容器/叶子的
 * fail-closed 解析规则。零 Kuikly，全部回环 JSON 样本。
 *
 * 钉住的契约（与 contracts/PluginUi.kt 同步）：容器只有 card/column；叶子全部业务无关
 * （text/hint ≤500 字符、field/form 引用注册字段、actions 引用注册动作、input 页内唯一、
 * result 占位）；容器深 ≤4、每页叶子 ≤64、页 ≤8；未知 type 丢节点不丢页；引用元数据
 * 一律取注册声明；全丢 → null（声明式回退页）。旧扁平 form/panel 的兼容回归在
 * [PluginUiModelTest]。
 */
class PluginUiTreeTest {

    private val fields = listOf(
        PluginConfigField(
            key = "greeting", label = "问候语", kind = "str",
            help = "唤醒后说的话", defaultHint = "你好", secret = false,
        ),
    )
    private val actions = listOf(
        PluginAction(id = "greet", label = "打招呼", danger = false, help = ""),
        PluginAction(id = "reset", label = "重置数据", danger = true, help = "不可恢复"),
    )

    private fun parse(tree: JsonObject): PluginUiDocument? =
        PluginUiDocument.parse(doc(listOf(page("p", tree))), fields, actions)

    // ---- 样本零件（与服务端 wasm-ui-contract 的节点形状一一对应） ----

    private fun doc(pages: List<JsonObject>): JsonObject = JsonObject(
        mapOf("a2ui" to JsonPrimitive("0.1"), "pages" to JsonArray(pages)),
    )

    private fun page(id: String, tree: JsonObject): JsonObject = JsonObject(
        mapOf(
            "id" to JsonPrimitive(id),
            "title" to JsonPrimitive(id),
            "kind" to JsonPrimitive("config"),
            "tree" to tree,
        ),
    )

    private fun text(t: String): JsonObject = JsonObject(
        mapOf("type" to JsonPrimitive("text"), "text" to JsonPrimitive(t)),
    )

    private fun hint(t: String): JsonObject = JsonObject(
        mapOf("type" to JsonPrimitive("hint"), "text" to JsonPrimitive(t)),
    )

    private fun card(title: String, child: JsonObject?): JsonObject {
        val m = mutableMapOf<String, kotlinx.serialization.json.JsonElement>(
            "type" to JsonPrimitive("card"),
        )
        if (title.isNotEmpty()) m["title"] = JsonPrimitive(title)
        if (child != null) m["child"] = child
        return JsonObject(m)
    }

    private fun column(children: List<JsonObject>): JsonObject = JsonObject(
        mapOf("type" to JsonPrimitive("column"), "children" to JsonArray(children)),
    )

    private fun fieldNode(key: String): JsonObject = JsonObject(
        mapOf("type" to JsonPrimitive("field"), "key" to JsonPrimitive(key)),
    )

    private fun actionsNode(ids: List<String>): JsonObject = JsonObject(
        mapOf("type" to JsonPrimitive("actions"), "actions" to JsonArray(ids.map { JsonPrimitive(it) })),
    )

    private fun inputNode(id: String, label: String = "输入", placeholder: String? = null): JsonObject {
        val m = mutableMapOf<String, kotlinx.serialization.json.JsonElement>(
            "type" to JsonPrimitive("input"),
            "id" to JsonPrimitive(id),
            "label" to JsonPrimitive(label),
        )
        if (placeholder != null) m["placeholder"] = JsonPrimitive(placeholder)
        return JsonObject(m)
    }

    private fun resultNode(): JsonObject = JsonObject(mapOf("type" to JsonPrimitive("result")))

    private fun cardChain(depth: Int, leaf: JsonObject): JsonObject {
        var node = leaf
        repeat(depth) { i -> node = card("L$i", node) }
        return node
    }

    // ============================================================
    // 正常解析：嵌套容器 + 全叶子类型
    // ============================================================

    @Test
    fun nestedTreeParsesContainersLeavesAndInputs() {
        val tree = column(
            listOf(
                card("说明卡", column(listOf(text("正文"), hint("次要说明")))),
                fieldNode("greeting"),
                actionsNode(listOf("greet", "reset")),
                inputNode("prompt", label = "测试输入", placeholder = "说点什么"),
                resultNode(),
            ),
        )
        val document = assertNotNull(parse(tree))
        val col = assertIs<PluginUiNode.Column>(document.pages.single().tree)
        assertEquals(5, col.children.size)
        // 容器递归：card > column > text/hint
        val inner = assertIs<PluginUiNode.Column>(
            assertIs<PluginUiNode.Card>(col.children[0]).child,
        )
        assertEquals("正文", assertIs<PluginUiNode.Text>(inner.children[0]).text)
        assertEquals("次要说明", assertIs<PluginUiNode.Hint>(inner.children[1]).text)
        // field 引用解析到注册声明（文档只给 key）
        assertEquals("问候语", assertIs<PluginUiNode.Field>(col.children[1]).field.label)
        // actions 引用解析到注册声明（label/danger 取声明，不取文档）
        val acts = assertIs<PluginUiNode.Actions>(col.children[2])
        assertEquals(listOf("打招呼", "重置数据"), acts.actions.map { it.label })
        assertTrue(acts.actions[1].danger)
        // input：id 收集进 page.inputIds（动作 payload 组装用），placeholder 可选
        val page = document.pages.single()
        assertEquals(setOf("prompt"), page.inputIds)
        val input = assertIs<PluginUiNode.Input>(col.children[3])
        assertEquals("prompt", input.id)
        assertEquals("说点什么", input.placeholder)
        assertIs<PluginUiNode.Result>(col.children[4])
    }

    // ============================================================
    // 容器深度 ≤4：第 5 层容器整体丢弃
    // ============================================================

    @Test
    fun containerDepthIsCappedAtFour() {
        // 4 层容器 + 叶子：合法（容器链长度恰为上限）
        val ok = assertNotNull(parse(cardChain(4, text("hi"))))
        var node: PluginUiNode = ok.pages.single().tree
        repeat(4) { node = assertIs<PluginUiNode.Card>(node).child }
        assertEquals("hi", assertIs<PluginUiNode.Text>(node).text)
        // 5 层容器：最内层容器丢弃并向上级联（card 无有效 child）→ 树为空 → 整页丢弃
        assertNull(parse(cardChain(5, text("hi"))))
        // 混排：超深分支丢弃不牵连同级兄弟
        val mixed = assertNotNull(
            parse(column(listOf(cardChain(5, text("deep")), text("kept")))),
        )
        val col = assertIs<PluginUiNode.Column>(mixed.pages.single().tree)
        assertEquals(listOf("kept"), col.children.map { assertIs<PluginUiNode.Text>(it).text })
    }

    // ============================================================
    // text/hint ≤500 字符：超出丢弃该节点（不截断）
    // ============================================================

    @Test
    fun textNodesOverLimitAreDropped() {
        assertEquals("x".repeat(500), assertIs<PluginUiNode.Text>(parse(text("x".repeat(500)))!!.pages.single().tree).text)
        assertNull(parse(text("x".repeat(501))), "超限 text 丢弃 → 页内无叶子 → 丢页")
        assertNull(parse(hint("x".repeat(501))), "超限 hint 同规则")
        // card 唯一孩子超限 → card 无有效 child → 一并丢弃
        assertNull(parse(card("t", text("x".repeat(501)))))
    }

    // ============================================================
    // 未知 type 丢节点不丢页；空容器丢弃
    // ============================================================

    @Test
    fun unknownNodesAndEmptyContainersAreDropped() {
        val video = JsonObject(mapOf("type" to JsonPrimitive("video")))
        // 兄弟存活：未知孩子只丢自己（向前兼容，单个未知组件不作废文档）
        val mixed = assertNotNull(parse(column(listOf(video, text("kept")))))
        val col = assertIs<PluginUiNode.Column>(mixed.pages.single().tree)
        assertEquals(1, col.children.size)
        // 未知根 → 丢页；好页保留
        val document = assertNotNull(
            PluginUiDocument.parse(
                doc(listOf(page("bad", video), page("good", text("ok")))),
                fields, actions,
            ),
        )
        assertEquals(listOf("good"), document.pages.map { it.id })
        // 空 card（缺 child / 缺 title / child 无效）与空 column 全部丢弃
        val badTrees = listOf(
            card("no-child", null),
            card("", text("t")),
            card("child-unknown", video),
            column(emptyList()),
            column(listOf(video)),
        )
        for (bad in badTrees) {
            assertNull(parse(bad), "空容器必须丢弃：$bad")
        }
    }

    // ============================================================
    // input：id 形状 + 页内唯一 fail-closed
    // ============================================================

    @Test
    fun inputNodesValidateShapeAndPageUniqueness() {
        val tree = column(
            listOf(
                inputNode("ok-1"),
                inputNode("UPPER"),            // 大写违规 → 丢弃
                inputNode("with space"),       // 非法字符 → 丢弃
                inputNode("a".repeat(33)),     // 超 32 字符 → 丢弃
                inputNode(""),                 // 空 id → 丢弃
                inputNode("ok-1"),             // 页内重复 → 第二个丢弃
                JsonObject(                    // 缺 label → 丢弃
                    mapOf("type" to JsonPrimitive("input"), "id" to JsonPrimitive("nolabel")),
                ),
            ),
        )
        val document = assertNotNull(parse(tree))
        assertEquals(setOf("ok-1"), document.pages.single().inputIds)
        val col = assertIs<PluginUiNode.Column>(document.pages.single().tree)
        assertEquals(1, col.children.size, "只有合法且唯一的 input 存活")
        assertEquals("输入", assertIs<PluginUiNode.Input>(col.children.single()).label)
    }

    // ============================================================
    // 树内引用 fail-closed：未声明引用逐条丢弃，元数据取注册声明
    // ============================================================

    @Test
    fun referencesInsideTreeFailClosed() {
        val tree = column(
            listOf(
                card("字段卡", column(listOf(fieldNode("ghost"), fieldNode("greeting")))),
                actionsNode(listOf("ghost", "greet", "reset")),
            ),
        )
        val document = assertNotNull(parse(tree))
        val col = assertIs<PluginUiNode.Column>(document.pages.single().tree)
        val fieldCol = assertIs<PluginUiNode.Column>(
            assertIs<PluginUiNode.Card>(col.children[0]).child,
        )
        assertEquals(listOf("greeting"), fieldCol.children.map { assertIs<PluginUiNode.Field>(it).field.key })
        val acts = assertIs<PluginUiNode.Actions>(col.children[1])
        assertEquals(listOf("greet", "reset"), acts.actions.map { it.id })
        // 全部引用未命中 → 该节点丢弃 → 页内无叶子 → 丢页（文档 null）
        assertNull(parse(actionsNode(listOf("ghost"))))
        assertNull(parse(fieldNode("ghost")))
    }

    // ============================================================
    // 叶子预算：嵌套树按文档顺序截断到每页 64
    // ============================================================

    @Test
    fun leafBudgetTruncatesNestedLeaves() {
        val tree = column((1..70).map { i -> text("t$i") })
        val document = assertNotNull(parse(tree))
        val col = assertIs<PluginUiNode.Column>(document.pages.single().tree)
        assertEquals(PluginUiDocument.MAX_TREE_LEAVES, col.children.size)
        assertEquals("t1", assertIs<PluginUiNode.Text>(col.children.first()).text)
        assertEquals("t64", assertIs<PluginUiNode.Text>(col.children.last()).text)
    }
}
