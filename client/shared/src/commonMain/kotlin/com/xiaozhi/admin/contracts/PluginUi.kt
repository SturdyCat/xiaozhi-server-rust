package com.xiaozhi.admin.contracts

import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject

/**
 * 插件声明的**动态 UI 文档**（A2UI 嵌套组件树，v0.1）纯解析模型。
 *
 * ## 数据源与契约
 *
 * 字段来自 `GET /api/plugins` 的 `wasm.plugins[].registration.ui`：宿主是**透传管道**
 * （装载期只校验「单个 JSON 对象」与 ≤256KiB，不解释内容），仅 **Active** 插件投影
 * 解析后的对象，其余 phase / 未声明投影 `null`——未注册插件不返回业务 UI。文档形状
 * 与 guest SDK / hello 夹具同源：`{a2ui:"0.1", pages:[{id,title,kind,tree,bindings}]}`；
 * `tree` 是**嵌套节点树**（底座只提供业务无关的基础组件库，布局与选件全由插件声明；
 * 0.1 语法是旧扁平两页形态的超集扩展，form/panel 原样保留兼容）。
 *
 * ## 组件（全部业务无关）
 *
 * - 容器（只做布局）：`card`（带标题、单 child）、`column`（children 数组）；
 * - 叶子：`text` / `hint`（正文/次要说明，各 ≤[MAX_TEXT_CHARS] 字符，超出丢弃）；
 *   `field`（单个配置字段引用）、`form`（旧扁平字段组 = column of field）；
 *   `actions`（动作 id 数组）、`panel`（旧扁平动作组）；
 *   `input`（页内临时测试输入，不落配置）、`result`（最近一次动作结果占位）。
 *
 * ## fail-closed 规则（声明清单是唯一权威）
 *
 * - `field`/`form` 的字段引用必须命中 `registration.config_fields[].key`；`actions`/
 *   `panel` 的动作 id 必须命中 `registration.actions[].id`——未命中**逐条丢弃**；
 *   label/kind/help/danger 等元数据一律取注册声明，文档内的同名键只是展示副本、
 *   不能改写文案或把危险动作洗白；
 * - `input.id` 必须匹配 `[a-z0-9_-]{1,32}` 且**页内唯一**，label 必填非空——违规丢弃
 *   该节点；
 * - 未知 `type` 丢弃该节点与子树（单个未知组件不作废整份文档）；card 无有效 child /
 *   column 无有效 children 丢弃该容器；
 * - 容器嵌套深度 ≤ [MAX_CONTAINER_DEPTH]（第 5 层容器整体丢弃）；
 * - `a2ui` 版本必须显式等于 [SUPPORTED_VERSION]：缺版本 / 更高版本 → 整份文档作废
 *   （fail-closed 回退，不猜形状）；
 * - 页数 ≤ [MAX_PAGES]、每页叶子 ≤ [MAX_TREE_LEAVES]（超界按文档顺序截断）；
 *   页内无任何有效叶子丢页；全部页被丢弃 → null（声明式回退页）。
 *
 * `bindings`（`{"config":[key],"actions":[id]}`）是插件对引用的自描述清单：渲染器以
 * tree 为布局源、不做配置写入，bindings 仅被容忍不解释——插件不得用它表达 tree 之外
 * 的引用（解析期只认 tree 内命中声明的引用）。
 *
 * 本文件零 Kuikly 依赖（kotlinx JSON 纯解析），规则可在 adminCoreTests 纯 JVM 回归。
 */
