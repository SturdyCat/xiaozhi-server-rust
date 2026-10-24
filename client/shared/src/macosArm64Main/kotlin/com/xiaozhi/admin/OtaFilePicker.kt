package com.xiaozhi.admin

import com.xiaozhi.admin.ui.*

import com.tencent.kuikly.core.module.NetworkModule
import com.tencent.kuikly.core.nvi.serialization.json.JSONObject
import com.tencent.kuikly.core.pager.Pager

/**
 * macOS（Mac Catalyst）实现：原生 `UIDocumentPicker` 选 `.bin`，字节经 Kuikly 原子桥回传。
 *
 * 选好的字节先存在 Kotlin 侧（几 MB，上传完即释放）：这样上传复用 `NetworkModule` 的
 * 二进制通道，鉴权/超时/错误处理与其它管理请求完全一致，不必在原生侧再写一套 HTTP。
 */
private var pickedBytes: ByteArray? = null

actual fun pickFirmwareFile(ctx: Pager, onResult: (name: String, size: Long) -> Unit) {
    ctx.acquireModule<XiaoZhiModule>(XiaoZhiModule.MODULE_NAME).pickFirmwareFile { name, bytes ->
        if (name.isEmpty() || bytes == null || bytes.isEmpty()) {
            pickedBytes = null
            onResult("", 0L)
        } else {
            pickedBytes = bytes
            onResult(name, bytes.size.toLong())
        }
    }
}

actual fun uploadPickedFirmware(
    ctx: Pager,
    url: String,
    onResult: (ok: Boolean, code: Int, body: String) -> Unit,
) {
    val bytes = pickedBytes
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
        // 几 MB 走局域网/公网：默认 30s 偏紧，放宽到 180s
        timeout = 180,
    ) { data, success, errorMsg, resp ->
        val code = resp.statusCode ?: 0
        val body = data.decodeToString()
        if (success && code in 200..299) pickedBytes = null
        onResult(success && code in 200..299, code, body.ifEmpty { errorMsg })
    }
}
