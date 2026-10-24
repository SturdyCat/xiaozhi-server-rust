package com.xiaozhi.admin

import com.xiaozhi.admin.ui.*

import com.tencent.kuikly.core.base.CursorType
import com.tencent.kuikly.core.base.ViewContainer
import com.tencent.kuikly.core.directives.vfor
import com.tencent.kuikly.core.directives.vif
import com.tencent.kuikly.core.views.Text
import com.tencent.kuikly.core.views.View

/**
 * 「文件」页渲染（数据源见 [FilesState]，纯逻辑见 [FilesModel]）。
 *
 * 呈现规则与运维页同一口径（见 OpsPages.kt 文件头）：
 * - 头部 = 面包屑（根「存储目录」+ 各级段，可点回跳）+ 刷新/新建文件夹/上传按钮；
 * - 清单失败就地显示原因 + 重试；首次加载先出骨架；空目录给可行动的解释；
 * - 目录行整行可点进入；文件行只展示；受保护文件（如 data.db）打「保护」徽标
 *   且**不给删除按钮**（服务端也会 403）；
 * - 删除一律**行内二次确认**（[FilesState.armRemove] 武装，再次点击才发请求）；
 * - 上传结果/失败原因走顶部 toast（[FilesState.upload] 回调）。
 *
 * ⚠️ 渲染函数必须是 `ViewContainer` 扩展，并在目标容器闭包内**非限定**调用（同其他页面）；
 * ⚠️ observable（path/confirmDelete/loading…）只在 attr/vif/event 闭包内读取才被响应式跟踪。
 */
fun ViewContainer<*, *>.renderFilesSection(state: FilesState, toast: ToastState, ctx: AdminPage) {
    filesHeader(state, toast, ctx)
    vif({ state.mkdirOpen }) { filesMkdirForm(state, toast, ctx) }
    opsError(state.loadError)
    vif({ state.loading && !state.loaded }) { opsSkeleton() }
    vif({ state.loaded && state.entries.size == 0 && state.loadError.isEmpty() }) {
        textNote(
            "此目录为空。上传模型（如 .onnx）或其他文件后会出现在这里；" +
                "EchoMuse 自定义唤醒词模型请上传到 $OWW_MODELS_DIR/（唤醒词面板会自动列出）。",
            micro = true,
        )
    }
    vfor({ state.entries }) { entry ->
        filesEntryRow(entry, state, toast, ctx)
    }
    vif({ state.entries.size > 0 }) {
        textNote(
            "「删除」对文件夹递归生效，需二次确认；受保护文件服务端拒绝删除。" +
                "上传仅支持 macOS App 与网页端；上传到 $OWW_MODELS_DIR/ 的唤醒词模型可在 EchoMuse 唤醒词面板选择" +
                "（上传 ≠ 安装，设备只认本地已装模型）。",
            micro = true,
        )
    }
}

/** 头部：面包屑（可点回跳）+ 刷新 / 新建文件夹 / 上传。窄屏自动换行。 */
private fun ViewContainer<*, *>.filesHeader(state: FilesState, toast: ToastState, ctx: AdminPage) {
    View {
        attr {
            flexDirectionRow()
            alignItemsCenter()
            flexWrapWrap()
            marginBottom(AdminSpace.sm)
        }
        // 面包屑（占满剩余宽度，按钮靠右）
        View {
            attr {
                flex(1f)
                flexDirectionRow()
                alignItemsCenter()
                flexWrapWrap()
            }
            filesCrumb(
                "存储目录",
                isCurrent = { state.path.isEmpty() },
            ) { state.openPath(ctx, "") }
            vfor({ state.crumbs }) { crumb ->
                Text {
                    attr {
                        fontSize(AdminType.body)
                        color(AdminColors.textTertiary)
                        text(" / ")
                    }
                }
                filesCrumb(
                    crumb.first,
                    isCurrent = { state.path == crumb.second },
                ) { state.openPath(ctx, crumb.second) }
            }
        }
        secondaryButton("刷新", loading = { state.loading }, loadingText = "刷新中…", icon = IconToken.Refresh) {
            state.refresh(ctx)
        }
        hGap(AdminSpace.sm)
        secondaryButton("新建文件夹", icon = IconToken.Folder) {
            state.mkdirDraft = ""
            state.mkdirOpen = !state.mkdirOpen
        }
        hGap(AdminSpace.sm)
        primaryButton(
            "上传模型或文件",
            loading = { state.uploading },
            loadingText = "上传中…",
            icon = IconToken.Document,
        ) {
            state.upload(ctx) { msg, level -> toast.show(msg, level) }
        }
    }
    vif({ state.root.isNotEmpty() }) {
        textNote("存储根目录：${state.root}", micro = true)
    }
}

