package com.xiaozhi.admin

import com.tencent.kuikly.core.pager.Pager

/**
 * 平台桥：**选一个待上传文件**（管理台「文件」页；[accept] 传给文件选择器的类型过滤，
 * 空串 = 不限类型）。
 *
 * 与 OTA 固件选包（[pickFirmwareFile]）同型但**完全独立**：固件必须是 .bin 裸镜像，
 * 而本桥面向任意普通文件（含 EchoMuse 的 .onnx 唤醒词模型），因此 web 端用独立的
 * window 槽位（`window.__xzAdminUploadFile`，不与 `__xzOtaFile` 串包）、mac 端持独立的
 * 字节缓存——两侧互不覆盖对方"已选未传"的文件。
 *
 * `onResult(文件名, 字节数)`；用户取消 / 平台不支持 → `("", 0)`。
 * 为什么必须分平台：与 OTA 选包同一理由（见 OtaFilePicker.kt 文件头）——
 * 浏览器与原生 App 拿"用户选的文件"的途径完全不同，而 Kuikly 的 `NetworkModule`
 * 只接受 `ByteArray`（二进制 POST），共同点是"先把文件拿到手"。
 */
expect fun pickUploadFile(ctx: Pager, accept: String, onResult: (name: String, size: Long) -> Unit)

/**
 * 平台桥：**把上一步选好的文件 POST 到 [url]**（raw octet-stream；鉴权 token 已由调用方
 * 编进 [url] 查询串，与服务端 `POST /api/files/upload` 契约一致）。
 *
 * - web（jsMain）：JS 侧持有 `File` → `fetch(url, {body: file})` 按流发送；
 * - macOS（macosArm64Main）：Kotlin 侧持有字节 → `NetworkModule.httpRequestBinary`；
 * - Android / iOS 宿主：直接回调"当前平台不支持上传"。
 *
 * `onResult(ok, statusCode, body)`：`body` 是服务端响应**原文**（成功 JSON /
 * 失败错误 JSON，错误文案提取走 [FilesModel.errorText]）；未选文件 → `(false, 0, "")`。
 */
expect fun uploadPickedFile(
    ctx: Pager,
    url: String,
    onResult: (ok: Boolean, code: Int, body: String) -> Unit,
)
