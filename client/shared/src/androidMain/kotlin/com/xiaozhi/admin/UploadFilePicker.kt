package com.xiaozhi.admin

import com.xiaozhi.admin.ui.*

import com.tencent.kuikly.core.pager.Pager

/**
 * Android 宿主：管理后台只在 **macOS App（Mac Catalyst）** 与 **网页端（h5App）** 提供，
 * 本平台不注册文件选择/上传实现——回调"不支持"并给出可操作出口，而不是静默失败。
 *
 * 之所以仍要一份 actual：[pickUploadFile] 是 `expect`（commonMain 的「文件」页要编译到
 * 所有目标），缺 actual 会直接编译失败。与 androidMain/OtaFilePicker.kt 的固件桩同型。
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
