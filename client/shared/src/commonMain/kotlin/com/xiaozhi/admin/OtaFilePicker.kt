package com.xiaozhi.admin

import com.tencent.kuikly.core.pager.Pager

/**
 * 平台桥：**选一个本地固件包**（`onResult(文件名, 字节数)`；取消 / 不支持 → `("", 0)`）。
 *
 * 为什么必须分平台：浏览器与原生 App 拿"用户选的文件"的途径完全不同，而 Kuikly 的
 * `NetworkModule` 只接受 `ByteArray`（二进制 POST），所以共同点是"先把文件拿到手"。
 * - macOS（`macosArm64Main`）：`XiaoZhiModule.pickFirmwareFile` → 原生 `UIDocumentPicker`，
 *   字节经 Kuikly **原子桥**（`ByteArray` 直通）回传，不经过 JSON 字符串；
 * - web（`jsMain`）：隐藏 `<input type="file">`，`File` 句柄暂存在 JS 侧（见下），
 *   只有文件名/大小回传 Kotlin——几 MB 的字节没必要往返一次；
 * - Android / iOS 宿主（`androidMain` / `iosMain`）：暂未实现，回调 `("", 0)`。
 */
expect fun pickFirmwareFile(ctx: Pager, onResult: (name: String, size: Long) -> Unit)

/**
 * 平台桥：**把上一步选好的固件包 POST 到 [url]**（裸 bin）。
 *
 * - macOS：Kotlin 侧持有字节 → `NetworkModule.httpRequestBinary`（`ByteArray` 直通）；
 * - web：JS 侧持有 `File` → `fetch(url, {method:'POST', body:file})`（浏览器按流发送，
 *   不必先把几 MB 读进 Kotlin 内存）；
 * - Android / iOS：直接回调"当前平台不支持上传"。
 *
 * `onResult(ok, statusCode, body)`：`body` 是服务端响应正文（成功是元数据 JSON、
 * 失败是错误文案），由调用方解析；用户取消选择 → `(false, 0, "")`。
 */
expect fun uploadPickedFirmware(
    ctx: Pager,
    url: String,
    onResult: (ok: Boolean, code: Int, body: String) -> Unit,
)
