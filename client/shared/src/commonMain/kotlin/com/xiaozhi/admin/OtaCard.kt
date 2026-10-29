package com.xiaozhi.admin

import androidx.compose.runtime.Composable
import androidx.compose.runtime.MutableState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import com.gearui.components.alert.Alert
import com.gearui.components.alert.AlertStatus
import com.gearui.components.button.Button
import com.gearui.components.button.ButtonSize
import com.gearui.components.button.ButtonTheme
import com.gearui.components.button.ButtonType
import com.gearui.components.dialog.ConfirmDialog
import com.gearui.components.progress.LinearProgress
import com.gearui.components.progress.ProgressStatus
import com.gearui.components.table.Table
import com.gearui.components.table.TableAlign
import com.gearui.components.table.TableColumn
import com.gearui.foundation.primitives.Text
import com.gearui.theme.Theme
import com.tencent.kuikly.compose.animation.core.LinearEasing
import com.tencent.kuikly.compose.animation.core.RepeatMode
import com.tencent.kuikly.compose.animation.core.animateFloat
import com.tencent.kuikly.compose.animation.core.infiniteRepeatable
import com.tencent.kuikly.compose.animation.core.rememberInfiniteTransition
import com.tencent.kuikly.compose.animation.core.tween
import com.tencent.kuikly.compose.foundation.layout.Column
import com.tencent.kuikly.compose.foundation.layout.fillMaxWidth
import com.tencent.kuikly.compose.foundation.layout.padding
import com.tencent.kuikly.compose.ui.Modifier
import com.tencent.kuikly.compose.ui.unit.dp
import com.tencent.kuikly.core.pager.Pager
import com.xiaozhi.admin.kit.GearUiSpace
import com.xiaozhi.admin.kit.actionRow
import com.xiaozhi.admin.kit.labeledField
import com.xiaozhi.admin.kit.switchRow

/**
 * 「OTA 固件」维护面（配置页 `ota` section 的正文）—— Compose 版，替换旧 DSL 的
 * `ViewContainer.otaCard(ota, toast, ctx)`。
 *
 * ## 为什么这块必须存在
 *
 * 固件托管此前**只能 curl**（`POST /api/ota/firmware?version=…`，服务端只保留"一个最新版"）。
 * 多型号/多功能设备接入后，「哪个设备会拿到哪个包」必须可见可管 —— **发错型号的包会直接变砖**。
 *
 * ## 三张卡 = 三个问题
 *
 * | 卡 | 回答的问题 | 落哪个状态字段 |
 * | --- | --- | --- |
 * | [OtaRulesCard] | 服务端按什么规则下发？包存在哪？ | `summary` / `dir` / `loadError` |
 * | [OtaUploadCard] | 怎么把包传上去？（自动识别 vs 手填覆盖） | `pickedName` / `uploading` / `manual` … |
 * | [OtaEntriesCard] | 现在托管了哪些、谁会拿到、怎么删 | `entries` / `confirmKey` |
 *
 * ## 三条硬性呈现规则
 *
 * 1. **先说清匹配规则再列条目**：管理员不该靠猜「为什么这台没升级」；每条还额外写出
 *    `matchNote`（"会拿到它的设备"），把匹配规则落成人话；
 * 2. **删除要二次确认**（[ConfirmDialog] `destructive = true`、`dismissOnOutside = false`）：
 *    固件是设备变砖级别的资源，一次误点不该生效；
 * 3. **失败给原因 + 怎么修**：上传失败用 `Alert(DANGER)` 常驻显示（toast 只有 3 秒，读不完
 *    一整条排障指引），加载失败用 [WarnLine]。
 *
 * ## 上传进度：为什么是不确定态进度条
 *
 * [OtaState] 没有任何「已上传字节 / 总字节」字段（`uploadPickedFirmware` 的回调只有
 * `(ok, code, body)`），而本任务硬约束是**状态层零改动** —— 因此拿不到真实百分比。
 * 这里用 [LinearProgress] 做**往复扫动的不确定态进度条**（`showLabel = false`，不显示任何
 * 假百分比），配合"已选包名称 + 体积"文案给出"确实在传"的可见反馈；
 * 传完由 `onDone` 的 toast + 列表自动刷新给出确凿结论。
 *
 * ## 上传链路：复用既有 expect/actual，不新写选择器
 *
 * 选包 / 传包全部走 [OtaState.pick] / [OtaState.upload]，平台差异由 `OtaFilePicker.kt` 的
 * `expect fun pickFirmwareFile` / `expect fun uploadPickedFirmware` 承接
 *（macOS 原生 `UIDocumentPicker` + `httpRequestBinary`；web 端 `<input type=file>` + `fetch`）。
 * 本文件**不碰任何 actual 实现，也不碰校验规则**（`OtaState.upload` 里的覆盖字段拼装原样复用）。
 */
