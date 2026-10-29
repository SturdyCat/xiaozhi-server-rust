package com.xiaozhi.admin

import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember

/**
 * 「固件与 OTA」section（`ota`）—— 配置页里的**固件维护面入口**。
 *
 * ## 历史与现状（T08 → T11）
 *
 * T08 时这里只有一张只读入口卡（摘要 + 刷新），把上传/删除指向"固件维护面"，
 * 理由写在旧文件头：不能有两份上传逻辑 —— 上传是唯一能把设备刷成砖的动作。
 *
 * T11 把那份唯一实现（`OtaCard.kt` 的 DSL `otaCard`）迁成了 Compose
 * [OtaFirmwareCard]，于是本文件从"指路的入口卡"变成"真正的挂载点"：
 * 仍然**只有一份**上传链路，只是它现在就在这一页里。`ConfigSectionHost` 的
 * `"ota" -> OtaSection(ota)` 一行未改。
 *
 * ## 为什么本文件仍保留一张"前言卡"
 *
 * 「固件托管与 `/api/config` 完全无关、保存配置不会动固件」是最常被误解的一点，
 * 且它是**这一页独有**的信息（不属于 [OtaFirmwareCard] 的通用能力），所以留在挂载点。
 *
 * ## 关于 `ToastState`
 *
 * `OtaSection(ota)` 的签名不能动（`ConfigCards.kt` 的分发只传 `ota`，而那个文件属 T08
 * 的地盘）。`ToastState` 本身是**无状态转发层**（构造参数 `scope` 保留但未使用，
 * 只把消息转发给 GearUI 的全局 `Toast` 单例，见 `ToastHost.kt`），
 * 因此在这里构造一个与别处行为完全一致，且不会引入第二套提示通道。
 */
@Composable
fun OtaSection(ota: OtaState) {
    val pager = LocalAdminPagerOrThrow()
    // ToastState 无实例状态（纯转发 GearUI 全局 Toast），remember 一份即可
    val toast = remember(pager) { ToastState(pager) }

    LabeledCard(title = "固件与 OTA") {
        HintLine(
            "固件包托管走独立接口（`GET /api/ota/firmware`），不随 `POST /api/config` 回传：" +
                "保存配置不会动固件，上传/删除固件也不会动配置。",
        )
        HintLine(
            "上传与删除就在本页下方（唯一一套链路）：设备 OTA 时按「功能 + 型号」匹配，发错型号的包" +
                "会直接变砖，所以删除必须二次确认。",
        )
    }

    // 唯一一份固件维护实现（列表 / 上传 / 删除），本文件不重复实现任何一步
    OtaFirmwareCard(ota = ota, toast = toast)
}
