package com.xiaozhi.admin

import com.xiaozhi.admin.ui.*

import com.tencent.kuikly.core.base.ViewContainer
import com.tencent.kuikly.core.directives.vfor
import com.tencent.kuikly.core.directives.vif
import com.tencent.kuikly.core.pager.Pager
import com.tencent.kuikly.core.views.KRNestedScrollMode
import com.tencent.kuikly.core.views.Scroller

/** 设备卡的「蓝牙代理」菜单面板：通用 GATT 工具，不是设备专属开关协议或实时通知面板。 */
fun ViewContainer<*, *>.echoMuseBlePanel(row: EchoMuseDeviceRow, state: EchoMuseState, ctx: Pager) {
    val ble = state.ble
    fieldLabel("通用 BLE / GATT 管理")
        textNote(text = { ble.status }, micro = true)
        textNote("仅面向你拥有或获授权的 IoT 外设。无配对/加密，需要安全属性可能返回 att:5。通用字节不具备开关等业务语义，先查外设协议。", micro = true)
        bleSettings(ble, state, ctx)
        fieldLabel("1. 广播快照与目标外设")
        actionRow {
            secondaryButton("读取广播快照", enabled = { ble.canRead && ble.idle }, loading = { ble.busy == "ble/adverts" }, icon = IconToken.Refresh) {
                ble.readAdverts(ctx)
            }
        }
        textNote(text = { ble.advertsSummary }, micro = true)
        vif({ ble.adverts.isNotEmpty() }) {
            Scroller {
                attr { height(240f); nestedScroll(KRNestedScrollMode.SELF_FIRST, KRNestedScrollMode.SELF_FIRST) }
                vfor({ ble.adverts }) { advert ->
                    insetPanel {                        textNote(text = { "${advert.address}；地址类型 ${when (advert.addressType) { 0 -> "public (0)"; 1 -> "random (1)"; else -> "未知" }}\nRSSI ${advert.rssi}；见于 ${advert.seen}\n原始 data (base64)：${advert.data}" }, micro = true)
                        actionRow {
                            secondaryButton("选择此外设", enabled = { ble.canRead && ble.idle }, icon = IconToken.Check) { ble.selectAdvert(advert) }
                        }
                    }
                }
            }
        }
        labeledField("目标 MAC 地址", { ble.address }, { ble.applyAddress(it) }, "aa:bb:cc:dd:ee:ff")
        actionRow {
            secondaryButton("地址类型", dynamicText = { if (ble.addressType == 0) "地址：public (0)" else "地址：random (1)" }) {
                ble.applyAddressType(if (ble.addressType == 0) 1 else 0)
            }
        }
        textNote("手填 MAC 时必须核对 public/random 地址类型；地址可能轮换。选择外设不会自动连接，也不会自动执行读写。", micro = true)
        fieldLabel("2. 槽位、连接与服务发现")
        actionRow {
            secondaryButton("查询槽位", enabled = { ble.canGatt && ble.idle }, loading = { ble.busy == "slots" }, icon = IconToken.Refresh) { ble.querySlots(ctx) }
            hGap(AdminSpace.sm)
            secondaryButton("连接外设", enabled = { ble.canGatt && ble.idle }, loading = { ble.busy == "connect" }, icon = IconToken.Bluetooth) { ble.connect(ctx) }
        }
        actionRow {
            secondaryButton("断开目标", enabled = { ble.canGatt && ble.idle }, loading = { ble.busy == "disconnect" }, icon = IconToken.Close) { ble.disconnect(ctx) }
            hGap(AdminSpace.sm)
            secondaryButton("发现服务", enabled = { ble.canGatt && ble.idle }, loading = { ble.busy == "services" }, icon = IconToken.Refresh) { ble.discover(ctx) }
        }
        outputBlock(content = { ble.slotsOutput }, height = 180f)
        vif({ ble.attributes.isNotEmpty() }) {
            Scroller {
                attr { height(320f); nestedScroll(KRNestedScrollMode.SELF_FIRST, KRNestedScrollMode.SELF_FIRST) }
                vfor({ ble.attributes }) { item ->
                    insetPanel {                        textNote(text = { "${item.kind}\nUUID: ${item.uuid}\n${item.label}" }, micro = true)
                        if (item.readWriteHandle != null) {
                            actionRow {
                                secondaryButton(
                                    if (item.kind == "descriptor") "选择描述符 handle" else "选择 value_handle",
                                    icon = IconToken.Check,
                                ) {
                                    ble.setHandle(item.readWriteHandle.toString())
                                }
                            }
                        }
                    }
                }
            }
        }
        fieldLabel("3. 特征 / 描述符读写")
        textNote("输入十进制 handle (1～65535)。特征读写使用 value_handle，不是声明 handle；描述符（如 UUID 0x2902 CCCD）使用描述符自身 handle。", micro = true)
        // Keep raw text: validation must reject, not turn pasted -1/1.2 into a different handle.
        labeledField("十进制 handle", { ble.handleDraft }, { ble.setHandle(it) }, "1～65535")
        actionRow {
            secondaryButton("读取 handle", enabled = { ble.canGatt && ble.idle }, loading = { ble.busy == "read" }, icon = IconToken.Refresh) { ble.read(ctx) }
        }
        vif({ ble.readOutput.isNotEmpty() }) { outputBlock(content = { ble.readOutput }, height = 180f) }
        labeledTextArea("写入 hex（非空，最多512字节）", { ble.hexDraft }, { ble.setHex(it) }, "01 00", height = 100f)
        textNote(text = {
            val bytes = bleHexToBytes(ble.hexDraft)
            if (bytes == null) "仅接受完整十六进制字节与空格/换行；不接受0x前缀，拒绝奇数位、非法字符及超过512字节"
            else "将发送 ${bytes.size} 字节；base64: ${bleHexToBase64(ble.hexDraft)}"
        }, micro = true)
        actionRow {
            secondaryButton("ATT写入模式", dynamicText = { if (ble.responseDraft) "有响应写 (response=true)" else "无响应写 (response=false)" }) {
                ble.toggleResponse()
            }
        }
        textNote("有响应成功仅代表 ATT 接受，不证明 IoT 业务动作完成；无响应写只表示设备发送，不代表外设接受。请读回或核对外设状态，避免盲目重试。", micro = true)
        actionRow {
            secondaryButton("检查并准备写入", enabled = { ble.canGatt && ble.idle }, icon = IconToken.Check) { ble.prepareWrite() }
        }
        vif({ ble.confirmed != null }) {
            textNote(text = {
                val intent = ble.confirmed
                "写入确认：桥接设备 ${intent?.deviceId}\nsession_id=${intent?.sessionId}\n外设 ${intent?.address} / addr_type=${intent?.addressType}\nhandle=${intent?.handle}；response=${intent?.response}\nhex=${intent?.valueBase64?.let(::bleBase64ToHex)}\n确认会改变外设状态；任何草稿、目标或会话变化都须重新确认。"
            }, micro = true)
            actionRow {
                primaryButton("确认写入上述字节", enabled = { ble.canGatt && ble.idle }, danger = { true }, icon = IconToken.Check) { ble.confirmWrite(ctx) }
            }
            actionRow { secondaryButton("取消写入", icon = IconToken.Close) { ble.cancelWrite() } }
        }
        textNote("通知/指示：先在服务树找到 UUID 0x2902（完整UUID含00002902）的描述符，选择其handle后普通写入小端hex：01 00=通知、02 00=指示、00 00=关闭。必须核对特征props支持；没有独立subscribe协议。不自动猜测CCCD handle。", micro = true)
        actionRow {
            secondaryButton("填入通知 0100") { ble.setHex("01 00") }
            hGap(AdminSpace.sm)
            secondaryButton("填入指示 0200") { ble.setHex("02 00") }
        }
        actionRow { secondaryButton("填入关闭 0000") { ble.setHex("00 00") } }
        textNote("以上只修改草稿，不发送；仍须检查目标与描述符handle并二次确认。", micro = true)
        fieldLabel("4. 手动读取通知与断开事件")
        actionRow {
            secondaryButton("读取 BLE 状态快照", enabled = { ble.canRead && ble.idle }, loading = { ble.busy == "ble/state" }, icon = IconToken.Refresh) { ble.readState(ctx) }
        }
        textNote("GET state只读取服务端最后槽位与最近notify/disconnected有界事件；不订阅手机蓝牙、不自动轮询，也不保证完整事件历史。", micro = true)
        outputBlock(content = { ble.eventsOutput }, height = 180f)
        vif({ ble.gattOutput.isNotEmpty() }) {
            fieldLabel("最近一次 GATT 原始响应（含服务树）")
            outputBlock(content = { ble.gattOutput }, height = 180f)
        }
        vif({ ble.busy.isNotEmpty() }) { textNote(text = { "正在等待 ${ble.busy}；GATT最长35秒，客户端45秒超时。修改草稿/目标不会撤销已经发送的设备请求，也不能解除当前请求忙态" }, micro = true) }
        vif({ ble.error.isNotEmpty() }) { textNote({ ble.error }, { "error" }, micro = true) }
        vif({ ble.result.isNotEmpty() }) { textNote(text = { ble.result }, micro = true) }
}

