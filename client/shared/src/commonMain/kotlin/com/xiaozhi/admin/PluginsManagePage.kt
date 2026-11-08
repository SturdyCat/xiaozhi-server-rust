package com.xiaozhi.admin

//! 插件管理页：纯清单 + 行级 actions。
//!
//! 页面只负责三件事：标题栏添加插件、每个 WASM 插件占一行、详情弹窗。
//! 版本/权限/依赖/registration UI 等明细不在列表展开，统一收进详情弹窗；
//! 数据源仍是 [PluginMetaState]，写通道仍是 [PluginsManageState]。
//!
//! 根容器必须是 [pageScroller]：裸 Column 在 Kuikly 布局里会让首行吃掉父容器
//! 全部剩余高度（行内徽标被拉伸到整屏高）、后续行被压成 0 高且无法滚动。

import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import com.gearui.components.button.Button
import com.gearui.components.button.ButtonSize
import com.gearui.components.button.ButtonTheme
import com.gearui.components.button.ButtonType
import com.gearui.components.dialog.ConfirmDialog
import com.tencent.kuikly.compose.foundation.layout.Column
import com.tencent.kuikly.compose.ui.Alignment
import com.tencent.kuikly.compose.foundation.layout.Row
import com.tencent.kuikly.compose.foundation.layout.fillMaxWidth
import com.tencent.kuikly.compose.ui.Modifier
import com.tencent.kuikly.compose.ui.unit.dp
import com.xiaozhi.admin.contracts.IconToken
import com.xiaozhi.admin.icons.GearIcon
import com.xiaozhi.admin.kit.HintLine
import com.xiaozhi.admin.kit.LabeledCard
import com.xiaozhi.admin.kit.WarnLine
import com.xiaozhi.admin.kit.bodyLine
import com.xiaozhi.admin.kit.dividerH
import com.xiaozhi.admin.kit.hGap
import com.xiaozhi.admin.kit.listRow
import com.xiaozhi.admin.kit.pageScroller
import com.xiaozhi.admin.kit.pluginLifecycleBadge
import com.xiaozhi.admin.kit.pluginStatusBadge
import com.xiaozhi.admin.kit.statusBadge
import com.xiaozhi.admin.kit.vGap

/**
 * 插件清单：列表页不承载业务详情，所有插件固定为一行并把操作收在右侧。
 * `meta` 与 `manage` 共用同一份清单快照，写操作成功后自动刷新。
 */
