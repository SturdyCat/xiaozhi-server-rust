package com.xiaozhi.admin

//! 「插件」管理页（P3 末件）：上传 / 清单 / 卸载 / 动作 / 扩展点解析。
//!
//! 数据源：[PluginMetaState]（`GET /api/plugins` 的 `wasm` 键快照，同一次拉取）；
//! 写通道：[PluginsManageState]（raw 上传桥 / unload / 统一动作）。组件复用 kit 与
//! OtaCard 同款零件（LabeledCard / HintLine / WarnLine / SectionSummaryRow /
//! pluginStatusBadge / ConfirmDialog），颜色走语义 token。插件管理入口和清单卡片
//! 永远保留；服务端没有插件时只显示空清单，不把缺少 `wasm` 投影误报成平台不可用。

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
import com.tencent.kuikly.compose.foundation.layout.fillMaxWidth
import com.tencent.kuikly.compose.ui.Modifier
import com.tencent.kuikly.compose.ui.unit.dp
import com.xiaozhi.admin.contracts.IconToken
import com.xiaozhi.admin.icons.GearIcon
import com.xiaozhi.admin.kit.HintLine
import com.xiaozhi.admin.kit.LabeledCard
import com.xiaozhi.admin.kit.SectionSummaryRow
import com.xiaozhi.admin.kit.WarnLine
import com.xiaozhi.admin.kit.actionRow
import com.xiaozhi.admin.kit.listRow
import com.xiaozhi.admin.kit.pluginLifecycleBadge
import com.xiaozhi.admin.kit.pluginStatusBadge
import com.xiaozhi.admin.kit.statusBadge
import com.xiaozhi.admin.kit.vGap

/**
 * 「插件」页入口：上传入口 + 已装载清单 + 扩展点解析 + 卸载二次确认。
 * `meta` 同时是清单快照与刷新载体（manage 写成功后调 `meta.load` 刷新）。
 */