/**
 * 面包屑段：当前段 textPrimary 不可点，其余 accent 绿可点（悬停指针）。
 * ⚠️ isCurrent 是 lambda——path 是 observable，只有在 attr 闭包内求值才会随跳转变色。
 */
private fun ViewContainer<*, *>.filesCrumb(label: String, isCurrent: () -> Boolean, onClick: () -> Unit) {
    Text {
        attr {
            fontSize(AdminType.body)
            fontWeightMedium()
            color(if (isCurrent()) AdminColors.textPrimary else AdminColors.accentTintText)
            text(label)
            if (!isCurrent()) cursor(CursorType.POINTER)
        }
        event { click { if (!isCurrent()) onClick() } }
    }
}

/** 「新建文件夹」表单卡：名称 1~64 字符（校验规则与 [FilesModel.validateFolderName] 一致）。 */
private fun ViewContainer<*, *>.filesMkdirForm(state: FilesState, toast: ToastState, ctx: AdminPage) {
    groupedCard("新建文件夹（位于当前目录）") {
        labeledField(
            "文件夹名称（1～64 字符，不含 / \\ 等非法字符）",
            { state.mkdirDraft },
            { state.mkdirDraft = it },
            "如 oww-models",
        )
        actionRow {
            primaryButton("创建", loading = { state.actionBusy }, loadingText = "创建中…") {
                state.mkdir(ctx) { ok, msg -> toast.show(msg, if (ok) "ok" else "error") }
            }
            hGap(AdminSpace.sm)
            secondaryButton("取消") {
                state.mkdirOpen = false
                state.mkdirDraft = ""
            }
        }
    }
}

/**
 * 一个条目行：图标 + 名称（目录可点进入）+ 尺寸/修改时间 + 保护徽标 + 删除（二次确认）。
 * ⚠️ 删除的武装判断读 confirmDelete/path 两个 observable，必须留在 vif 闭包内；
 * entry 本身是不可变快照，构建期取值安全。
 */
private fun ViewContainer<*, *>.filesEntryRow(
    entry: FilesEntry,
    state: FilesState,
    toast: ToastState,
    ctx: AdminPage,
) {
    View {
        attr {
            flexDirectionRow()
            alignItemsCenter()
            paddingTop(AdminSpace.xs)
            paddingBottom(AdminSpace.xs)
        }
        appIcon(
            { if (entry.dir) IconToken.Folder else IconToken.Document },
            size = 18f,
            tint = { if (entry.dir) AdminColors.accentTintText else AdminColors.textTertiary },
        )
        // 名称：目录整块可点进入；文件只展示（不与删除按钮争点击）
        View {
            attr {
                flex(1f)
                flexDirectionRow()
                alignItemsCenter()
                marginLeft(AdminSpace.sm)
                if (entry.dir) cursor(CursorType.POINTER)
            }
            event { click { if (entry.dir) state.openPath(ctx, FilesModel.joinPath(state.path, entry.name)) } }
            Text {
                attr {
                    flex(1f)
                    fontSize(AdminType.body)
                    fontWeightMedium()
                    color(AdminColors.textPrimary)
                    lines(1)
                    text(if (entry.dir) "${entry.name}/" else entry.name)
                }
            }
        }
        Text {
            attr {
                marginLeft(AdminSpace.sm)
                fontSize(AdminType.caption)
                color(AdminColors.textTertiary)
                text(FilesModel.entryMeta(entry, localZoneOffsetSeconds()))
            }
        }
        // 受保护文件：只在偏离时打标，且不给删除入口（服务端也会 403）
        if (entry.isProtected) {
            hGap(AdminSpace.sm)
            statusBadge(state = { "busy" }, text = { "保护" })
        } else {
            View {
                attr { flexDirectionRow(); alignItemsCenter(); marginLeft(AdminSpace.sm) }
                vif({ state.confirmDelete != FilesModel.joinPath(state.path, entry.name) }) {
                    secondaryButton("删除") { state.armRemove(entry) }
                }
                vif({ state.confirmDelete == FilesModel.joinPath(state.path, entry.name) }) {
                    primaryButton(
                        "再次点击确认删除",
                        danger = { true },
                        loading = { state.actionBusy },
                        loadingText = "删除中…",
                        icon = IconToken.Delete,
                    ) {
                        state.remove(ctx, entry) { ok, msg -> toast.show(msg, if (ok) "ok" else "error") }
                    }
                    hGap(AdminSpace.xs)
                    secondaryButton("取消") { state.confirmDelete = "" }
                }
            }
        }
        dividerH()
    }
}