@Composable
fun OtaFirmwareCard(ota: OtaState, toast: ToastState) {
    val ctx = LocalAdminPagerOrThrow()
    // 上传结果的就地反馈（失败原因）：纯展示态，**不进状态层**（状态字段零改动）。
    // 过期策略：每次点「上传固件」先清空，故不会把上一次的失败原因带到下一次。
    val feedback = remember { mutableStateOf(OtaUploadFeedback()) }

    Column(modifier = Modifier.fillMaxWidth()) {
        OtaRulesCard(ota = ota, ctx = ctx)
        OtaUploadCard(ota = ota, ctx = ctx, toast = toast, feedback = feedback)
        OtaEntriesCard(ota = ota)
    }

    // 删除二次确认弹层：`Dialog.Host` 走 Overlay，不占布局，故与卡片流并列
    OtaDeleteDialog(ota = ota, ctx = ctx, toast = toast)
}

/**
 * 一次上传动作的结果（`@Composable` 局部记忆，不写回状态层）。
 *
 * 之所以不在状态层加字段：`OtaState` 的字段清单是 T05 的既定契约（零改动硬约束），
 * 而"失败的完整原因"只是当前这次交互的瞬时信息 —— 换 section 就该丢掉。
 *
 * @param ok      `true` = 上传成功（成功后靠 toast + 列表刷新表达，不再常驻告警）
 * @param message 失败原因（成功时为空白）；空串表示无需显示告警
 */
private data class OtaUploadFeedback(
    val ok: Boolean = false,
    val message: String = "",
)

/**
 * 卡 1：规则说明 + 托管位置 + 清单摘要 + 刷新。
 *
 * 「刷新」是幂等只读请求，但仍然在请求中禁点（否则连点会让列表来回闪）。
 */
@Composable
private fun OtaRulesCard(ota: OtaState, ctx: Pager) {
    LabeledCard(title = "OTA 固件托管") {
        HintLine(
            "服务端按「功能 + 型号」二维托管固件：设备 OTA 时上报 application.name（功能）、" +
                "chip_model_name（型号）、board.type（板型）；三者命中且版本更新才会下发下载地址。" +
                "不同型号 / 不同功能的包绝不互发。",
        )
        SectionSummaryRow(
            summary = ota.summary.ifEmpty { if (ota.loading) "加载中…" else "—" },
            actionText = "刷新清单",
            busy = ota.loading,
            busyText = "刷新中…",
            onAction = { ota.refresh(ctx) },
        )
        // 加载失败：就地报错 + 可行动指引（不弹 toast —— toast 只表达"用户动作的结果"）
        if (ota.loadError.isNotEmpty()) {
            WarnLine(ota.loadError + "（确认服务端地址可达后点「刷新清单」）", error = true)
        }
        // 存储位置可见：一眼定位"固件到底存哪了"
        if (ota.dir.isNotEmpty()) {
            HintLine("存储目录：${ota.dir}")
        }
    }
}

/**
 * 卡 2：选包 + 上传 + 手动覆盖字段。
 *
 * 正常路径（ESP-IDF 包）**不需要手填任何字段** —— 功能/型号/版本由服务端从包内元数据
 * （`chip_id` + `esp_app_desc_t`）解析，所以"手动指定"默认收起。
 */
