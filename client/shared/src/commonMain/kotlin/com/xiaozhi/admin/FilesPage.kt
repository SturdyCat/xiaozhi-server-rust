package com.xiaozhi.admin

import androidx.compose.runtime.Composable
import com.gearui.components.button.Button
import com.gearui.components.button.ButtonSize
import com.gearui.components.button.ButtonTheme
import com.gearui.components.button.ButtonType
import com.gearui.components.cell.Cell
import com.gearui.components.cellgroup.CellGroup
import com.gearui.components.dialog.ConfirmDialog
import com.gearui.foundation.primitives.Text
import com.gearui.theme.Theme
import com.tencent.kuikly.compose.foundation.clickable
import com.tencent.kuikly.compose.foundation.layout.Row
import com.tencent.kuikly.compose.foundation.layout.fillMaxWidth
import com.tencent.kuikly.compose.foundation.layout.padding
import com.tencent.kuikly.compose.ui.Alignment
import com.tencent.kuikly.compose.ui.Modifier
import com.tencent.kuikly.compose.ui.unit.dp
import com.tencent.kuikly.core.pager.Pager
import com.xiaozhi.admin.contracts.IconToken
import com.xiaozhi.admin.icons.GearIcon
import com.xiaozhi.admin.kit.LocalAdminSpacing
import com.xiaozhi.admin.kit.actionRow
import com.xiaozhi.admin.kit.labeledField
import com.xiaozhi.admin.kit.pageScroller
import com.xiaozhi.admin.kit.statusBadge

/**
 * 「文件」页（`files`）—— Compose 版：服务端存储目录浏览 + 新建文件夹 / 上传 / 删除。
 *
 * 数据源 [FilesState]，纯逻辑 [FilesModel]（路径校验 / 清单解析 / 尺寸与时间格式化）。
 *
 * ## 结构
 *
 * 1. **面包屑**（根「存储目录」+ 各级段，可点回跳）+ 刷新 / 新建文件夹 / 上传；
 * 2. 清单失败**就地**显示原因 + 「刷新」（`Alert(DANGER)`），首次加载先出骨架；
 * 3. 空目录给可行动的解释（含 `oww-models/` 的用途）；
 * 4. 目录与文件一律 `CellGroup` + `Cell`：目录整行可点进入，文件行只展示；
 *    尺寸 / 修改时间走 `Cell` 的 `note`（行尾，天然右对齐），受保护文件打「保护」徽标
 *    且**不给删除按钮**（服务端也会 403）。
 *
 * ## 删除一律二次确认（`ConfirmDialog(destructive)`）
 *
 * 点「删除」→ `state.armRemove(entry)` 武装 → 弹 `ConfirmDialog`（`destructive = true`、
 * `dismissOnOutside = false`，即**点遮罩不关闭**，必须明确选择）。确认后
 * `state.remove(...)`；取消则 `state.confirmDelete = ""` 复位。
 * 换目录 / 刷新也会复位（[FilesState.load] 里已有）。
 */
@Composable
fun FilesPage(state: FilesState, toast: ToastState) {
    val ctx = LocalAdminPagerOrThrow()

    pageScroller {
        FilesBreadcrumbs(state) { rel -> state.openPath(ctx, rel) }

        actionRow {
            OpsOutlineButton(
                text = "刷新",
                onClick = { state.refresh(ctx) },
                loading = state.loading,
                enabled = !state.loading,
            )
            OpsOutlineButton(
                text = "新建文件夹",
                onClick = {
                    state.mkdirDraft = ""
                    state.mkdirOpen = !state.mkdirOpen
                },
            )
            OpsPrimaryButton(
                text = "上传模型或文件",
                onClick = { state.upload(ctx) { msg, level -> toast.show(msg, level) } },
                loading = state.uploading,
                enabled = !state.uploading,
            )
        }

        if (state.root.isNotEmpty()) {
            HintLine("存储根目录：${state.root}")
        }
        if (state.loadError.isNotEmpty()) {
            OpsLoadError(message = state.loadError, onRefresh = { state.refresh(ctx) })
        }

        if (state.loading && !state.loaded) {
            OpsSkeleton()
        } else {
            if (state.mkdirOpen) {
                FilesMkdirForm(state, toast)
            }
            if (state.loaded && state.entries.isEmpty() && state.loadError.isEmpty()) {
                OpsEmptyState(
                    message = "此目录为空",
                    description = "上传模型（如 .onnx）或其他文件后会出现在这里；" +
                        "EchoMuse 自定义唤醒词模型请上传到 $OWW_MODELS_DIR/（唤醒词面板会自动列出）。",
                )
            }
            if (state.entries.isNotEmpty()) {
                CellGroup(
                    items = state.entries,
                    title = "当前目录（${state.entries.size} 项）",
                    key = { it.name },
                    itemContent = { entry -> FilesEntryRow(entry, state, ctx) },
                )
                HintLine(
                    "「删除」对文件夹递归生效，需二次确认；受保护文件服务端拒绝删除。" +
                        "上传仅支持 macOS App 与网页端；上传到 $OWW_MODELS_DIR/ 的唤醒词模型可在" +
                        " EchoMuse 唤醒词面板选择（上传 ≠ 安装，设备只认本地已装模型）。",
                )
            }
        }
    }

    // 二次确认弹层：`Dialog.Host` 走 Overlay，不占布局，故与滚动区并列即可
    FilesDeleteDialog(state, toast)
}