class PluginUiDocument private constructor(
    /** 文档版本；当前仅支持 "0.1"。 */
    val version: String,
    /** 页面（按文档顺序；已应用 fail-closed 过滤与截断）。 */
    val pages: List<PluginUiPage>,
) {
    companion object {
        const val SUPPORTED_VERSION = "0.1"
        const val MAX_PAGES = 8
        const val MAX_TREE_LEAVES = 64

        /** 容器（card/column）嵌套深度上限：第 5 层容器整体丢弃。 */
        const val MAX_CONTAINER_DEPTH = 4

        /** text/hint 的字符上限：超出丢弃该节点（不截断——插件文档不该被静默改写）。 */
        const val MAX_TEXT_CHARS = 500

        /** `input.id` 形状：小写字母/数字/下划线/连字符，1..32 字符。 */
        private val INPUT_ID = Regex("""[a-z0-9_-]{1,32}""")

        /**
         * 解析一份 UI 文档对象（`registration.ui` 投影）。[fields] / [actions] 是同一
         * 插件的注册声明（`registration.config_fields` / `registration.actions`），作为
         * fail-closed 白名单。任何失败（null / 零有效页）都让调用方走声明式回退页。
         */
        fun parse(
            doc: JsonObject?,
            fields: List<PluginConfigField>,
            actions: List<PluginAction>,
        ): PluginUiDocument? {
            doc ?: return null
            val version = doc.string("a2ui") ?: return null
            if (version != SUPPORTED_VERSION) return null
            val pagesEl = doc["pages"] as? JsonArray ?: return null
            val out = mutableListOf<PluginUiPage>()
            val seenIds = mutableSetOf<String>()
            for (el in pagesEl) {
                if (out.size >= MAX_PAGES) break
                val page = parsePage(el as? JsonObject ?: continue, fields, actions) ?: continue
                if (!seenIds.add(page.id)) continue
                out += page
            }
            if (out.isEmpty()) return null
            return PluginUiDocument(version, out)
        }

        private fun parsePage(
            obj: JsonObject,
            fields: List<PluginConfigField>,
            actions: List<PluginAction>,
        ): PluginUiPage? {
            val id = obj.string("id")?.takeIf { it.isNotBlank() } ?: return null
            val tree = obj["tree"] as? JsonObject ?: return null
            val ctx = PageCtx(fields, actions)
            val node = parseNode(tree, 0, ctx) ?: return null
            return PluginUiPage(
                id = id,
                title = obj.string("title")?.takeIf { it.isNotBlank() } ?: id,
                kind = obj.string("kind").orEmpty(),
                tree = node,
                inputIds = ctx.inputIds.toSet(),
            )
        }

        /**
         * 递归解析一个节点。[depth] 是该节点上方的容器链长度（根调用为 0）：容器入链前
         * 校验 `depth + 1 ≤ MAX_CONTAINER_DEPTH`，叶子不占深度；节点解析失败返回 null
         * 由上层丢弃（card 失去 child 即整体丢弃、column 过滤无效孩子）。
         */
        private fun parseNode(obj: JsonObject, depth: Int, ctx: PageCtx): PluginUiNode? {
            return when (obj.string("type")) {
                "card" -> {
                    if (depth + 1 > MAX_CONTAINER_DEPTH) return null
                    val title = obj.string("title")?.takeIf { it.isNotBlank() } ?: return null
                    val child = (obj["child"] as? JsonObject)
                        ?.let { parseNode(it, depth + 1, ctx) } ?: return null
                    PluginUiNode.Card(title, child)
                }
                "column" -> {
                    if (depth + 1 > MAX_CONTAINER_DEPTH) return null
                    val rows = obj["children"] as? JsonArray ?: return null
                    val children = rows.mapNotNull { el ->
                        (el as? JsonObject)?.let { parseNode(it, depth + 1, ctx) }
                    }
                    if (children.isEmpty()) null else PluginUiNode.Column(children)
                }
                "text" -> obj.string("text")
                    ?.takeIf { it.length <= MAX_TEXT_CHARS }
                    ?.takeIf { ctx.takeLeaf() }
                    ?.let { PluginUiNode.Text(it) }
                "hint" -> obj.string("text")
                    ?.takeIf { it.length <= MAX_TEXT_CHARS }
                    ?.takeIf { ctx.takeLeaf() }
                    ?.let { PluginUiNode.Hint(it) }
                "field" -> {
                    val key = obj.string("key")?.takeIf { it.isNotBlank() } ?: return null
                    if (!ctx.takeLeaf()) return null
                    // fail-closed：只渲染注册声明过的配置字段；元数据取声明，文档不能改写
                    ctx.fields.firstOrNull { it.key == key }?.let { PluginUiNode.Field(it) }
                }
                "form" -> {
                    val rows = obj["fields"] as? JsonArray ?: return null
                    val declared = rows.mapNotNull { el ->
                        val row = el as? JsonObject ?: return@mapNotNull null
                        val key = row.string("key")?.takeIf { it.isNotBlank() } ?: return@mapNotNull null
                        if (!ctx.takeLeaf()) return@mapNotNull null
                        ctx.fields.firstOrNull { it.key == key }
                    }
                    if (declared.isEmpty()) null else PluginUiNode.Form(declared)
                }
                "actions" -> {
                    val ids = obj["actions"] as? JsonArray ?: return null
                    val declared = ids.mapNotNull { el ->
                        val id = (el as? JsonPrimitive)?.takeIf { it.isString }?.content
                            ?.takeIf { it.isNotBlank() } ?: return@mapNotNull null
                        if (!ctx.takeLeaf()) return@mapNotNull null
                        // fail-closed：引用必须命中注册声明；label/danger/help 一律取声明
                        ctx.actions.firstOrNull { it.id == id }
                    }
                    if (declared.isEmpty()) null else PluginUiNode.Actions(declared)
                }
                "panel" -> {
                    val rows = obj["actions"] as? JsonArray ?: return null
                    val declared = rows.mapNotNull { el ->
                        val row = el as? JsonObject ?: return@mapNotNull null
                        val id = row.string("action")?.takeIf { it.isNotBlank() } ?: return@mapNotNull null
                        if (!ctx.takeLeaf()) return@mapNotNull null
                        ctx.actions.firstOrNull { it.id == id }
                    }
                    if (declared.isEmpty()) null else PluginUiNode.Panel(declared)
                }
                "input" -> {
                    val inputId = obj.string("id") ?: return null
                    if (!INPUT_ID.matches(inputId)) return null
                    val label = obj.string("label")?.takeIf { it.isNotBlank() } ?: return null
                    if (!ctx.takeLeaf()) return null
                    // 页内唯一：同 id 的第二个 input 丢弃（首个保留）
                    if (!ctx.inputIds.add(inputId)) return null
                    PluginUiNode.Input(inputId, label, obj.string("placeholder").orEmpty())
                }
                "result" -> {
                    if (!ctx.takeLeaf()) return null
                    PluginUiNode.Result
                }
                // 未知类型：丢弃该节点与子树，不让单个未知组件作废整份文档
                else -> null
            }
        }

        private fun JsonObject.string(key: String): String? =
            (this[key] as? JsonPrimitive)?.takeIf { it.isString }?.content

        /** 单页解析上下文：叶子预算 + input id 页内唯一集。 */
        private class PageCtx(
            val fields: List<PluginConfigField>,
            val actions: List<PluginAction>,
        ) {
            var leaves = 0
            val inputIds = mutableSetOf<String>()

            fun takeLeaf(): Boolean {
                if (leaves >= MAX_TREE_LEAVES) return false
                leaves++
                return true
            }
        }
    }
}

