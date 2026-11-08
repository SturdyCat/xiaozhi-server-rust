package com.xiaozhi.admin

//! 「插件」管理页状态（P3 末件）：raw 上传桥 + 生命周期（注册/启用/停用/移除）+ 卸载 + 统一动作的客户端通道。
//!
//! 清单数据复用 [PluginMetaState]（同一次 `GET /api/plugins`，`wasm` 键解析成
//! [PluginSlotSnapshot]），本状态只持有**写通道**与 busy/结果态：
//! - 上传：`pickUploadFile`（平台桥选 .wasm）→ `uploadPickedFile` raw 直传
//!   `POST /api/plugins/upload/raw`（id 缺省 = 服务端从组件 manifest 探测；成功 = staged 待注册）；
//! - 生命周期：`register/activate/deactivate/remove` → `POST /api/plugins/{id}/{op}`
//!   （409 = 依赖/版本冲突，错误文案就地展示）；旧服务端（无 lifecycle 投影）继续走 `unload`；
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
        postJson(ctx, "$base/api/plugins/$pluginId/unload", JSONObject()) { ok, _, _, reason ->
            busy = ""
            val msg = reason.ifEmpty { if (ok) "已卸载 $pluginId" else "卸载失败" }
            resultText = msg
            if (ok) meta.load(ctx, base)
            onDone(ok, msg)
        }
    }

    /** 注册（staged → registered）：清单投影出现，激活后才运行业务。 */
    fun register(ctx: Pager, pluginId: String, onDone: (ok: Boolean, msg: String) -> Unit) =
        lifecyclePost(ctx, "register", "注册", pluginId, "已注册 $pluginId", onDone)

    /** 启用（registered → active）：依赖未满足时服务端 409，文案就地展示。 */
    fun activate(ctx: Pager, pluginId: String, onDone: (ok: Boolean, msg: String) -> Unit) =
        lifecyclePost(ctx, "activate", "启用", pluginId, "已启用 $pluginId", onDone)

    /** 停用（active → registered）：被依赖插件未停用前服务端 409（页面侧也先行禁用按钮）。 */
    fun deactivate(ctx: Pager, pluginId: String, onDone: (ok: Boolean, msg: String) -> Unit) =
        lifecyclePost(ctx, "deactivate", "停用", pluginId, "已停用 $pluginId", onDone)

    /** 移除（仅 staged / registered 非运行插件）：回收注册声明与产物。 */
    fun remove(ctx: Pager, pluginId: String, onDone: (ok: Boolean, msg: String) -> Unit) =
        lifecyclePost(ctx, "remove", "移除", pluginId, "已移除 $pluginId", onDone)

    /** 生命周期端点统一通道（`POST /api/plugins/{id}/{op}`）；成功后刷新清单。 */
    private fun lifecyclePost(
        ctx: Pager,
        op: String,
        opLabel: String,
        pluginId: String,
        okMsg: String,
        onDone: (ok: Boolean, msg: String) -> Unit,
    ) {
        if (busy.isNotEmpty()) return
        busy = "$op:$pluginId"
        postJson(ctx, "$base/api/plugins/$pluginId/$op", JSONObject()) { ok, _, _, reason ->
            busy = ""
            val msg = reason.ifEmpty { if (ok) okMsg else "${opLabel}失败" }
            resultText = msg
            if (ok) meta.load(ctx, base)
            onDone(ok, msg)
        }
    }

    /** 执行插件声明过的管理动作（payload 固定 `{}`；清单页/回退页用，现状不变）。 */
    fun runAction(
        ctx: Pager,
        pluginId: String,
        actionId: String,
        onDone: (ok: Boolean, msg: String) -> Unit,
    ) {
        runAction(ctx, pluginId, actionId, JSONObject()) { ok, msg, _ -> onDone(ok, msg) }
    }

    /**
     * 执行插件声明过的管理动作（payload 由调用方组装：动态 UI 页面含 input 时传该页
     * 全部输入值对象，无输入页传空对象）；[onDone] 的 `rawBody` 是服务端回包原文
     * （动态 UI 的 `result` 节点展示用）。
     */
    fun runAction(
        ctx: Pager,
        pluginId: String,
        actionId: String,
        payload: JSONObject,
        onDone: (ok: Boolean, msg: String, rawBody: String) -> Unit,
    ) {
        if (busy.isNotEmpty()) return
        busy = "$pluginId:$actionId"
        postJson(ctx, "$base/api/plugins/$pluginId/actions/$actionId", payload) { ok, _, data, reason ->
            busy = ""
            val msg = reason.ifEmpty { if (ok) "已执行 $actionId" else "动作失败" }
            resultText = msg
            onDone(ok, msg, data.toString())
        }
    }

    /** JSON POST 公共通道（WS RPC + seq 守卫），口径同 FilesState.postJson。 */
    private fun postJson(
        ctx: Pager,
        url: String,
        body: JSONObject,
        onDone: (ok: Boolean, code: Int?, data: JSONObject, reason: String) -> Unit,
    ) {
        val s = ++seq
        val (b, path) = splitAdminUrl(url)
        adminTransport.rpc(ctx, b, token, "POST", path, body) { ok, data, status, message ->
            if (s != seq) return@rpc
            val reason = data.text("error").takeIf { it.isNotEmpty() }
                ?: data.text("message").takeIf { it.isNotEmpty() }
                ?: message
            onDone(ok, status.takeIf { it > 0 }, data, reason)
        }
    }

    private fun messageOf(body: String, ok: Boolean, code: Int): String {
        val obj = runCatching { JSONObject(body) }.getOrNull()
        val err = obj?.optString("error", "") ?: ""
        if (err.isNotEmpty()) return err
        if (ok) {
            val id = obj?.optString("id", "") ?: ""
            // 生命周期契约：上传成功只进 staged（待注册），不再宣称「已启用」
            return if (id.isNotEmpty()) "已上传，待注册：$id" else "上传成功"
        }
        return "上传失败（HTTP ${if (code > 0) code else "网络"}）"
    }
}

