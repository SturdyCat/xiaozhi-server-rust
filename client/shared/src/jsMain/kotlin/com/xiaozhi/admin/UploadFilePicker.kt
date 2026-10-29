package com.xiaozhi.admin

import com.tencent.kuikly.core.pager.Pager

/**
 * web（H5）实现：隐藏 `<input type="file">` 选文件 + `fetch` 直发。
 *
 * 全程用裸 JS（`js("...")`），与 jsMain/OtaFilePicker.kt 同一取舍：只有十来行 DOM/fetch
 * 操作，不值得为它引入 DOM 依赖；`fetch(..., {body: File})` 由浏览器按流发送，几 MB 的
 * 模型文件不必先读进 Kotlin 内存。
 *
 * `File` 句柄挂在 `window.__xzAdminUploadFile`（**独立于 OTA 的 `__xzOtaFile`**：
 * 固件必须是 .bin，这里的文件任意类型，槽位混用会让两边互相覆盖"已选未传"的文件）。
 * [accept] 原样传给 `<input accept>`；空串 = 不限类型。
 */
private val jsPickUploadFile: dynamic = js(
    """(function(accept, cb){
        var input = document.createElement('input');
        input.type = 'file';
        if (accept) { input.accept = accept; }
        input.style.display = 'none';
        document.body.appendChild(input);
        input.addEventListener('change', function(){
            var f = input.files && input.files[0];
            if (document.body.contains(input)) document.body.removeChild(input);
            if (!f) { window.__xzAdminUploadFile = null; cb('', '0'); return; }
            window.__xzAdminUploadFile = f;
            cb(f.name, String(f.size));
        });
        input.click();
    })"""
)

private val jsUploadPickedFile: dynamic = js(
    """(function(url, cb){
        var f = window.__xzAdminUploadFile;
        if (!f) { cb(false, 0, ''); return; }
        fetch(url, {
            method: 'POST',
            headers: {'Content-Type': 'application/octet-stream'},
            body: f
        }).then(function(r){
            return r.text().then(function(t){ cb(r.ok, String(r.status), t); });
        }).catch(function(e){
            cb(false, 0, String(e));
        });
    })"""
)

actual fun pickUploadFile(ctx: Pager, accept: String, onResult: (name: String, size: Long) -> Unit) {
    // JS 侧只回传字符串（名字/大小），避免 number ↔ Kotlin 数值类型的桥接歧义
    jsPickUploadFile(accept) { name: Any?, size: Any? ->
        onResult(name as? String ?: "", (size as? String)?.toLongOrNull() ?: 0L)
    }
}

actual fun uploadPickedFile(
    ctx: Pager,
    url: String,
    onResult: (ok: Boolean, code: Int, body: String) -> Unit,
) {
    jsUploadPickedFile(url) { ok: Any?, code: Any?, body: Any? ->
        onResult(ok == true, (code as? String)?.toIntOrNull() ?: 0, body as? String ?: "")
    }
}
