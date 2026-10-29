package com.xiaozhi.admin

import androidx.compose.runtime.Composable
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import com.gearui.components.button.ButtonTheme
import com.gearui.components.button.ButtonType
import com.gearui.components.button.Button
import com.gearui.components.collapse.Collapse
import com.gearui.components.collapse.CollapsePanel
import com.gearui.components.table.Table
import com.gearui.components.table.TableAlign
import com.gearui.components.table.TableColumn
import com.gearui.foundation.primitives.Text
import com.gearui.theme.Theme
import com.tencent.kuikly.compose.foundation.layout.Arrangement
import com.tencent.kuikly.compose.foundation.layout.Column
import com.tencent.kuikly.compose.foundation.layout.Row
import com.tencent.kuikly.compose.foundation.layout.fillMaxWidth
import com.tencent.kuikly.compose.foundation.layout.padding
import com.tencent.kuikly.compose.ui.Alignment
import com.tencent.kuikly.compose.ui.Modifier
import com.tencent.kuikly.compose.ui.unit.dp
import com.xiaozhi.admin.contracts.IconToken
import com.xiaozhi.admin.icons.GearIcon
import com.xiaozhi.admin.kit.GearUiSpace
import com.xiaozhi.admin.kit.actionRow
import com.xiaozhi.admin.kit.labeledField
import com.xiaozhi.admin.kit.statusBadge

/**
 * 设备页（`devices`）—— Compose 版。
 *
 * ## 清单 + 展开详情
 *
 * 上半是 `Table`：设备号 / 偏离状态 / 会话次数 / 轮次 / 活会话 / 最后活跃，
 * **数值列全部右对齐**（P1-2，否则位数不同就对不成列）。点任意一行展开该设备的详情面板
 * （`Collapse`）：协商参数、上次异常、按设备覆盖参数（个性化配置）与消息注入入口。
 * 详情默认全部收起 —— 一屏 N 台设备每人一个输入框不可用。
 *
 * ## 「只在偏离时打标」
 *
 * 在线是正常态，**不渲染徽标**；列里给一行弱化文字「在线」即可。只有「已断开」「测试台」
 * 这种偏离态才占视觉（[statusBadge]）。
 *
 * ## 本页刻意只读
 *
 * 除「个性化配置」（P1-5：以该设备为编辑目标跳「语音流程」）与「注入消息」（P1-2）外，
 * 没有绑定/解绑/工具调用等写操作 —— 那些要动设备侧状态，本期不做。
 */
@Composable
fun OpsDevicesPage(
    devices: DevicesState,
    form: ConfigFormState,
    onEditDevice: (String) -> Unit,
) {
    val ctx = LocalAdminPagerOrThrow()
    // 展开的设备号集合：纯展示态，不进状态层（状态层字段零改动）
    val expanded = remember { mutableStateOf(emptySet<String>()) }

    OpsPageFrame(
        summary = devices.summary,
        loading = devices.loading,
        loaded = devices.loaded,
        loadError = devices.loadError,
        onRefresh = { devices.refresh(ctx) },
    ) {
        if (devices.loaded && devices.devices.isEmpty()) {
            OpsEmptyState(
                message = "还没有任何设备连过",
                description = devices.note.ifEmpty {
                    "设备只在唤醒 / 按键触发对话时才建立连接（开机只走 OTA），" +
                        "所以刚部署、或设备一直没被唤醒时这里是空的。"
                },
            )
        }
        if (devices.devices.isNotEmpty()) {
            val opened = devices.devices.filter { it.deviceId in expanded.value }
            LabeledCard(title = "设备清单（共 ${devices.devices.size} 台）") {
                DevicesTable(
                    devices = devices,
                    onToggle = { id ->
                        expanded.value = if (id in expanded.value) {
                            expanded.value - id
                        } else {
                            expanded.value + id
                        }
                    },
                )
                HintLine("点一行展开详情：协商参数、上次异常、按设备覆盖参数与消息注入。在线是正常态，不单独打标。")
            }
            if (opened.isNotEmpty()) {
                Collapse(
                    children = opened.map { row ->
                        CollapsePanel(
                            value = row.deviceId,
                            isExpanded = true,
                            headerBuilder = { DeviceDetailHeader(row) },
                            body = {
                                DeviceDetailBody(
                                    row = row,
                                    devices = devices,
                                    form = form,
                                    onEditDevice = onEditDevice,
                                )
                            },
                        )
                    },
                    // 点标题栏 = 收起该设备（Collapse 的展开态由外部持有，故这里回写集合）
                    expansionCallback = { index, _ ->
                        val target = opened.getOrNull(index)
                        if (target != null) expanded.value = expanded.value - target.deviceId
                    },
                    modifier = Modifier.fillMaxWidth().padding(bottom = GearUiSpace.lg),
                )
            }
        }
        if (devices.note.isNotEmpty()) {
            HintLine(devices.note)
        }
    }
}