@Composable
fun PluginsManagePage(meta: PluginMetaState, manage: PluginsManageState, toast: ToastState) {
    val ctx = LocalAdminPagerOrThrow()
    var feedback by remember { mutableStateOf("") }
    var confirmUnload by remember { mutableStateOf<String?>(null) }
    var confirmLifecycle by remember { mutableStateOf<Pair<String, String>?>(null) }
    var detailId by remember { mutableStateOf<String?>(null) }
    val snap = meta.wasmSlots
    val realLoadError = meta.loadError.takeUnless(::isWasmPlatformUnavailable)

    pageScroller {
        // 标题行：页面标题 + 右侧唯一的「添加插件」入口。
        listRow(
            title = "插件",
            trailing = {
                EchoMuseActionButton(
                    icon = IconToken.Plus,
                    contentDescription = "添加插件",
                    onClick = {
                        feedback = ""
                        manage.uploadPicked(ctx) { ok, msg ->
                            feedback = msg
                            toast.show(msg, if (ok) "ok" else "error")
                        }
                    },
                    disabled = manage.uploading || manage.busy.isNotEmpty(),
                    loading = manage.uploading,
                )
            },
        )
        HintLine("上传 WASM 插件后，它会进入列表；详情、依赖与生命周期操作从右侧 actions 进入。")
        if (feedback.isNotEmpty()) {
            if (feedback.contains("失败") || feedback.contains("错误")) {
                WarnLine(feedback)
            } else {
                bodyLine(feedback)
            }
        }
        if (!realLoadError.isNullOrEmpty()) {
            WarnLine(realLoadError + "（确认服务端地址可达后点「刷新清单」）", error = true)
        }
        dividerH(trailingGap = 0.dp)

        val plugins = snap?.plugins
        when {
            meta.loading && plugins == null -> HintLine("正在获取插件清单…")
            plugins.isNullOrEmpty() -> HintLine("暂无插件。点击右上角 +，选择 .wasm 文件上传。")
            else -> {
                for ((index, plugin) in plugins.withIndex()) {
                    PluginListRow(
                        plugin = plugin,
                        manage = manage,
                        onDetails = { detailId = plugin.id },
                        onUnload = { confirmUnload = plugin.id },
                        onLifecycleConfirm = { op -> confirmLifecycle = op to plugin.id },
                        onAction = { actionId ->
                            manage.runAction(ctx, plugin.id, actionId) { ok, msg ->
                                toast.show(msg, if (ok) "ok" else "error")
                            }
                        },
                        onLifecycleDone = { op ->
                            manageLifecycle(ctx, manage, plugin.id, op, toast)
                        },
                    )
                    if (index != plugins.lastIndex) dividerH()
                }
            }
        }

        // 旧服务端没有 lifecycle 投影时保留旧卸载通道；新服务端的 remove/deactivate
        // 走下面的生命周期确认弹窗。
        val unloadTarget = confirmUnload?.let { id -> snap?.plugin(id) }
        ConfirmDialog(
            visible = unloadTarget != null,
            title = "删除插件？",
            message = "${unloadTarget?.title ?: ""}（${unloadTarget?.id ?: ""}）将被卸载并删除产物文件。此操作不可撤销。",
            confirmText = "删除",
            cancelText = "取消",
            destructive = true,
            dismissOnOutside = false,
            onConfirm = {
                val id = confirmUnload
                confirmUnload = null
                if (id != null) {
                    manage.unload(ctx, id) { ok, msg -> toast.show(msg, if (ok) "ok" else "error") }
                }
            },
            onCancel = { confirmUnload = null },
        )

        val lifecycleTarget = confirmLifecycle?.let { (op, id) -> op to snap?.plugin(id) }
        val lifecyclePlugin = lifecycleTarget?.second
        val isRemove = lifecycleTarget?.first == "remove"
        ConfirmDialog(
            visible = lifecyclePlugin != null,
            title = if (isRemove) "删除插件？" else "禁用插件？",
            message = buildString {
                append(lifecyclePlugin?.title ?: "")
                append("（")
                append(lifecyclePlugin?.id ?: "")
                append("）")
                if (isRemove) {
                    append("将从服务端移除并删除产物文件。仅禁用（非运行）插件可删除；此操作不可撤销。")
                } else {
                    append("将停止运行；注册声明与配置保留，之后可重新激活。")
                    append("若其他插件依赖它，需先停用依赖者。")
                }
            },
            confirmText = if (isRemove) "删除" else "禁用",
            cancelText = "取消",
            destructive = isRemove,
            dismissOnOutside = false,
            onConfirm = {
                val target = confirmLifecycle
                confirmLifecycle = null
                val (op, id) = target ?: return@ConfirmDialog
                manageLifecycle(ctx, manage, id, op, toast)
            },
            onCancel = { confirmLifecycle = null },
        )

        val detailTarget = detailId?.let { snap?.plugin(it) }
        if (detailTarget != null) {
            PluginManageDetailDialog(
                plugin = detailTarget,
                uiPages = meta.pluginUiDocs[detailTarget.id]?.pages
                    ?.map { "${it.title}（${it.kind}）" }
                    .orEmpty(),
                onDismiss = { detailId = null },
            )
        }
    }
}

@Composable
private fun PluginListRow(
    plugin: com.xiaozhi.admin.contracts.PluginSlotPlugin,
    manage: PluginsManageState,
    onDetails: () -> Unit,
    onUnload: () -> Unit,
    onLifecycleConfirm: (String) -> Unit,
    onAction: (String) -> Unit,
    onLifecycleDone: (String) -> Unit,
) {
    Column(modifier = Modifier.fillMaxWidth()) {
        // 第一行只放名称、状态标签和 actions；不在列表展开依赖/权限等详情。
        listRow(
            title = plugin.title.ifEmpty { plugin.id },
            trailing = {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    if (plugin.official) {
                        statusBadge(state = "idle", text = "系统插件")
                        hGap()
                    } else {
                        statusBadge(state = "busy", text = "用户插件")
                        hGap()
                    }
                    if (plugin.lifecycle == null) {
                        pluginStatusBadge(phase = plugin.phase, enabled = true)
                    } else {
                        pluginLifecycleBadge(lifecycle = plugin.lifecycle, phase = plugin.phase)
                    }
                    hGap()
                    EchoMuseActionButton(
                        icon = IconToken.Info,
                        contentDescription = "查看插件详情",
                        onClick = onDetails,
                        disabled = manage.busy.isNotEmpty(),
                    )
                    when (plugin.lifecycle) {
                        "staged" -> {
                            EchoMuseActionButton(
                                icon = IconToken.Check,
                                contentDescription = "注册插件",
                                onClick = { onLifecycleDone("register") },
                                disabled = manage.busy.isNotEmpty(),
                                loading = manage.busy == "register:${plugin.id}",
                            )
                            EchoMuseActionButton(
                                icon = IconToken.Delete,
                                contentDescription = "删除插件",
                                onClick = { onLifecycleConfirm("remove") },
                                destructive = true,
                                disabled = manage.busy.isNotEmpty(),
                            )
                        }
                        "registered" -> {
                            EchoMuseActionButton(
                                icon = IconToken.Check,
                                contentDescription = "激活插件",
                                onClick = { onLifecycleDone("activate") },
                                disabled = !plugin.canActivate || manage.busy.isNotEmpty(),
                                loading = manage.busy == "activate:${plugin.id}",
                            )
                            EchoMuseActionButton(
                                icon = IconToken.Delete,
                                contentDescription = "删除插件",
                                onClick = { onLifecycleConfirm("remove") },
                                destructive = true,
                                disabled = manage.busy.isNotEmpty(),
                            )
                        }
                        "active" -> EchoMuseActionButton(
                            icon = IconToken.Close,
                            contentDescription = "禁用插件",
                            onClick = { onLifecycleConfirm("deactivate") },
                            disabled = !plugin.canDeactivate || manage.busy.isNotEmpty(),
                            loading = manage.busy == "deactivate:${plugin.id}",
                        )
                        null -> {
                            for (action in plugin.actions) {
                                EchoMuseActionButton(
                                    icon = IconToken.Play,
                                    contentDescription = action.label,
                                    onClick = { onAction(action.id) },
                                    disabled = manage.busy.isNotEmpty(),
                                    loading = manage.busy == "${plugin.id}:${action.id}",
                                )
                            }
                            EchoMuseActionButton(
                                icon = IconToken.Delete,
                                contentDescription = "删除插件",
                                onClick = onUnload,
                                destructive = true,
                                disabled = manage.busy.isNotEmpty(),
                                loading = manage.busy == "unload:${plugin.id}",
                            )
                        }
                    }
                }
            },
        )
        // 第二行是次要信息：id/版本 + 一段可换行的简短描述。
        HintLine(
            buildString {
                append(plugin.id)
                if (plugin.version.isNotBlank()) append(" · v${plugin.version}")
                if (plugin.description.isNotBlank()) append("\n${plugin.description}")
            },
        )
        if (plugin.lifecycle == "registered" && plugin.missingRequirements.isNotEmpty()) {
            HintLine("依赖未满足，无法激活：" + plugin.missingRequirements.joinToString("、") { it.id })
        }
    }
}

