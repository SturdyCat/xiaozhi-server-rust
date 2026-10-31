package com.xiaozhi.admin

import androidx.compose.runtime.Composable
import com.gearui.components.button.Button
import com.gearui.components.button.ButtonTheme
import com.gearui.components.button.ButtonType
import com.tencent.kuikly.compose.ui.unit.dp
import com.xiaozhi.admin.contracts.IconToken
import com.xiaozhi.admin.icons.GearIcon
import com.xiaozhi.admin.kit.LocalAdminSpacing
import com.xiaozhi.admin.kit.actionRow
import com.xiaozhi.admin.kit.fieldLabelRow
import com.xiaozhi.admin.kit.labeledField
import com.xiaozhi.admin.kit.labeledTextArea

/**
 * 设备卡的「蓝牙代理」面板（Stage 06 的内容）：通用 GATT 工具，
 * 不是设备专属开关协议或实时通知面板。
 *
 * ## 面板的四段结构（与旧实现同序，编号即操作顺序）
 *
 * 1. **广播快照与目标外设**：读取设备端广播快照 → 从快照里选目标（或手填 MAC）；
 * 2. **槽位、连接与服务发现**：查槽位 → 连接 → 发现服务（服务树在这里展开）；
 * 3. **特征 / 描述符读写**：handle 十进制手填，读 / 写 hex（写入必须二次确认）；
 * 4. **手动读取通知与断开事件**：`GET state` 只读最近的有界事件，不订阅、不轮询。
 *
 * ## 两条安全红线（文案与状态层同一口径，不许软化）
 *
 * - **写入必须二次确认**：`prepareWrite()` 先落"写入确认"意图（含桥接设备 / 会话 /
 *   外设 / handle / hex），点「确认写入上述字节」才真发；任何草稿、目标或会话变化都会
 *   清掉确认（`clearConfirmation`）——这是防止"看着 A 外设把字节写进 B 外设"的唯一闸。
 * - **无配对/加密**：需要安全属性可能返回 `att:5`；通用字节不具备开关等业务语义，
 *   先查外设协议再动。
 *
 * ⚠️ 广播/属性清单改用普通 `for` 循环（页面级滚动）：旧版是卡内 `Scroller` 定高 240/320，
 * 那是为了不让内滚吞掉页面滚动；Compose 的页面滚动在本页由外层 `ScrollView` 承担，
 * 清单直接排进页面流，少一层滚动嵌套，也少一处高度魔法。
 */