/** 清单表：数值列右对齐；整行可点即展开/收起详情。 */
@Composable
private fun DevicesTable(devices: DevicesState, onToggle: (String) -> Unit) {
    val columns: List<TableColumn<DeviceRow>> = listOf(
        TableColumn(
            key = "device",
            title = "设备",
            render = { row, _ -> DeviceIdCell(row) },
        ),
        TableColumn(
            key = "state",
            title = "状态",
            width = 96.dp,
            align = TableAlign.CENTER,
            render = { row, _ -> DeviceStateCell(row) },
        ),
        TableColumn(
            key = "sessions",
            title = "会话",
            width = 72.dp,
            align = TableAlign.RIGHT,
            render = { row, _ -> OpsNumberCell("${row.sessionCount}") },
        ),
        TableColumn(
            key = "turns",
            title = "轮次",
            width = 72.dp,
            align = TableAlign.RIGHT,
            render = { row, _ -> OpsNumberCell("${row.turnCount}") },
        ),
        TableColumn(
            key = "live",
            title = "活会话",
            width = 80.dp,
            align = TableAlign.RIGHT,
            render = { row, _ -> OpsNumberCell("${row.liveSessions}") },
        ),
        TableColumn(
            key = "age",
            title = "最后活跃",
            width = 104.dp,
            align = TableAlign.RIGHT,
            render = { row, _ -> OpsNumberCell(row.ageText) },
        ),
    )
    Table(
        data = devices.devices,
        columns = columns,
        bordered = true,
        striped = true,
        onRowClick = { row, _ -> onToggle(row.deviceId) },
        emptyText = "暂无设备",
    )
}

/** 设备号单元格：没报 Device-Id 的设备用括号占位（服务端可能给空串）。 */
@Composable
private fun DeviceIdCell(row: DeviceRow) {
    Text(
        text = row.deviceId.ifEmpty { "(设备未报 Device-Id)" },
        style = Theme.typography.bodyMedium,
        color = Theme.colors.foreground,
        maxLines = 1,
    )
}

/**
 * 状态单元格：**正常不打标**。
 *
 * - 在线（真实设备）→ 弱化的「在线」二字，不占视觉；
 * - 已断开 → `idle` 灰徽标；
 * - 测试台连接 → `busy` 黄徽标（它不是真实设备，指标含义不同，必须区分）。
 */
@Composable
private fun DeviceStateCell(row: DeviceRow) {
    when {
        !row.online -> statusBadge(state = "idle", text = "已断开")
        row.isTest -> statusBadge(state = "busy", text = "测试台")
        else -> Text(
            text = "在线",
            style = Theme.typography.caption,
            color = Theme.colors.mutedForeground,
        )
    }
}