private fun manageLifecycle(
    ctx: com.tencent.kuikly.core.pager.Pager,
    manage: PluginsManageState,
    pluginId: String,
    op: String,
    toast: ToastState,
) {
    val callback = { ok: Boolean, msg: String -> toast.show(msg, if (ok) "ok" else "error") }
    when (op) {
        "register" -> manage.register(ctx, pluginId, callback)
        "activate" -> manage.activate(ctx, pluginId, callback)
        "deactivate" -> manage.deactivate(ctx, pluginId, callback)
        "remove" -> manage.remove(ctx, pluginId, callback)
    }
}

private fun isWasmPlatformUnavailable(message: String): Boolean {
    val lower = message.lowercase()
    val mentionsWasm = lower.contains("wasm-plugins") || lower.contains("wasm 插件平台")
    if (!mentionsWasm) return false
    return lower.contains("未启用") || lower.contains("feature") || lower.contains("构建") ||
        lower.contains("不可用")
}

/**
 * 动态插件声明式运行页：仅消费 Active 插件的 registration.ui；插件未声明 UI 时回退到
 * manifest 字段与动作，业务 UI 不回编进静态管理台。
 */
@Composable
fun PluginRuntimePage(
    plugin: com.xiaozhi.admin.contracts.PluginSlotPlugin,
    uiDocs: Map<String, com.xiaozhi.admin.contracts.PluginUiDocument>,
    manage: PluginsManageState,
    toast: ToastState,
) {
    val ctx = LocalAdminPagerOrThrow()
    val doc = uiDocs[plugin.id]
    pageScroller {
        LabeledCard(title = plugin.title) {
            HintLine(plugin.description.ifEmpty { "该页面由插件清单声明，底座不内置业务页面。" })
            HintLine("${plugin.id} · v${plugin.version} · ${plugin.phase}")
            if (plugin.implements.isNotEmpty()) HintLine("扩展点：" + plugin.implements.joinToString("、"))
            if (plugin.permissions.isNotEmpty()) HintLine("权限：" + plugin.permissions.joinToString("、"))
        }
        if (doc != null) {
            PluginUiDocumentContent(doc, plugin, manage, toast)
            PluginUiDocFooter()
        } else {
            if (plugin.configFields.isNotEmpty()) {
                LabeledCard(title = "插件配置字段") {
                    for (field in plugin.configFields) {
                        HintLine(
                            "${field.label}（${field.key} · ${field.kind}）" +
                                field.help.takeIf { it.isNotBlank() }?.let { "：$it" }.orEmpty(),
                        )
                    }
                    HintLine("字段由插件 manifest 声明；底座不会为未声明字段生成业务配置。")
                }
            }
            LabeledCard(title = "插件动作") {
                if (plugin.actions.isEmpty()) {
                    HintLine("该插件未声明管理动作。")
                } else {
                    for (action in plugin.actions) {
                        Button(
                            text = if (manage.busy == "${plugin.id}:${action.id}") "执行中…" else action.label,
                            onClick = {
                                manage.runAction(ctx, plugin.id, action.id) { ok, msg ->
                                    toast.show(msg, if (ok) "ok" else "error")
                                }
                            },
                            type = ButtonType.OUTLINE,
                            size = ButtonSize.SMALL,
                            disabled = manage.busy.isNotEmpty(),
                        )
                    }
                }
            }
        }
    }
}