@Composable
private fun OtaUploadCard(
    ota: OtaState,
    ctx: Pager,
    toast: ToastState,
    feedback: MutableState<OtaUploadFeedback>,
) {
    LabeledCard(title = "上传固件包") {
        HintLine(
            "选择 idf.py build 产出的 app .bin：功能 / 型号 / 版本由包内元数据自动识别（ESP-IDF 镜像头" +
                "的 chip_id + esp_app_desc_t），无需手填。同 功能+型号 再次上传即覆盖旧包。",
        )

        // 已选包：名称 + 体积（体积走 formatBytes，与列表同一套格式化，避免两处口径不同）
        if (ota.hasPicked) {
            Text(
                text = "已选：${ota.pickedName}（${formatBytes(ota.pickedSize)}）",
                modifier = Modifier.fillMaxWidth().padding(top = GearUiSpace.xs),
                style = Theme.typography.bodyMedium,
                color = Theme.colors.primary,
            )
        }
        // 选包失败 / 平台不支持选择本地文件：降级提示（黄），不是异常
        if (ota.pickError.isNotEmpty()) {
            WarnLine(ota.pickError)
        }

        // 上传中：不确定态进度条 + 明确文案（服务端不回报百分比，见文件头说明）
        if (ota.uploading) {
            OtaUncertainProgress(hint = "上传中：${ota.pickedName.ifEmpty { "固件包" }}（${formatBytes(ota.pickedSize)}）" + "；服务端不回报上传百分比，进度条为不确定态")
        }

        // 上传失败：常驻 Alert + 完整原因（401 这类需要照着改打开方式的指引，toast 读不完）
        val current = feedback.value
        if (!ota.uploading && !current.ok && current.message.isNotEmpty()) {
            Alert(
                title = "上传失败",
                description = current.message,
                status = AlertStatus.DANGER,
                modifier = Modifier.padding(top = GearUiSpace.sm),
            )
        }

        actionRow {
            OpsOutlineButton(
                text = "选择 .bin 文件",
                onClick = { ota.pick(ctx) },
                enabled = !ota.uploading,
            )
            // 未选包不可点；上传中显示 loading 并禁点（防重复提交把包传两遍）
            OpsPrimaryButton(
                text = "上传固件",
                onClick = {
                    // 先清掉上一次结果，避免"上一轮的失败原因"在新一轮里闪一下
                    feedback.value = OtaUploadFeedback()
                    ota.upload(ctx) { ok, msg ->
                        // 用户取消选择不是错误：不占用就地告警位，只用中性 toast 说一声
                        val cancelled = msg.startsWith("已取消")
                        feedback.value = OtaUploadFeedback(
                            ok = ok,
                            message = if (ok || cancelled) "" else msg,
                        )
                        toast.show(msg, when {
                            cancelled -> "info"
                            ok -> "ok"
                            else -> "error"
                        })
                    }
                },
                enabled = ota.hasPicked && !ota.uploading,
                loading = ota.uploading,
            )
        }

        // 手动覆盖默认收起：正常路径用不到，展开后才是四个可选字段
        switchRow(
            label = "手动指定功能 / 型号 / 版本（默认自动识别）",
            checked = ota.manual,
            onToggle = { ota.manual = it },
            help = "只有非 ESP-IDF 镜像、或包内元数据缺字段时才需要手填；留空即由服务端自动识别。",
        )
        if (ota.manual) {
            labeledField(
                label = "功能（project）",
                value = ota.project,
                onValueChange = { ota.project = it },
                placeholder = "如 xiaozhi",
            )
            labeledField(
                label = "型号（model）",
                value = ota.model,
                onValueChange = { ota.model = it },
                placeholder = "如 esp32c3 / esp32s3",
            )
            labeledField(
                label = "板型（board，可选）",
                value = ota.board,
                onValueChange = { ota.board = it },
                placeholder = "如 bread-compact-wifi；留空 = 该型号通用",
            )
            labeledField(
                label = "版本（version，可选）",
                value = ota.version,
                onValueChange = { ota.version = it },
                placeholder = "如 2.0.4；留空 = 用包内版本",
            )
        }
    }
}

