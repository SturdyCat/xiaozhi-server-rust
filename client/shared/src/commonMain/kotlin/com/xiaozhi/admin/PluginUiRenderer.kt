package com.xiaozhi.admin

//! 动态插件 UI 壳的渲染层（A2UI v0.1 嵌套组件树 → GearUI）：把 [PluginUiDocument] 的
//! 页/节点树分发到既有零件（LabeledCard / HintLine / labeledField / Button / actionRow），
//! 不手搓官方已有组件。
//!
//! 边界（与解析层契约一致，见 contracts/PluginUi.kt）：
//! - 只渲染解析后已通过 fail-closed 过滤的页/树；渲染层不做第二道过滤、也不补节点
//!   （tree 没引用的字段/动作不会在这里出现——声明清单是唯一权威）；
//! - field/form 是**字段元数据展示**（与声明式回退页同口径）：客户端没有插件配置值的
//!   读取与写入通道，不渲染可编辑输入框，不伪造「已保存」（配置写入是后续独立工作）；
//! - actions/panel 复用 [PluginsManageState.runAction]（既有动作通道），busy 口径与
//!   插件管理页一致；danger 动作 ConfirmDialog 二次确认（GearUI 无 DANGER 按钮型，
//!   与卸载同口径），确认后与普通动作走同一条通道与 payload；
//! - input 是页内临时测试输入：值存页面级 remember 态——不进 ConfigFormState、不标脏、
//!   不落配置；页面含 input 时动作 payload = 该页全部输入值对象（未填 = 空串），
//!   无输入页保持空对象（现状不变）；
//! - result 展示该页最近一次动作调用的 ok/err + 原文回包；页按文档顺序整页渲染，
//!   `kind` 仅作展示标注。

import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import com.gearui.components.button.Button
import com.gearui.components.button.ButtonSize
import com.gearui.components.button.ButtonType
import com.gearui.components.dialog.ConfirmDialog
import com.gearui.foundation.layout.Spacing
import com.gearui.foundation.primitives.Text
import com.gearui.theme.Theme
import com.tencent.kuikly.compose.foundation.layout.Column
import com.tencent.kuikly.compose.foundation.layout.fillMaxWidth
import com.tencent.kuikly.compose.foundation.layout.padding
import com.tencent.kuikly.compose.ui.Modifier
import com.tencent.kuikly.core.nvi.serialization.json.JSONObject
import com.xiaozhi.admin.contracts.PluginAction
import com.xiaozhi.admin.contracts.PluginConfigField
import com.xiaozhi.admin.contracts.PluginSlotPlugin
import com.xiaozhi.admin.contracts.PluginUiDocument
import com.xiaozhi.admin.contracts.PluginUiNode
import com.xiaozhi.admin.contracts.PluginUiPage
import com.xiaozhi.admin.kit.actionRow
import com.xiaozhi.admin.kit.labeledField

/**
 * 页面 `result` 节点展示的最近一次动作调用结果（UI 运行态，非持久契约）；
 * [rawBody] 是服务端回包原文（WS RPC 解析对象的 JSON 文本）。
 */
class PluginUiActionResult(
    val actionId: String,
    val ok: Boolean,
    val message: String,
    val rawBody: String,
)

/** 渲染一份动态 UI 文档（调用方保证 doc 非空；页/树已在解析期 fail-closed）。 */
@Composable
internal fun PluginUiDocumentContent(
    doc: PluginUiDocument,
    plugin: PluginSlotPlugin,
    manage: PluginsManageState,
    toast: ToastState,
) {
    for (page in doc.pages) {
        PluginUiPageView(page, plugin, manage, toast)
    }
}

@Composable
private fun PluginUiPageView(
    page: PluginUiPage,
    plugin: PluginSlotPlugin,
    manage: PluginsManageState,
    toast: ToastState,
) {
    val ctx = LocalAdminPagerOrThrow()
    // 页面级运行态（纯展示，remember 即可；不进状态层、不标脏、不落配置）
    var inputValues by remember(page.id) { mutableStateOf(emptyMap<String, String>()) }
    var lastResult by remember(page.id) { mutableStateOf<PluginUiActionResult?>(null) }
    var confirmDanger by remember(page.id) { mutableStateOf<PluginAction?>(null) }

    /** 动作 payload：页面含 input → 全部输入值对象（未填 = 空串）；否则空对象（现状不变）。 */
    fun payloadOf(): JSONObject {
        val payload = JSONObject()
        for (id in page.inputIds) {
            payload.put(id, inputValues[id] ?: "")
        }
        return payload
    }

    fun execute(action: PluginAction) {
        manage.runAction(ctx, plugin.id, action.id, payloadOf()) { ok, msg, raw ->
            lastResult = PluginUiActionResult(action.id, ok, msg, raw)
            toast.show(msg, if (ok) "ok" else "error")
        }
    }

    LabeledCard(title = page.title) {
        if (page.kind.isNotEmpty()) {
            HintLine("页面类型：${page.kind}")
        }
        PluginUiNodeView(
            node = page.tree,
            plugin = plugin,
            manage = manage,
            inputValues = inputValues,
            onInput = { id, value -> inputValues = inputValues + (id to value) },
            lastResult = lastResult,
            onAction = { action ->
                if (action.danger) confirmDanger = action else execute(action)
            },
        )
    }

    // 危险动作二次确认（元数据取注册声明；点遮罩不关闭，GearUI 无 DANGER 按钮型）
    val danger = confirmDanger
    ConfirmDialog(
        visible = danger != null,
        title = "执行危险动作？",
        message = buildString {
            append(plugin.title)
            append(" · ")
            append(danger?.label ?: "")
            if (!danger?.help.isNullOrBlank()) {
                append("：")
                append(danger?.help)
            }
            append("。该动作由插件声明为危险操作，请确认后执行。")
        },
        confirmText = "执行",
        cancelText = "取消",
        destructive = true,
        dismissOnOutside = false,
        onConfirm = {
            val action = confirmDanger
            confirmDanger = null
            if (action != null) execute(action)
        },
        onCancel = { confirmDanger = null },
    )
}