/**
 * 文档的一页。`kind` 是插件自述（hello 夹具为 `config` / `test`）：渲染器只做展示
 * 语义，不按 kind 分支出别的能力。[inputIds] 是解析期收集的页内合法输入 id（动作
 * payload 组装用：页面含 input 时 payload = 全部输入值对象）。
 */
class PluginUiPage(
    val id: String,
    val title: String,
    val kind: String,
    val tree: PluginUiNode,
    val inputIds: Set<String> = emptySet(),
)

/**
 * 页面组件树节点（嵌套；fail-closed 解析后的引用集，渲染层按类型分发到 GearUI 零件）。
 *
 * 容器只做布局（card/column）；叶子全部是业务无关基础件——text/hint 纯文案、field/form
 * 引用注册配置字段、actions/panel 引用注册动作（元数据已解析为声明对象，节点上不携带
 * 文案）、input 页内临时输入、result 动作结果占位。
 */
sealed class PluginUiNode {
    /** 带标题卡片：单 [child]；无有效 child 时解析期已丢弃该容器。 */
    class Card(val title: String, val child: PluginUiNode) : PluginUiNode()

    /** 纵向布局：[children] 全部有效（空列 / 全无效列解析期已丢弃）。 */
    class Column(val children: List<PluginUiNode>) : PluginUiNode()