/**
 * 不确定态进度条：往复扫动 + 一行说明。
 *
 * 为什么不用 `LoadingIndicator`（转圈）而用 [LinearProgress]：上传是"有明确终点的一次性
 * 长动作"，横向进度条比转圈更能表达"正在推进"，且本任务的呈现要求就是进度条。
 *
 * 为什么 `showLabel = false`：拿不到真实百分比，任何数字都是编的 —— 不显示才是诚实的。
 */
@Composable
private fun OtaUncertainProgress(hint: String) {
    val transition = rememberInfiniteTransition()
    val sweep by transition.animateFloat(
        initialValue = 0.08f,
        targetValue = 0.92f,
        animationSpec = infiniteRepeatable(
            // 1500ms 单程：GearUI 的 LinearProgress 内部对目标值还有一段 300ms 补间，
            // 单程太短会被补间追平而看不出扫动，故取一个明显更长的周期。
            animation = tween(durationMillis = 1500, easing = LinearEasing),
            repeatMode = RepeatMode.Reverse,
        ),
    )
    Column(modifier = Modifier.fillMaxWidth().padding(top = GearUiSpace.sm)) {
        LinearProgress(
            progress = sweep,
            modifier = Modifier.fillMaxWidth(),
            status = ProgressStatus.PRIMARY,
            showLabel = false,
        )
        Text(
            text = hint,
            modifier = Modifier.fillMaxWidth().padding(top = GearUiSpace.xs),
            style = Theme.typography.caption,
            color = Theme.colors.mutedForeground,
        )
    }
}

/**
 * 卡 3：已托管清单。
 *
 * 空清单给**可行动的解释**（"上传后设备下次 OTA 检查即可拿到升级"），不是一句"暂无数据"。
 */
@Composable
private fun OtaEntriesCard(ota: OtaState) {
    LabeledCard(title = "已托管固件") {
        if (ota.loaded && ota.entries.isEmpty() && ota.loadError.isEmpty()) {
            HintLine("尚未托管任何固件：上传后，设备下次 OTA 检查（开机 / 重连）即可拿到升级。")
        }
        if (ota.entries.isNotEmpty()) {
            OtaEntriesTable(ota)
            HintLine(
                "大小 / 编译时间右对齐便于纵向比对；「删除」需二次确认，删除后该 功能+型号 的设备" +
                    "不再收到这条升级（已升级到该版本的设备不受影响）。",
            )
        }
    }
}

/**
 * 清单表：固件（含匹配说明）/ 版本 / 大小 / 编译时间 / 操作。
 *
 * ## 为什么 `rowHeight` 显式抬到 72dp
 *
 * GearUI `Table` 的行是**定高**（`Row(Modifier.height(rowHeight))`，默认
 * `ControlGeometry.controlMedium`）。本表"固件"列要放标题 + 匹配说明 + sha256/IDF 三行，
 * 用默认高度会被裁掉后两行 —— 所以必须显式抬高，这是组件的既定行为，不是多余参数。
 *
 * ## 数值列右对齐（P1-2）
 *
 * 大小 / 编译时间是数值列，`TableAlign.RIGHT` + 固定宽度：位数不同也能纵向对齐比较。
 * 版本列居中（版本号是"标签"不是"数量"，居中比右对齐更好扫）。
 */
@Composable
private fun OtaEntriesTable(ota: OtaState) {
    val columns: List<TableColumn<OtaEntryRow>> = listOf(
        TableColumn(
            key = "firmware",
            title = "固件",
            render = { row, _ -> OtaFirmwareCell(row) },
        ),
        TableColumn(
            key = "version",
            title = "版本",
            width = 96.dp,
            align = TableAlign.CENTER,
            render = { row, _ -> OtaPlainCell(row.version.ifEmpty { "—" }) },
        ),
        TableColumn(
            key = "size",
            title = "大小",
            width = 88.dp,
            align = TableAlign.RIGHT,
            render = { row, _ -> OpsNumberCell(formatBytes(row.size)) },
        ),
        TableColumn(
            key = "built",
            title = "编译时间",
            width = 132.dp,
            align = TableAlign.RIGHT,
            render = { row, _ -> OpsNumberCell(row.builtAt.ifEmpty { "—" }) },
        ),
        TableColumn(
            key = "actions",
            title = "操作",
            width = 88.dp,
            align = TableAlign.RIGHT,
            render = { row, _ ->
                // 只做"武装"：真正删除在 ConfirmDialog 确认后调 ota.remove
                Button(
                    text = "删除",
                    onClick = { ota.confirmKey = row.key },
                    theme = ButtonTheme.DEFAULT,
                    type = ButtonType.OUTLINE,
                    size = ButtonSize.SMALL,
                )
            },
        ),
    )
    Table(
        data = ota.entries,
        columns = columns,
        striped = true,
        bordered = true,
        rowHeight = 72.dp,
        emptyText = "尚未托管任何固件",
    )
}