/** 详情面板标题栏：设备号 + 偏离徽标 + 一句话活跃度。 */
@Composable
private fun DeviceDetailHeader(row: DeviceRow) {
    Row(
        modifier = Modifier.fillMaxWidth(),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(GearUiSpace.xs),
    ) {
        Column(modifier = Modifier.weight(1f)) {
            Text(
                text = row.deviceId.ifEmpty { "(设备未报 Device-Id)" },
                style = Theme.typography.titleSmall,
                color = Theme.colors.foreground,
                maxLines = 1,
            )
            Text(
                text = row.activity,
                style = Theme.typography.caption,
                color = Theme.colors.mutedForeground,
                maxLines = 1,
            )
        }
        if (!row.online) {
            statusBadge(state = "idle", text = "已断开")
        } else if (row.isTest) {
            statusBadge(state = "busy", text = "测试台")
        }
    }
}

/**
 * 详情正文：协商参数 / 上次异常 / Client-Id / 「个性化配置」/ 消息注入。
 *
 * 注入表单用 `devices.injectTarget` 控制展开（状态层既有字段，一次只展开一台设备）。
 */
@Composable
private fun DeviceDetailBody(
    row: DeviceRow,
    devices: DevicesState,
    form: ConfigFormState,
    onEditDevice: (String) -> Unit,
) {
    val ctx = LocalAdminPagerOrThrow()
    Column(modifier = Modifier.fillMaxWidth()) {
        HintLine(row.subtitle)
        HintLine(row.activity)
        // 上次会话异常：这是"设备为什么突然不出声"的第一现场，必须显示
        if (row.lastError.isNotEmpty()) {
            WarnLine("上次会话异常：${row.lastError}", error = true)
        }
        if (row.clientId.isNotEmpty() && row.clientId != row.deviceId) {
            HintLine("Client-Id ${row.clientId}")
        }
        // 个性化配置入口（P1-5）：按设备覆盖会话内参数（音色/语速/提示词/人格等白名单字段）。
        // 测试台连接不显示（is_test 不是真实设备，个性化无意义）。
        if (!row.isTest && row.deviceId.isNotEmpty()) {
            actionRow {
                Button(
                    text = if (form.editDeviceId == row.deviceId) "正在编辑该设备" else "个性化配置",
                    onClick = { onEditDevice(row.deviceId) },
                    theme = ButtonTheme.DEFAULT,
                    type = ButtonType.OUTLINE,
                    iconWidget = {
                        GearIcon(
                            token = IconToken.Settings,
                            size = 16.dp,
                            tint = Theme.colors.foreground,
                        )
                    },
                )
            }
        }
        // 设备消息注入（P1-2）：在线的真实设备才能注入（文本作为用户话语走完整管线）；
        // 测试台连接不显示入口（服务端也会拒绝——它走独立请求-响应，语义不通）。
        if (row.online && !row.isTest) {
            actionRow {
                if (devices.injectTarget != row.deviceId) {
                    OpsOutlineButton(
                        text = "注入消息",
                        onClick = { devices.toggleInject(row.deviceId) },
                    )
                } else {
                    OpsOutlineButton(
                        text = "收起",
                        onClick = { devices.toggleInject(row.deviceId) },
                    )
                }
            }
            if (devices.injectTarget == row.deviceId) {
                labeledField(
                    label = "注入文本（作为用户话语走完整 LLM→TTS 管线）",
                    value = devices.injectDraft,
                    onValueChange = { devices.injectDraft = it },
                    placeholder = "例如：现在几点了",
                )
                actionRow {
                    OpsPrimaryButton(
                        text = if (devices.injectBusy) "注入中…" else "发送",
                        onClick = { devices.inject(ctx, row.deviceId) },
                        enabled = !devices.injectBusy,
                        loading = devices.injectBusy,
                    )
                }
                if (devices.injectMsg.isNotEmpty()) {
                    if (devices.injectMsg.startsWith("✓")) {
                        HintLine(devices.injectMsg)
                    } else {
                        WarnLine(devices.injectMsg, error = true)
                    }
                }
            }
        }
    }
}