    /** 正文文案（≤500 字符）。 */
    class Text(val text: String) : PluginUiNode()

    /** 次要说明文案（≤500 字符）。 */
    class Hint(val text: String) : PluginUiNode()

    /** 单个配置字段引用（元数据 = 注册声明；客户端只展示，不做配置写入）。 */
    class Field(val field: PluginConfigField) : PluginUiNode()

    /** 旧扁平形态（v0.1 起保留兼容）= column of [Field]。 */
    class Form(val fields: List<PluginConfigField>) : PluginUiNode()

    /** 动作按钮组（id 字符串数组，元数据 = 注册声明；danger 由声明驱动二次确认）。 */
    class Actions(val actions: List<PluginAction>) : PluginUiNode()

    /** 旧扁平形态（v0.1 起保留兼容）= [Actions]（`{"action":id}` 行）。 */
    class Panel(val actions: List<PluginAction>) : PluginUiNode()

    /** 页内临时测试输入（不落配置、不标脏；id 页内唯一、`[a-z0-9_-]{1,32}`）。 */
    class Input(val id: String, val label: String, val placeholder: String) : PluginUiNode()

    /** 最近一次动作调用结果占位（调用结果本身是页面运行态，非文档内容）。 */
    object Result : PluginUiNode()
}

/**
 * `wasm` 键内全部插件 UI 文档的一次性解析（与 [PluginSlotSnapshot] 同一段文本）。
 *
 * 为什么独立解析而不是给 [PluginSlotPlugin] 加字段：本轮 `PluginSlots.kt` 由门控任务独占
 * 编辑，这里并行解析同一段 `wasm` 文本、并绑定**同一份快照**做 fail-closed 白名单——
 * 复用同一次拉取、不发第二个请求，也不产生第二份清单语义。
 *
 * 任何整体失败（缺 `wasm` 键 / 损坏 JSON / 快照为 null）→ 空表：所有插件走声明式回退页。
 */
object PluginUiSlots {

    /**
     * @param wasmJsonText `GET /api/plugins` 回包的 `wasm` 键原文（与
     *   `PluginSlotSnapshot.parse` 同输入）
     * @param snapshot 同文本解析出的槽位快照（提供每插件的声明字段/动作白名单；
     *   null = 静态基线态，动态 UI 一并不可用）
     */
    fun parse(
        wasmJsonText: String?,
        snapshot: PluginSlotSnapshot?,
    ): Map<String, PluginUiDocument> {
        if (snapshot == null) return emptyMap()
        val text = wasmJsonText?.takeIf { it.isNotBlank() } ?: return emptyMap()
        val wasm = runCatching { Json.parseToJsonElement(text).jsonObject }.getOrNull()
            ?: return emptyMap()
        val rows = runCatching { wasm["plugins"]?.jsonArray }.getOrNull() ?: return emptyMap()
        val out = mutableMapOf<String, PluginUiDocument>()
        for (rowEl in rows) {
            val row = rowEl as? JsonObject ?: continue
            val id = row.string("id")?.takeIf { it.isNotBlank() } ?: continue
            val plugin = snapshot.plugin(id) ?: continue
            // 宿主投影：registration.ui = 解析后的文档对象（仅 Active 插件；其余 null）
            val docObj = (row["registration"] as? JsonObject)?.get("ui") as? JsonObject
                ?: continue
            val doc = PluginUiDocument.parse(docObj, plugin.configFields, plugin.actions) ?: continue
            out[id] = doc
        }
        return out
    }

    private fun JsonObject.string(key: String): String? =
        (this[key] as? JsonPrimitive)?.takeIf { it.isString }?.content
}