/**
 * 「固件」单元格：标题 + **会拿到它的设备** + sha256/IDF 指纹。
 *
 * 旧版（`legacy`，无型号信息）的匹配说明用警告色 —— 它是"需要处理"的状态，不是普通说明。
 */
@Composable
private fun OtaFirmwareCell(row: OtaEntryRow) {
    Column(modifier = Modifier.fillMaxWidth()) {
        Text(
            text = row.title,
            style = Theme.typography.bodyMedium,
            color = Theme.colors.foreground,
            maxLines = 1,
        )
        Text(
            text = row.matchNote,
            modifier = Modifier.fillMaxWidth().padding(top = 2.dp),
            style = Theme.typography.caption,
            color = if (row.legacy) Theme.colors.warning else Theme.colors.mutedForeground,
        )
        val meta = otaFirmwareMeta(row)
        if (meta.isNotEmpty()) {
            Text(
                text = meta,
                modifier = Modifier.fillMaxWidth().padding(top = 2.dp),
                style = Theme.typography.caption,
                color = Theme.colors.mutedForeground,
                maxLines = 1,
            )
        }
    }
}

/**
 * 指纹行：`IDF v5.1 · sha256 abc123456789`（缺失项自动省略）。
 *
 * 旧实现把它拼在副标题里；拆成独立一行是因为版本/大小/编译时间已经各有自己的列，
 * 再拼在一起会重复显示同一批信息。
 */
private fun otaFirmwareMeta(row: OtaEntryRow): String = listOfNotNull(
    row.idfVer.takeIf { it.isNotEmpty() }?.let { "IDF $it" },
    row.sha256.takeIf { it.isNotEmpty() }?.let { "sha256 ${it.take(12)}" },
).joinToString(" · ")

/** 普通文本单元格（版本列这类"标签"值）。 */
@Composable
private fun OtaPlainCell(text: String) {
    Text(
        text = text,
        style = Theme.typography.bodyMedium,
        color = Theme.colors.foreground,
        maxLines = 1,
    )
}

/**
 * 删除二次确认弹层。
 *
 * 武装目标从 `ota.confirmKey`（`OtaEntryRow.key` = `功能|型号|板型`）反查条目 ——
 * 状态层只存 key，不存条目对象，刷新后旧武装自动失效（找不到就是不可见，等价于复位）。
 *
 * `dismissOnOutside = false`（K4）：点遮罩不关闭，必须明确选「取消」或「删除」。
 */
@Composable
private fun OtaDeleteDialog(ota: OtaState, ctx: Pager, toast: ToastState) {
    val target = ota.entries.firstOrNull { it.key == ota.confirmKey }
    ConfirmDialog(
        visible = target != null,
        title = "删除固件确认",
        message = if (target == null) {
            ""
        } else {
            buildString {
                append("确定删除「")
                append(target.title)
                append("」吗？")
                append("删除后，该 功能+型号 的设备将不再收到这条升级；已升级到该版本的设备不受影响。")
                append("此操作不可撤销。")
            }
        },
        confirmText = "删除",
        cancelText = "取消",
        destructive = true,
        // 点遮罩**不**关闭：删除固件不可撤销，必须明确点「取消」才走开
        dismissOnOutside = false,
        onConfirm = {
            if (target != null) {
                ota.remove(ctx, target) { ok, msg -> toast.show(msg, if (ok) "ok" else "error") }
            }
        },
        onCancel = { ota.confirmKey = "" },
    )
}
