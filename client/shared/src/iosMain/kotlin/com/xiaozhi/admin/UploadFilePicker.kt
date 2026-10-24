package com.xiaozhi.admin

import com.xiaozhi.admin.ui.*

import com.tencent.kuikly.core.pager.Pager

/**
 * iOS 宿主：与 Android 同理——管理后台只在 **macOS App（Mac Catalyst）** 与 **网页端** 提供，
 * iOS 端暂未接入 `UIDocumentPicker`（接入时把 [pickUploadFile] 指向原生选择器 +
 * `NetworkModule.httpRequestBinary` 即可，commonMain 的「文件」页无需改动）。
 */
actual fun pickUploadFile(ctx: Pager, accept: String, onResult: (name: String, size: Long) -> Unit) {
    onResult("", 0L)
}

actual fun uploadPickedFile(
    ctx: Pager,
    url: String,
    onResult: (ok: Boolean, code: Int, body: String) -> Unit,
) {
    onResult(false, 0, "当前平台不支持上传（请在 macOS App 或网页端操作）")
}