@Composable
fun PluginsManagePage(meta: PluginMetaState, manage: PluginsManageState, toast: ToastState) {
    val ctx = LocalAdminPagerOrThrow()
    // 上传结果就地回显（纯展示态，不进状态层）；每次上传前清空
    val feedback = remember { mutableStateOf("") }
    // 卸载二次确认：持有待确认的插件 id（null = 关闭）
    var confirmUnload by remember { mutableStateOf<String?>(null) }
    // 生命周期二次确认：op（register/activate/deactivate/remove）→ 插件 id（null = 关闭）
    var confirmLifecycle by remember { mutableStateOf<Pair<String, String>?>(null) }
    // 详情弹窗：持有插件 id（null = 关闭；内容从当前清单快照组装，零请求）
    var detailId by remember { mutableStateOf<String?>(null) }
    val snap = meta.wasmSlots

    Column(modifier = Modifier.fillMaxWidth()) {
        if (meta.loadError.isNotEmpty()) {
            LabeledCard(title = "插件平台") {
                WarnLine(meta.loadError + "（确认服务端地址可达后点「刷新清单」）", error = true)
            }
        }

        // 上传入口：id 缺省 = 服务端从组件 manifest 探测（"选文件即上传"）
        LabeledCard(title = "上传插件组件") {
            HintLine(
                "选择本地 .wasm 组件上传：服务端做隔离验证（manifest / 注册包 / 自检）后进入" +
                    "「待注册」状态（staged），注册并激活后才运行业务；失败不影响已装载插件。" +
                    "上传落盘为持久意图，重启自动重装。",
            )
            SectionSummaryRow(
                summary = feedback.value.ifEmpty {
                    if (manage.uploading) "上传中…" else "组件 id 由服务端从组件内读取，无需手工指定"
                },
                actionText = "选择 .wasm 并上传",
                busy = manage.uploading,
                busyText = "上传中…",
                onAction = {
                    feedback.value = ""
                    manage.uploadPicked(ctx) { ok, msg ->
                        feedback.value = msg
                        toast.show(msg, if (ok) "ok" else "error")
                    }
                },
            )
            if (feedback.value.isNotEmpty() && !manage.uploading && !feedback.value.contains("已上传")) {
                WarnLine(feedback.value)
            }
        }

        // 已装载清单（含 staged / registered：上传与注册后的插件同样在列，生命周期按钮随状态渲染）
        LabeledCard(title = "已装载插件") {
            val plugins = snap?.plugins
            if (plugins.isNullOrEmpty()) {
                // 加载中不得宣判「暂无插件」：快照晚于首帧到达，清单为空只在加载完成后成立。
                if (meta.loading) {
                    HintLine("正在获取插件清单…")
                } else {
                    HintLine("暂无插件：上传 .wasm 组件后进入「待注册」，注册并激活后才运行业务。")
                }
            } else {
                for (plugin in plugins) {
                    listRow(title = plugin.title) {
                        if (!plugin.official) {
                            statusBadge(state = "busy", text = "用户实现")
                        }
                        // lifecycle 缺键 = 旧服务端：维持原 phase 徽标；新服务端按生命周期打标
                        if (plugin.lifecycle == null) {
                            pluginStatusBadge(phase = plugin.phase, enabled = true)
                        } else {
                            pluginLifecycleBadge(lifecycle = plugin.lifecycle, phase = plugin.phase)
                        }
                    }
                    HintLine(
                        "${plugin.id} · v${plugin.version}" +
                            plugin.apiVersion.takeIf { it.isNotBlank() }?.let { " · 契约 v$it" }.orEmpty(),
                    )
                    if (plugin.description.isNotEmpty()) {
                        HintLine(plugin.description)
                    }
                    if (plugin.permissions.isNotEmpty()) {
                        HintLine("权限：" + plugin.permissions.joinToString("、"))
                    }
                    // 依赖门控就地提示（按钮侧同时禁用；文案解释缺什么/先停用谁）
                    if (plugin.lifecycle == "registered" && plugin.missingRequirements.isNotEmpty()) {
                        WarnLine(
                            "依赖未满足：" + plugin.missingRequirements.joinToString("、") { r ->
                                "${r.id} ≥ ${r.minVersion}（当前 ${r.currentState}）"
                            } + "；满足后才能激活。",
                        )
                    }
                    if (plugin.dependents.isNotEmpty()) {
                        HintLine("被依赖：" + plugin.dependents.joinToString("、") + "；先停用依赖者才能禁用本插件。")
                    }
                    // 生命周期按钮（按状态渲染；旧服务端 lifecycle 缺键 → 维持动作 + 旧卸载通道）
                    actionRow {
                        Button(
                            text = "详情",
                            onClick = { detailId = plugin.id },
                            theme = ButtonTheme.DEFAULT,
                            type = ButtonType.OUTLINE,
                            size = ButtonSize.SMALL,
                        )
                        when (plugin.lifecycle) {
                            "staged" -> {
                                // 注册幂等无危险：直接执行（不进确认弹窗）
                                lifecycleButton(manage, plugin.id, "register", "注册", busyLabel = "注册中…") {
                                    manage.register(ctx, plugin.id) { ok, msg ->
                                        toast.show(msg, if (ok) "ok" else "error")
                                    }
                                }
                                lifecycleButton(manage, plugin.id, "remove", "移除", busyLabel = "移除中…") {
                                    confirmLifecycle = "remove" to plugin.id
                                }
                            }
                            "registered" -> {
                                // requires 未满足 → 激活禁用（缺失原因见上方依赖提示）；激活无需确认
                                lifecycleButton(
                                    manage, plugin.id, "activate", "激活", busyLabel = "激活中…",
                                    enabled = plugin.canActivate,
                                ) {
                                    manage.activate(ctx, plugin.id) { ok, msg ->
                                        toast.show(msg, if (ok) "ok" else "error")
                                    }
                                }
                                lifecycleButton(manage, plugin.id, "remove", "移除", busyLabel = "移除中…") {
                                    confirmLifecycle = "remove" to plugin.id
                                }
                            }
                            "active" -> {
                                // dependents 非空 → 禁用禁用按钮 + 提示先停依赖者
                                lifecycleButton(
                                    manage, plugin.id, "deactivate", "禁用", busyLabel = "禁用中…",
                                    enabled = plugin.canDeactivate,
                                ) {
                                    confirmLifecycle = "deactivate" to plugin.id
                                }
                            }
                            null -> {
                                // 旧服务端兼容：动作按钮按注册清单渲染 + 旧卸载端点
                                for (action in plugin.actions) {
                                    Button(
                                        text = if (manage.busy == "${plugin.id}:${action.id}") "执行中…" else action.label,
                                        onClick = {
                                            manage.runAction(ctx, plugin.id, action.id) { ok, msg ->
                                                toast.show(msg, if (ok) "ok" else "error")
                                            }
                                        },
                                        theme = ButtonTheme.DEFAULT,
                                        type = ButtonType.OUTLINE,
                                        size = ButtonSize.SMALL,
                                        disabled = manage.busy.isNotEmpty(),
                                    )
                                }
                                Button(
                                    text = if (manage.busy == "unload:${plugin.id}") "卸载中…" else "卸载",
                                    onClick = { confirmUnload = plugin.id },
                                    theme = ButtonTheme.DEFAULT,
                                    type = ButtonType.OUTLINE, // GearUI 无 DANGER 型：危险意图由文案 + 弹层二次确认承载
                                    size = ButtonSize.SMALL,
                                    iconWidget = { color -> GearIcon(token = IconToken.Delete, size = 14.dp, tint = color) },
                                )
                            }
                        }
                    }
                    vGap()
                }
            }
        }

        // 扩展点解析（排他扩展点的当前实现与回退原因一目了然）
        LabeledCard(title = "扩展点解析") {
            val extensions = snap?.extensions
            if (extensions.isNullOrEmpty()) {
                HintLine("暂无已声明的扩展点。")
            } else {
                for ((point, state) in extensions) {
                    listRow(title = point) {
                        when (state.state) {
                            "resolved" -> statusBadge(state = "ok", text = state.implementation)
                            "fallback" -> statusBadge(state = "busy", text = "回退：${state.implementation}")
                            else -> statusBadge(state = "idle", text = "未解析")
                        }
                    }
                }
                HintLine("切换实现：保存「语音流程」编排或调扩展点首选端点；用户实现失败自动回退官方默认。")
            }
        }

        // 卸载二次确认（destructive；点遮罩不关闭）
        val unloadTarget = confirmUnload?.let { id -> snap?.plugin(id) }
        ConfirmDialog(
            visible = unloadTarget != null,
            title = "卸载插件？",
            message = buildString {
                append(unloadTarget?.title ?: "")
                append("（")
                append(unloadTarget?.id ?: "")
                append("）将被卸载并删除产物文件；kv 缓存一并回收。")
                append("此操作不可撤销；重新启用需要重新上传。")
            },
            confirmText = "卸载",
            cancelText = "取消",
            destructive = true,
            dismissOnOutside = false,
            onConfirm = {
                val id = confirmUnload
                confirmUnload = null
                if (id != null) {
                    manage.unload(ctx, id) { ok, msg ->
                        toast.show(msg, if (ok) "ok" else "error")
                    }
                }
            },
            onCancel = { confirmUnload = null },
        )

        // 生命周期二次确认（注册/激活幂等无危险直接执行；remove / deactivate 必确认）
        val lifecycleTarget = confirmLifecycle?.let { (op, id) -> op to snap?.plugin(id) }
        val lifecyclePlugin = lifecycleTarget?.second
        val isRemove = lifecycleTarget?.first == "remove"
        ConfirmDialog(
            visible = lifecyclePlugin != null,
            title = if (isRemove) "移除插件？" else "禁用插件？",
            message = buildString {
                append(lifecyclePlugin?.title ?: "")
                append("（")
                append(lifecyclePlugin?.id ?: "")
                append("）")
                if (isRemove) {
                    append("将从服务端移除并删除产物文件。仅禁用（非运行）插件可移除；")
                    append("此操作不可撤销，重新启用需要重新上传。")
                } else {
                    append("将停止运行；注册声明与配置保留，之后可重新激活。")
                    append("若其他插件依赖它，需先停用依赖者。")
                }
            },
            confirmText = if (isRemove) "移除" else "禁用",
            cancelText = "取消",
            destructive = isRemove,
            dismissOnOutside = false,
            onConfirm = {
                val target = confirmLifecycle
                confirmLifecycle = null
                val (op, id) = target ?: return@ConfirmDialog
                val callback = { ok: Boolean, msg: String -> toast.show(msg, if (ok) "ok" else "error") }
                when (op) {
                    "deactivate" -> manage.deactivate(ctx, id, callback)
                    "remove" -> manage.remove(ctx, id, callback)
                }
            },
            onCancel = { confirmLifecycle = null },
        )

        // 详情弹窗（快照静态渲染；uiPages 从已解析的动态文档提取，仅 Active 有内容）
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

/**
 * 生命周期按钮统一封装：busy 键按 `op:pluginId` 判定、全局 busy 防重入（激活等幂等动作
 * 同样互斥，避免清单刷新与写通道并发）；[enabled]=false 渲染禁用态（依赖/被依赖门控由
 * 调用方传入，原因文案在行内提示承载）。
 */
@Composable
private fun lifecycleButton(
    manage: PluginsManageState,
    pluginId: String,
    op: String,
    label: String,
    busyLabel: String,
    enabled: Boolean = true,
    onConfirm: () -> Unit,
) {
    Button(
        text = if (manage.busy == "$op:$pluginId") busyLabel else label,
        onClick = onConfirm,
        theme = ButtonTheme.DEFAULT,
        type = ButtonType.OUTLINE,
        size = ButtonSize.SMALL,
        disabled = !enabled || manage.busy.isNotEmpty(),
    )
}

/**
 * 动态插件的声明式运行页：**优先消费插件 `registration.ui` 动态文档**（通用 UI 壳，
 * 见 [PluginUiDocumentContent]；仅 Active 插件由宿主投影，未装载/降级一律缺席）；
 * 插件未声明 / 解析失败时回退到既有声明式页面——只渲染 manifest registration 提供的
 * 描述、字段元数据和动作。这里不复制任何业务页面；业务插件需要更丰富的界面时，
 * 应继续扩展 manifest/WASM action，而不是把业务组件重新编回 server 或静态管理台。
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
    Column(modifier = Modifier.fillMaxWidth()) {
        LabeledCard(title = plugin.title) {
            HintLine(plugin.description.ifEmpty { "该页面由插件清单声明，底座不内置业务页面。" })
            HintLine("${plugin.id} · v${plugin.version} · ${plugin.phase}")
            if (plugin.implements.isNotEmpty()) HintLine("扩展点：${plugin.implements.joinToString("、")}")
            if (plugin.permissions.isNotEmpty()) HintLine("权限：${plugin.permissions.joinToString("、")}")
        }
        if (doc != null) {
            // 动态文档模式：布局/引用全部由插件声明，节点已在解析期按注册声明 fail-closed
            PluginUiDocumentContent(doc, plugin, manage, toast)
            PluginUiDocFooter()
        } else {
            // 声明式回退：插件未声明 ui / 非 Active（宿主投影 null）/ 文档解析未通过
            if (plugin.configFields.isNotEmpty()) {
                LabeledCard(title = "插件配置字段") {
                    for (field in plugin.configFields) {
                        HintLine("${field.label}（${field.key} · ${field.kind}）${field.help.takeIf { it.isNotBlank() }?.let { "：$it" } ?: ""}")
                    }
                    HintLine("字段由插件 manifest 声明；底座不会为未声明字段生成业务配置。")
                }
            }
            LabeledCard(title = "插件动作") {
                if (plugin.actions.isEmpty()) {
                    HintLine("该插件未声明管理动作。")
                } else {
                    actionRow {
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
}

