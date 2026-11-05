package com.xiaozhi.admin

//! 「插件」管理页（P3 末件）：上传 / 清单 / 卸载 / 动作 / 扩展点解析。
//!
//! 数据源：[PluginMetaState]（`GET /api/plugins` 的 `wasm` 键快照，同一次拉取）；
//! 写通道：[PluginsManageState]（raw 上传桥 / unload / 统一动作）。组件复用 kit 与
//! OtaCard 同款零件（LabeledCard / HintLine / WarnLine / SectionSummaryRow /
//! pluginStatusBadge / ConfirmDialog），颜色走语义 token；服务端无 `wasm` 键时明确
//! 提示平台不可用，不伪造空清单。

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
import com.gearui.foundation.primitives.Text
import com.gearui.theme.Theme
import com.tencent.kuikly.compose.foundation.layout.Column
import com.tencent.kuikly.compose.foundation.layout.Row
import com.tencent.kuikly.compose.foundation.layout.fillMaxWidth
import com.tencent.kuikly.compose.foundation.layout.padding
import com.tencent.kuikly.compose.ui.Alignment
import com.tencent.kuikly.compose.ui.Modifier
import com.tencent.kuikly.compose.ui.unit.dp
import com.xiaozhi.admin.contracts.IconToken
import com.xiaozhi.admin.icons.GearIcon
import com.xiaozhi.admin.kit.actionRow
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
    val snap = meta.wasmSlots

    Column(modifier = Modifier.fillMaxWidth()) {
        if (meta.loadError.isNotEmpty()) {
            LabeledCard(title = "插件平台") {
                WarnLine(meta.loadError + "（确认服务端地址可达后点「刷新清单」）", error = true)
            }
        } else if (snap == null) {
            LabeledCard(title = "插件平台不可用") {
                WarnLine(
                    "服务端未启用 WASM 插件平台：需要以 `wasm-plugins` feature 构建的服务端" +
                        "（旧版本服务端没有该能力）。平台可用时这里会列出已装载插件、扩展点解析与上传入口。",
                )
            }
        }

        // 上传入口：id 缺省 = 服务端从组件 manifest 探测（"选文件即上传"）
        LabeledCard(title = "上传插件组件") {
            HintLine(
                "选择本地 .wasm 组件上传：服务端做隔离验证（manifest / 注册包 / 自检）后才采用；" +
                    "失败不影响已装载插件。上传落盘为持久意图，重启自动重装。",
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

        // 已装载清单
        LabeledCard(title = "已装载插件") {
            val plugins = snap?.plugins
            if (plugins.isNullOrEmpty()) {
                HintLine("暂无已装载插件：上传组件，或检查启动引导日志（数据卷缺产物时按引导表构建）。")
            } else {
                for (plugin in plugins) {
                    Row(modifier = Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
                        Text(
                            text = plugin.title,
                            modifier = Modifier.weight(1f),
                            style = Theme.typography.bodyMedium,
                            color = Theme.colors.foreground,
                        )
                        if (!plugin.official) {
                            statusBadge(state = "busy", text = "用户实现")
                        }
                        pluginStatusBadge(phase = plugin.phase, enabled = true)
                    }
                    HintLine("${plugin.id} · v${plugin.version}")
                    if (plugin.description.isNotEmpty()) {
                        HintLine(plugin.description)
                    }
                    if (plugin.permissions.isNotEmpty()) {
                        HintLine("权限：" + plugin.permissions.joinToString("、"))
                    }
                    // 动作 + 卸载（危险动作卸载走弹层二次确认；动作按钮按注册清单渲染）
                    actionRow {
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
                            type = ButtonType.OUTLINE, // GearUI 无 DANGER 型：危险意图由「卸载」文案 + 弹层二次确认承载
                            size = ButtonSize.SMALL,
                            iconWidget = { color -> GearIcon(token = IconToken.Delete, size = 14.dp, tint = color) },
                        )
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
                    Row(modifier = Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
                        Text(
                            text = point,
                            modifier = Modifier.weight(1f),
                            style = Theme.typography.bodyMedium,
                            color = Theme.colors.foreground,
                        )
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
    }
}