/** 节点分发：容器递归，叶子落到 GearUI 零件（tree 已 fail-closed，这里不再过滤）。 */
@Composable
private fun PluginUiNodeView(
    node: PluginUiNode,
    plugin: PluginSlotPlugin,
    manage: PluginsManageState,
    inputValues: Map<String, String>,
    onInput: (String, String) -> Unit,
    lastResult: PluginUiActionResult?,
    onAction: (PluginAction) -> Unit,
) {
    when (node) {
        is PluginUiNode.Card -> LabeledCard(title = node.title) {
            PluginUiNodeView(node.child, plugin, manage, inputValues, onInput, lastResult, onAction)
        }
        is PluginUiNode.Column -> Column(modifier = Modifier.fillMaxWidth()) {
            for (child in node.children) {
                PluginUiNodeView(child, plugin, manage, inputValues, onInput, lastResult, onAction)
            }
        }
        is PluginUiNode.Text -> Text(
            text = node.text,
            modifier = Modifier.fillMaxWidth().padding(top = Spacing.sm),
            style = Theme.typography.bodyMedium,
            color = Theme.colors.foreground,
        )
        is PluginUiNode.Hint -> HintLine(node.text)
        is PluginUiNode.Field -> PluginUiFieldRow(node.field)
        is PluginUiNode.Form -> {
            for (field in node.fields) {
                PluginUiFieldRow(field)
            }
            HintLine("字段由插件 registration 声明；当前版本展示声明元数据，配置值读写随后续插件面开放。")
        }
        is PluginUiNode.Actions -> PluginUiActions(node.actions, plugin, manage, onAction)
        // 旧扁平形态与 actions 同语义：按钮组（元数据已取注册声明）
        is PluginUiNode.Panel -> PluginUiActions(node.actions, plugin, manage, onAction)
        // 页内临时输入：值写页面级 remember 态，不进任何配置草稿
        is PluginUiNode.Input -> labeledField(
            label = node.label,
            value = inputValues[node.id] ?: "",
            onValueChange = { value -> onInput(node.id, value) },
            placeholder = node.placeholder,
        )
        PluginUiNode.Result -> PluginUiResultView(lastResult)
    }
}

/** 声明字段的元数据展示行（不含值：客户端无插件配置读取通道，不伪造）。 */
@Composable
private fun PluginUiFieldRow(field: PluginConfigField) {
    val secretNote = if (field.secret) " · 密钥字段（值不回显）" else ""
    val help = field.help.takeIf { it.isNotBlank() }?.let { "：$it" } ?: ""
    HintLine("${field.label}（${field.kind}$secretNote）$help")
}

/** 动作按钮组：元数据来自注册声明；danger 动作由页面侧 ConfirmDialog 二次确认后执行。 */
@Composable
private fun PluginUiActions(
    actions: List<PluginAction>,
    plugin: PluginSlotPlugin,
    manage: PluginsManageState,
    onAction: (PluginAction) -> Unit,
) {
    actionRow {
        for (action in actions) {
            Button(
                text = if (manage.busy == "${plugin.id}:${action.id}") "执行中…" else action.label,
                onClick = { onAction(action) },
                type = ButtonType.OUTLINE,
                size = ButtonSize.SMALL,
                disabled = manage.busy.isNotEmpty(),
            )
        }
    }
}

/** `result` 节点：该页最近一次动作调用的 ok/err + 原文回包（尚未执行时说明占位语义）。 */
@Composable
private fun PluginUiResultView(result: PluginUiActionResult?) {
    if (result == null) {
        HintLine("该页尚未执行动作；动作完成后此处显示最近一次调用结果与回包原文。")
        return
    }
    Text(
        text = (if (result.ok) "成功" else "失败") + " · " + result.actionId + "：" + result.message,
        modifier = Modifier.fillMaxWidth().padding(top = Spacing.sm),
        style = Theme.typography.bodyMedium,
        color = if (result.ok) Theme.colors.foreground else Theme.colors.destructive,
    )
    if (result.rawBody.isNotEmpty()) {
        Text(
            text = "回包原文：${result.rawBody}",
            modifier = Modifier.fillMaxWidth().padding(top = Spacing.xs),
            style = Theme.typography.caption,
            color = Theme.colors.mutedForeground,
        )
    }
}

/** 文档页的收尾说明（文档模式专属；回退页不渲染）。 */
@Composable
internal fun PluginUiDocFooter() {
    Column {
        HintLine(
            "页面布局由插件 registration.ui（A2UI ${PluginUiDocument.SUPPORTED_VERSION} 嵌套组件树）声明：" +
                "容器只做布局，字段/动作只来自注册清单，input 仅页内临时输入，底座不生成业务表单。",
        )
    }
}
