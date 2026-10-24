package com.xiaozhi.admin

import com.xiaozhi.admin.ui.*

import com.tencent.kuikly.core.pager.Pager

/**
 * web（H5）实现：隐藏 `<input type="file">` 选包 + `fetch` 直发。
 *
 * 全程用裸 JS（`js("...")`）：只有十来行 DOM/fetch 操作，不值得为它给 shared 的 js 源集
 * 引入 DOM/typed-array 依赖（`kotlinx-browser`）；而且 `fetch(..., {body: File})` 由浏览器
 * 按流发送，几 MB 的包**不必先读进 Kotlin 内存**（`FileReader` → `ByteArray` 那条路要走两次拷贝）。
 *
 * `File` 句柄挂在 `window.__xzOtaFile`（单页管理界面，同一时刻只有一个待上传包）。
 */
private val jsPickFile: dynamic = js(
    """(function(cb){
        var input = document.createElement('input');
        input.type = 'file';
        input.accept = '.bin,application/octet-stream';
        input.style.display = 'none';
        document.body.appendChild(input);
        input.addEventListener('change', function(){
            var f = input.files && input.files[0];
            if (document.body.contains(input)) document.body.removeChild(input);
            if (!f) { window.__xzOtaFile = null; cb('', '0'); return; }
            window.__xzOtaFile = f;
            cb(f.name, String(f.size));
        });
        input.click();
    })"""
)

private val jsUploadFile: dynamic = js(
    """(function(url, cb){
        var f = window.__xzOtaFile;
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

actual fun pickFirmwareFile(ctx: Pager, onResult: (name: String, size: Long) -> Unit) {
    // JS 侧只回传字符串（名字/大小），避免 number ↔ Kotlin 数值类型的桥接歧义
    jsPickFile { name: Any?, size: Any? ->
        onResult(name as? String ?: "", (size as? String)?.toLongOrNull() ?: 0L)
    }
}

actual fun uploadPickedFirmware(
    ctx: Pager,
    url: String,
    onResult: (ok: Boolean, code: Int, body: String) -> Unit,
) {
    jsUploadFile(url) { ok: Any?, code: Any?, body: Any? ->
        onResult(ok == true, (code as? String)?.toIntOrNull() ?: 0, body as? String ?: "")
    }
}
