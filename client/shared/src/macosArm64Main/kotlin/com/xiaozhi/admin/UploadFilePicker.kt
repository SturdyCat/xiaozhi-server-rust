package com.xiaozhi.admin

import com.xiaozhi.admin.ui.*

import com.tencent.kuikly.core.module.NetworkModule
import com.tencent.kuikly.core.nvi.serialization.json.JSONObject
import com.tencent.kuikly.core.pager.Pager

/**
 * macOS（Mac Catalyst）实现：复用固件选择器的原生桥（`XiaoZhiModule.pickFirmwareFile`，
 * UTTypeData 通用文档选择器，不限 .bin），字节经 Kuikly 原子桥回传。
 *
 * 选好的字节存在 Kotlin 侧**独立的** [pickedUploadBytes]（与 OTA 的 pickedBytes 分开持有，
 * 互不覆盖）；上传复用 `NetworkModule` 的二进制通道，鉴权/超时/错误处理与其它管理请求一致。
 * ObjC 侧不允许也不需要改动：通用文档选择器本就不限类型，commonMain 的 accept 参数在
 * mac 端没有对应过滤位（选择器交给用户自选）。
 */
private var pickedUploadBytes: ByteArray? = null

actual fun pickUploadFile(ctx: Pager, accept: String, onResult: (name: String, size: Long) -> Unit) {
    ctx.acquireModule<XiaoZhiModule>(XiaoZhiModule.MODULE_NAME).pickFirmwareFile { name, bytes ->
        if (name.isEmpty() || bytes == null || bytes.isEmpty()) {
            pickedUploadBytes = null
            onResult("", 0L)
        } else {
            pickedUploadBytes = bytes
            onResult(name, bytes.size.toLong())
        }
    }
}

actual fun uploadPickedFile(
    ctx: Pager,
    url: String,
    onResult: (ok: Boolean, code: Int, body: String) -> Unit,
) {
    val bytes = pickedUploadBytes
    if (bytes == null) {
        onResult(false, 0, "")
        return
    }
    val headers = JSONObject().apply { put("Content-Type", "application/octet-stream") }
    ctx.acquireModule<NetworkModule>(NetworkModule.MODULE_NAME).httpRequestBinary(
        url = url,
        isPost = true,
        bytes = bytes,
        headers = headers,
        // 几 MB 模型走局域网/公网：默认 30s 偏紧，放宽到 180s（同 OTA 上传口径）
        timeout = 180,
    ) { data, success, errorMsg, resp ->
        val code = resp.statusCode ?: 0
        val body = data.decodeToString()
        if (success && code in 200..299) pickedUploadBytes = null
        onResult(success && code in 200..299, code, body.ifEmpty { errorMsg })
    }
}
