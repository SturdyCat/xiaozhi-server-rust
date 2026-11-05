package com.xiaozhi.admin

//! 「插件」管理页状态（P3 末件）：raw 上传桥 + 卸载 + 统一动作的客户端通道。
//!
//! 清单数据复用 [PluginMetaState]（同一次 `GET /api/plugins`，`wasm` 键解析成
//! [PluginSlotSnapshot]），本状态只持有**写通道**与 busy/结果态：
//! - 上传：`pickUploadFile`（平台桥选 .wasm）→ `uploadPickedFile` raw 直传
//!   `POST /api/plugins/upload/raw`（id 缺省 = 服务端从组件 manifest 探测）；
//! - 卸载：`POST /api/plugins/{id}/unload`（注册表 + kv + 产物文件三回收）；
//! - 动作：`POST /api/plugins/{id}/actions/{name}`（白名单在宿主）。
//!
//! 线程契约：回调都落 context 线程（adminTransport / 平台桥既有纪律）；
//! 成功后回调 [reload] 刷新清单（读通道 = [PluginMetaState.load]）。

import com.tencent.kuikly.core.nvi.serialization.json.JSONObject
import com.tencent.kuikly.core.pager.Pager
import com.xiaozhi.admin.adminTransport
import com.xiaozhi.admin.pickUploadFile
import com.xiaozhi.admin.splitAdminUrl
import com.xiaozhi.admin.state.stateVar
import com.xiaozhi.admin.uploadPickedFile

class PluginsManageState(private val meta: PluginMetaState) {
    private var token = ""
    private var base = ""
    private var seq = 0

    /** 上传进行中（按钮置「上传中…」并拦截重复点击）。 */
    var uploading by stateVar(false)

    /** 最近一条结果文案（上传/卸载/动作共用；toast 之外的常驻回显）。 */
    var resultText by stateVar("")

    /** 进行中的动作标记（`pluginId:actionId` 或 `unload:<pluginId>`），空 = 空闲。 */
    var busy by stateVar("")

    fun configureAccess(token: String, base: String) {
        this.token = token
        this.base = base
    }

    /** 选一个 .wasm 组件（平台桥）；id 缺省 = 服务端从组件 manifest 探测。 */
    fun uploadPicked(ctx: Pager, onDone: (ok: Boolean, msg: String) -> Unit) {
        if (uploading) return
        pickUploadFile(ctx, ".wasm") { name, _ ->
            if (name.isEmpty()) {
                onDone(false, "未选择文件（或当前平台不支持本地文件：请用 macOS App / 网页端）")
                return@pickUploadFile
            }
            uploading = true
            resultText = "上传中：$name …"
            val s = ++seq
            uploadPickedFile(ctx, "$base/api/plugins/upload/raw?token=$token") { ok, code, body ->
                if (s != seq) return@uploadPickedFile
                uploading = false
                val msg = messageOf(body, ok, code)
                resultText = msg
                if (ok) meta.load(ctx, base)
                onDone(ok, msg)
            }
        }
    }

    /** 卸载插件（危险动作：页面侧二次确认后才调用）。 */
    fun unload(ctx: Pager, pluginId: String, onDone: (ok: Boolean, msg: String) -> Unit) {
        if (busy.isNotEmpty()) return
        busy = "unload:$pluginId"
        postJson(ctx, "$base/api/plugins/$pluginId/unload", JSONObject()) { ok, _, reason ->
            busy = ""
            val msg = reason.ifEmpty { if (ok) "已卸载 $pluginId" else "卸载失败" }
            resultText = msg
            if (ok) meta.load(ctx, base)
            onDone(ok, msg)
        }
    }

    /** 执行插件声明过的管理动作（payload 固定 `{}`；带参动作随插件面迭代放开）。 */
    fun runAction(
        ctx: Pager,
        pluginId: String,
        actionId: String,
        onDone: (ok: Boolean, msg: String) -> Unit,
    ) {
        if (busy.isNotEmpty()) return
        busy = "$pluginId:$actionId"
        postJson(ctx, "$base/api/plugins/$pluginId/actions/$actionId", JSONObject()) { ok, _, reason ->
            busy = ""
            val msg = reason.ifEmpty { if (ok) "已执行 $actionId" else "动作失败" }
            resultText = msg
            onDone(ok, msg)
        }
    }

    /** JSON POST 公共通道（WS RPC + seq 守卫），口径同 FilesState.postJson。 */
    private fun postJson(
        ctx: Pager,
        url: String,
        body: JSONObject,
        onDone: (ok: Boolean, code: Int?, reason: String) -> Unit,
    ) {
        val s = ++seq
        val (b, path) = splitAdminUrl(url)
        adminTransport.rpc(ctx, b, token, "POST", path, body) { ok, data, status, message ->
            if (s != seq) return@rpc
            val reason = data.text("error").takeIf { it.isNotEmpty() }
                ?: data.text("message").takeIf { it.isNotEmpty() }
                ?: message
            onDone(ok, status.takeIf { it > 0 }, reason)
        }
    }

    private fun messageOf(body: String, ok: Boolean, code: Int): String {
        val obj = runCatching { JSONObject(body) }.getOrNull()
        val err = obj?.optString("error", "") ?: ""
        if (err.isNotEmpty()) return err
        if (ok) {
            val id = obj?.optString("id", "") ?: ""
            return if (id.isNotEmpty()) "已上传并启用：$id" else "上传成功"
        }
        return "上传失败（HTTP ${if (code > 0) code else "网络"}）"
    }
}