@Composable
internal fun EchoMuseBlePanel(row: EchoMuseDeviceRow, state: EchoMuseState, showSettings: Boolean = true) {
    val ble = state.ble
    val pager = LocalAdminPagerOrThrow()

    EmGroupLabel("通用 BLE / GATT 管理", topGap = LocalAdminSpacing.current.xs)
    if (ble.status.isNotEmpty()) EmNote(ble.status)
    EmNote(
        "仅面向你拥有或获授权的 IoT 外设。无配对/加密，需要安全属性可能返回 att:5。" +
            "通用字节不具备开关等业务语义，先查外设协议。",
    )
    if (showSettings) EchoMuseBleSettings(ble = ble, state = state)
    else EmNote("BLE 扫描与连接开关请在「官方 UI → 配置 → 蓝牙」中设置。")

    // ---- 1. 广播快照与目标外设 ----
    fieldLabelRow("1. 广播快照与目标外设")
    actionRow {
        Button(
            text = "读取广播快照",
            onClick = { ble.readAdverts(pager) },
            type = ButtonType.OUTLINE,
            loading = ble.busy == "ble/adverts",
            disabled = !(ble.canRead && ble.idle),
            iconWidget = { GearIcon(token = IconToken.Refresh, size = 16.dp) },
        )
    }
    EmNote(ble.advertsSummary)
    for (advert in ble.adverts) {
        EmBleAdvertCard(ble = ble, advert = advert)
    }
    labeledField("目标 MAC 地址", ble.address, { ble.applyAddress(it) }, placeholder = "aa:bb:cc:dd:ee:ff")
    actionRow {
        Button(
            text = if (ble.addressType == 0) "地址：public (0)" else "地址：random (1)",
            onClick = { ble.applyAddressType(if (ble.addressType == 0) 1 else 0) },
            type = ButtonType.OUTLINE,
        )
    }
    EmNote("手填 MAC 时必须核对 public/random 地址类型；地址可能轮换。选择外设不会自动连接，也不会自动执行读写。")

    // ---- 2. 槽位、连接与服务发现 ----
    fieldLabelRow("2. 槽位、连接与服务发现")
    actionRow {
        Button(
            text = "查询槽位",
            onClick = { ble.querySlots(pager) },
            type = ButtonType.OUTLINE,
            loading = ble.busy == "slots",
            disabled = !(ble.canGatt && ble.idle),
            iconWidget = { GearIcon(token = IconToken.Refresh, size = 16.dp) },
        )
        Button(
            text = "连接外设",
            onClick = { ble.connect(pager) },
            type = ButtonType.OUTLINE,
            loading = ble.busy == "connect",
            disabled = !(ble.canGatt && ble.idle),
            iconWidget = { GearIcon(token = IconToken.Bluetooth, size = 16.dp) },
        )
    }
    actionRow {
        Button(
            text = "断开目标",
            onClick = { ble.disconnect(pager) },
            type = ButtonType.OUTLINE,
            loading = ble.busy == "disconnect",
            disabled = !(ble.canGatt && ble.idle),
            iconWidget = { GearIcon(token = IconToken.Close, size = 16.dp) },
        )
        Button(
            text = "发现服务",
            onClick = { ble.discover(pager) },
            type = ButtonType.OUTLINE,
            loading = ble.busy == "services",
            disabled = !(ble.canGatt && ble.idle),
            iconWidget = { GearIcon(token = IconToken.Refresh, size = 16.dp) },
        )
    }
    EmOutputBlock(text = ble.slotsOutput, height = 180.dp)
    for (item in ble.attributes) {
        EmBleAttributeCard(ble = ble, item = item)
    }

    // ---- 3. 特征 / 描述符读写 ----
    fieldLabelRow("3. 特征 / 描述符读写")
    EmNote("输入十进制 handle (1～65535)。特征读写使用 value_handle，不是声明 handle；描述符（如 UUID 0x2902 CCCD）使用描述符自身 handle。")
    // Keep raw text: validation must reject, not turn pasted -1/1.2 into a different handle.
    labeledField("十进制 handle", ble.handleDraft, { ble.setHandle(it) }, placeholder = "1～65535")
    actionRow {
        Button(
            text = "读取 handle",
            onClick = { ble.read(pager) },
            type = ButtonType.OUTLINE,
            loading = ble.busy == "read",
            disabled = !(ble.canGatt && ble.idle),
            iconWidget = { GearIcon(token = IconToken.Refresh, size = 16.dp) },
        )
    }
    if (ble.readOutput.isNotEmpty()) {
        EmOutputBlock(text = ble.readOutput, height = 180.dp)
    }
    labeledTextArea(
        label = "写入 hex（非空，最多512字节）",
        value = ble.hexDraft,
        onValueChange = { ble.setHex(it) },
        placeholder = "01 00",
        minLines = 3,
    )
    EmNote(
        run {
            val bytes = bleHexToBytes(ble.hexDraft)
            if (bytes == null) {
                "仅接受完整十六进制字节与空格/换行；不接受0x前缀，拒绝奇数位、非法字符及超过512字节"
            } else {
                "将发送 ${bytes.size} 字节；base64: ${bleHexToBase64(ble.hexDraft)}"
            }
        },
    )
    actionRow {
        Button(
            text = if (ble.responseDraft) "有响应写 (response=true)" else "无响应写 (response=false)",
            onClick = { ble.toggleResponse() },
            type = ButtonType.OUTLINE,
        )
    }
    EmNote(
        "有响应成功仅代表 ATT 接受，不证明 IoT 业务动作完成；无响应写只表示设备发送，" +
            "不代表外设接受。请读回或核对外设状态，避免盲目重试。",
    )
    actionRow {
        Button(
            text = "检查并准备写入",
            onClick = { ble.prepareWrite() },
            type = ButtonType.OUTLINE,
            disabled = !(ble.canGatt && ble.idle),
            iconWidget = { GearIcon(token = IconToken.Check, size = 16.dp) },
        )
    }
    if (ble.confirmed != null) {
        val intent = ble.confirmed
        EmWarn(
            "写入确认：桥接设备 ${intent?.deviceId}\nsession_id=${intent?.sessionId}\n" +
                "外设 ${intent?.address} / addr_type=${intent?.addressType}\n" +
                "handle=${intent?.handle}；response=${intent?.response}\n" +
                "hex=${intent?.valueBase64?.let(::bleBase64ToHex)}\n" +
                "确认会改变外设状态；任何草稿、目标或会话变化都须重新确认。",
        )
        actionRow {
            Button(
                text = "确认写入上述字节",
                onClick = { ble.confirmWrite(pager) },
                theme = ButtonTheme.DANGER,
                type = ButtonType.FILL,
                disabled = !(ble.canGatt && ble.idle),
                iconWidget = { GearIcon(token = IconToken.Check, size = 16.dp) },
            )
        }
        actionRow {
            Button(
                text = "取消写入",
                onClick = { ble.cancelWrite() },
                type = ButtonType.OUTLINE,
                iconWidget = { GearIcon(token = IconToken.Close, size = 16.dp) },
            )
        }
    }
    EmNote(
        "通知/指示：先在服务树找到 UUID 0x2902（完整UUID含00002902）的描述符，选择其handle后" +
            "普通写入小端hex：01 00=通知、02 00=指示、00 00=关闭。必须核对特征props支持；" +
            "没有独立subscribe协议。不自动猜测CCCD handle。",
    )
    actionRow {
        Button(text = "填入通知 0100", onClick = { ble.setHex("01 00") }, type = ButtonType.OUTLINE)
        Button(text = "填入指示 0200", onClick = { ble.setHex("02 00") }, type = ButtonType.OUTLINE)
    }
    actionRow {
        Button(text = "填入关闭 0000", onClick = { ble.setHex("00 00") }, type = ButtonType.OUTLINE)
    }
    EmNote("以上只修改草稿，不发送；仍须检查目标与描述符handle并二次确认。")

    // ---- 4. 手动读取通知与断开事件 ----
    fieldLabelRow("4. 手动读取通知与断开事件")
    actionRow {
        Button(
            text = "读取 BLE 状态快照",
            onClick = { ble.readState(pager) },
            type = ButtonType.OUTLINE,
            loading = ble.busy == "ble/state",
            disabled = !(ble.canRead && ble.idle),
            iconWidget = { GearIcon(token = IconToken.Refresh, size = 16.dp) },
        )
    }
    EmNote("GET state只读取服务端最后槽位与最近notify/disconnected有界事件；不订阅手机蓝牙、不自动轮询，也不保证完整事件历史。")
    EmOutputBlock(text = ble.eventsOutput, height = 180.dp)
    if (ble.gattOutput.isNotEmpty()) {
        fieldLabelRow("最近一次 GATT 原始响应（含服务树）")
        EmOutputBlock(text = ble.gattOutput, height = 180.dp)
    }
    if (ble.busy.isNotEmpty()) {
        EmNote(
            "正在等待 ${ble.busy}；GATT最长35秒，客户端45秒超时。修改草稿/目标不会撤销已经发送的" +
                "设备请求，也不能解除当前请求忙态",
        )
    }
    if (ble.error.isNotEmpty()) EmWarn(ble.error, error = true)
    if (ble.result.isNotEmpty()) EmNote(ble.result)
}