private fun ViewContainer<*, *>.bleSettings(ble: EchoMuseBleState, state: EchoMuseState, ctx: Pager) {
    fieldLabel("每设备 BLE 设置（部分更新）")
    actionRow {
        secondaryButton("扫描开关", dynamicText = { "BLE 扫描：${if (ble.enabledDraft) "开启" else "关闭"}" }) { ble.toggleEnabled() }
        hGap(AdminSpace.sm)
        secondaryButton("连接开关", dynamicText = { "BLE 连接：${if (ble.connectionsDraft) "开启" else "关闭"}" }) { ble.toggleConnections() }
    }
    actionRow {
        secondaryButton("保存 BLE 设置", enabled = { ble.canRead && ble.idle }, loading = { ble.busy == "settings" }, icon = IconToken.Save) {
            ble.saveSettings(ctx) { state.refresh(ctx) }
        }
    }
    textNote("保存只发 bleProxyEnabled/bleProxyConnections，不覆盖唤醒词等设置；不自动开启服务端 ble_proxy，也不能证明设备已应用。先开启服务端，再保存并查询槽位。", micro = true)
    vif({ ble.settingsResult.isNotEmpty() }) { textNote(text = { ble.settingsResult }, micro = true) }
}

/** All dynamic diagnostic values are read inside attr closures, never passed as static textNote. */