/** 面包屑行：根「存储目录」+ 各级段；当前段不可点、其余用品牌绿。 */
@Composable
private fun FilesBreadcrumbs(state: FilesState, onOpen: (String) -> Unit) {
    Row(
        modifier = Modifier.fillMaxWidth().padding(bottom = LocalAdminSpacing.current.xs),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        FilesCrumb(label = "存储目录", current = state.path.isEmpty()) { onOpen("") }
        for (crumb in state.crumbs) {
            Text(
                text = " / ",
                style = Theme.typography.bodyMedium,
                color = Theme.colors.mutedForeground,
            )
            FilesCrumb(
                label = crumb.first,
                current = state.path == crumb.second,
            ) { onOpen(crumb.second) }
        }
    }
}

/** 单个面包屑段：当前段 `foreground` 不可点，其余 `primary` 可点。 */
@Composable
private fun FilesCrumb(label: String, current: Boolean, onClick: () -> Unit) {
    Text(
        text = label,
        modifier = if (current) {
            Modifier
        } else {
            Modifier.clickable { onClick() }.padding(horizontal = 2.dp)
        },
        style = Theme.typography.bodyMedium,
        color = if (current) Theme.colors.foreground else Theme.colors.primary,
        maxLines = 1,
    )
}

/** 「新建文件夹」表单卡：名称 1~64 字符（校验规则与 [FilesModel.validateFolderName] 一致）。 */
@Composable
private fun FilesMkdirForm(state: FilesState, toast: ToastState) {
    val ctx = LocalAdminPagerOrThrow()
    LabeledCard(title = "新建文件夹（位于当前目录）") {
        labeledField(
            label = "文件夹名称（1～64 字符，不含 / 等非法字符）",
            value = state.mkdirDraft,
            onValueChange = { state.mkdirDraft = it },
            placeholder = "如 oww-models",
        )
        actionRow {
            OpsPrimaryButton(
                text = "创建",
                onClick = {
                    state.mkdir(ctx) { ok, msg -> toast.show(msg, if (ok) "ok" else "error") }
                },
                loading = state.actionBusy,
                enabled = !state.actionBusy,
            )
            OpsOutlineButton(
                text = "取消",
                onClick = {
                    state.mkdirOpen = false
                    state.mkdirDraft = ""
                },
            )
        }
    }
}

/** 一个条目行：图标 + 名称（目录可点进入）+ 尺寸/修改时间（行尾）+ 保护徽标 / 删除。 */
@Composable
private fun FilesEntryRow(entry: FilesEntry, state: FilesState, ctx: Pager) {
    val meta = FilesModel.entryMeta(entry, localZoneOffsetSeconds())
    Cell(
        title = if (entry.dir) "${entry.name}/" else entry.name,
        note = meta.ifEmpty { null },
        arrow = entry.dir,
        onClick = entryOpenAction(state, ctx, entry),
        leading = {
            GearIcon(
                token = if (entry.dir) IconToken.Folder else IconToken.Document,
                size = 18.dp,
                tint = if (entry.dir) Theme.colors.primary else Theme.colors.mutedForeground,
            )
        },
        trailing = {
            if (entry.isProtected) {
                // 受保护文件：只在偏离时打标，且不给删除入口（服务端也会 403）
                statusBadge(state = "busy", text = "保护")
            } else {
                Button(
                    text = "删除",
                    onClick = { state.armRemove(entry) },
                    theme = ButtonTheme.DEFAULT,
                    type = ButtonType.OUTLINE,
                    size = ButtonSize.SMALL,
                )
            }
        },
    )
}

/**
 * 条目的点击动作：目录进入子目录，文件不给动作（`null` = 该行不可点）。
 *
 * 单独抽成函数而不是写行内的 `if (...) { ... } else null` —— 后者会被 Kotlin 解析成
 * "if 块 + null"，类型对不上，是这种写法最常见的坑。
 */
private fun entryOpenAction(state: FilesState, ctx: Pager, entry: FilesEntry): (() -> Unit)? =
    if (entry.dir) {
        { state.openPath(ctx, FilesModel.joinPath(state.path, entry.name)) }
    } else {
        null
    }

/**
 * 删除二次确认弹层。
 *
 * 武装目标从 `state.confirmDelete`（相对路径）反查条目 —— 状态层只存路径，不存条目对象，
 * 这样换目录后旧武装自动失效（`load` 会复位 `confirmDelete`）。
 */
@Composable
private fun FilesDeleteDialog(state: FilesState, toast: ToastState) {
    val ctx = LocalAdminPagerOrThrow()
    val target = state.entries.firstOrNull {
        FilesModel.joinPath(state.path, it.name) == state.confirmDelete
    }
    ConfirmDialog(
        visible = target != null,
        title = "删除确认",
        message = if (target == null) {
            ""
        } else {
            buildString {
                append("确定删除「")
                append(target.name)
                append("」吗？")
                if (target.dir) append("文件夹会连同其中的全部内容一起删除。")
                append("此操作不可撤销。")
            }
        },
        confirmText = "删除",
        cancelText = "取消",
        destructive = true,
        // 点遮罩**不**关闭：删除是不可撤销动作，必须明确点「取消」才走开
        dismissOnOutside = false,
        enabled = !state.actionBusy,
        onConfirm = {
            if (target != null) {
                state.remove(ctx, target) { ok, msg -> toast.show(msg, if (ok) "ok" else "error") }
            }
        },
        onCancel = { state.confirmDelete = "" },
    )
}