/**
 * 一条广播（快照里的候选外设）。
 *
 * 「选择此外设」只改**目标草稿**：不自动连接、不自动读写（红线，旧实现同口径）。
 */
@Composable
private fun EmBleAdvertCard(ble: EchoMuseBleState, advert: BleAdvertRow) {
    LabeledCard(title = advert.address) {
        EmNote(
            "地址类型 ${when (advert.addressType) {
                0 -> "public (0)"
                1 -> "random (1)"
                else -> "未知"
            }}\nRSSI ${advert.rssi}；见于 ${advert.seen}\n原始 data (base64)：${advert.data}",
        )
        actionRow {
            Button(
                text = "选择此外设",
                onClick = { ble.selectAdvert(advert) },
                type = ButtonType.OUTLINE,
                disabled = !(ble.canRead && ble.idle),
                iconWidget = { GearIcon(token = IconToken.Check, size = 16.dp) },
            )
        }
    }
}

/**
 * 一条服务树条目（service / characteristic / descriptor）。
 *
 * 有可读写 handle 的条目给「选择 handle」——只填草稿，读写仍走第 3 段的确认流。
 */
@Composable
private fun EmBleAttributeCard(ble: EchoMuseBleState, item: BleAttributeRow) {
    LabeledCard(title = "${item.kind} · ${item.uuid}") {
        EmNote(item.label)
        if (item.readWriteHandle != null) {
            actionRow {
                Button(
                    text = if (item.kind == "descriptor") "选择描述符 handle" else "选择 value_handle",
                    onClick = { ble.setHandle(item.readWriteHandle.toString()) },
                    type = ButtonType.OUTLINE,
                    iconWidget = { GearIcon(token = IconToken.Check, size = 16.dp) },
                )
            }
        }
    }
}

/**
 * 每设备 BLE 设置（部分更新）。
 *
 * `bleProxyEnabled` / `bleProxyConnections` 就是 Stage 06 头里说的那两个白名单键——
 * 放在 GATT 工具前面是因为它们是代理的**总开关**：不先开启，后面所有操作都会 409。
 * 保存只发这两个键，不覆盖唤醒词等设置。
 */
@Composable
internal fun EchoMuseBleSettings(ble: EchoMuseBleState, state: EchoMuseState) {
    val pager = LocalAdminPagerOrThrow()
    fieldLabelRow("每设备 BLE 设置（部分更新）")
    actionRow {
        Button(
            text = "BLE 扫描：${if (ble.enabledDraft) "开启" else "关闭"}",
            onClick = { ble.toggleEnabled() },
            type = ButtonType.OUTLINE,
        )
        Button(
            text = "BLE 连接：${if (ble.connectionsDraft) "开启" else "关闭"}",
            onClick = { ble.toggleConnections() },
            type = ButtonType.OUTLINE,
        )
    }
    actionRow {
        Button(
            text = "保存 BLE 设置",
            onClick = { ble.saveSettings(pager) { state.refresh(pager) } },
            type = ButtonType.OUTLINE,
            loading = ble.busy == "settings",
            disabled = !(ble.canRead && ble.idle),
            iconWidget = { GearIcon(token = IconToken.Save, size = 16.dp) },
        )
    }
    EmNote(
        "保存只发 bleProxyEnabled/bleProxyConnections，不覆盖唤醒词等设置；不自动开启服务端 " +
            "ble_proxy，也不能证明设备已应用。先开启服务端，再保存并查询槽位。",
    )
    if (ble.settingsResult.isNotEmpty()) EmNote(ble.settingsResult)
}
