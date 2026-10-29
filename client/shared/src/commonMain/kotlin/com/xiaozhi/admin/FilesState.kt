package com.xiaozhi.admin


import com.tencent.kuikly.core.base.PagerScope
import com.tencent.kuikly.core.nvi.serialization.json.JSONObject
import com.tencent.kuikly.core.pager.Pager
import com.xiaozhi.admin.state.stateVar
import com.xiaozhi.admin.state.stateListOf

/**
 * 「文件」页的数据源：服务端存储目录浏览（`GET /api/files`）+ 新建文件夹 / 上传 / 删除
 * （`POST /api/files/{mkdir,upload,delete}`——Kuikly 的 NetworkModule 只有 GET/POST，
 * 服务端因此只提供这组 POST 端点，见 admin AGENTS.md「HTTP API 一览」）。
 *
 * 设计口径与 [OtaState]（同为"只读清单 + 写动作"）一致：
 * - **失败就地显示**：清单加载失败 → [loadError] +「刷新」，不弹 toast；写动作（新建/
 *   上传/删除）结果经 [upload] / [mkdir] / [remove] 的回调走页面顶部 toast；
 * - **鉴权**：GET 把 token 编进查询串（[echoMuseAuthenticatedUrl]），POST 走 Bearer 头
 *   （[echoMuseHeaders]）——与 EchoMuse 端点同一约定；token/base 由 [configureAccess] 传入；
 * - **代际守卫**：所有请求回调按 [seq] 丢弃迟到回包（切换服务器/快速换目录后，旧目录的
 *   清单不会覆盖新目录；迟到的上传结果不会误报）；
 * - **删除二次确认**：任何删除（含目录，服务端递归）都要先 [armRemove] 武装
 *   [confirmDelete]，再点一次才真正发请求；换目录/刷新即复位。
 *
 * ⚠️ 本类不渲染：渲染见 FilesPage.kt，纯逻辑（路径校验/清单解析/格式化）见 [FilesModel]。
 */
class FilesState(private val scope: PagerScope) {

    var loading by stateVar(false)
        private set
    var loaded by stateVar(false)
        private set
    var loadError by stateVar("")
        private set

    /** 服务端存储根目录绝对路径（"文件到底存哪了"直接可见）。 */
    var root by stateVar("")
        private set

    /** 当前相对目录（空串 = 根目录「存储目录」）。 */
    var path by stateVar("")
        private set

    /** 面包屑轨迹（段名 → 截至该段的相对路径），[load] 成功/进入时整表重建。 */
    val crumbs: MutableList<Pair<String, String>> = stateListOf()

    /** 当前目录条目（服务端已按目录优先排序，客户端保持原序）。 */
    val entries: MutableList<FilesEntry> = stateListOf()

    /** 行内删除二次确认：已武装的相对路径（空串 = 无）；换目录/刷新复位。 */
    var confirmDelete by stateVar("")

    /** mkdir/delete 互斥（低频管理操作，无需并发）。 */
    var actionBusy by stateVar(false)
        private set

    /** 「新建文件夹」表单开关与名称草稿。 */
    var mkdirOpen by stateVar(false)
    var mkdirDraft by stateVar("")

    /** 上传进行中（选包 → 二进制 POST 期间拦截重复点击）。 */
    var uploading by stateVar(false)
        private set

    /** 上次使用的服务端基址 / 鉴权 token（[refresh] 与写动作复用）。 */
    private var base = ""
    private var token = ""

    /** 操作序号：迟到的回调按序号丢弃（OtaState 同款 seq 守卫）。 */
    private var seq = 0

    /** token 或 base 变化：清空目录状态并使全部在途回包失效（mac「切换服务器」后同款）。 */
    fun configureAccess(token: String, baseUrl: String = "") {
        if (this.token == token && this.base == baseUrl) return
        seq++
        this.token = token
        this.base = baseUrl
        loading = false
        loaded = false
        loadError = ""
        root = ""
        path = ""
        crumbs.clear()
        entries.clear()
        confirmDelete = ""
        actionBusy = false
        uploading = false
        mkdirOpen = false
        mkdirDraft = ""
    }

    /**
     * 拉取 [rel] 目录的清单（空串 = 根目录）。先就地置 [path]（面包屑即时响应），
     * 成功后以服务端回显为准；失败置 [loadError]。任何进入/刷新都会复位删除确认与新建表单。
     */
    fun load(ctx: Pager, rel: String = path) {
        val target = rel.trim().trim('/')
        if (target.isNotEmpty()) {
            val err = FilesModel.validateRelPath(target)
            if (err != null) {
                loadError = err
                return
            }
        }
        val s = ++seq
        loading = true
        // 刷新已有数据的目录不回到骨架（loaded 保持 true，口径同运维页：刷新反馈由「刷新中…」承担）
        loadError = ""
        path = target
        confirmDelete = ""
        mkdirOpen = false
        mkdirDraft = ""
        crumbs.clear()
        crumbs.addAll(FilesModel.crumbTrail(target))
        getJson(ctx, echoMuseAuthenticatedUrl("$base/api/files?path=${echoMuseUrlComponent(target)}", token)) { ok, data, message ->
            if (s != seq) return@getJson
            loading = false
            if (!ok) {
                // 服务端错误统一 {"error":"…"}：优先透出原因（如 404「目录不存在: oww-models」
                // ——首次进 oww-models/ 还没上传过时的常态），拿不到再退 HTTP 码
                val reason = data.text("error").takeIf { it.isNotEmpty() } ?: message
                loadError = "目录读取失败: ${reason.ifEmpty { "HTTP 错误" }}（可上传文件或新建文件夹，目录会自动创建）"
                return@getJson
            }
            val listing = FilesModel.parseListing(data.toString())
            root = listing.root
            path = listing.path.ifEmpty { target }
            crumbs.clear()
            crumbs.addAll(FilesModel.crumbTrail(path))
            entries.clear()
            entries.addAll(listing.entries)
            loaded = true
            loadError = ""
        }
    }

    /** 进入子目录 / 面包屑跳转（非法路径就地报错，不发请求）。 */
    fun openPath(ctx: Pager, rel: String) {
        val target = rel.trim().trim('/')
        if (target.isNotEmpty()) {
            val err = FilesModel.validateRelPath(target)
            if (err != null) {
                loadError = err
                return
            }
        }
        load(ctx, target)
    }

    /** 返回上一级（已在根目录则刷新根目录）。 */
    fun up(ctx: Pager) = load(ctx, FilesModel.parentOf(path))

    /** 按当前目录重新拉取（「刷新」/ 写动作成功后）。 */
    fun refresh(ctx: Pager) = load(ctx, path)

    // ============================================================
    // 新建文件夹（POST /api/files/mkdir；409 = 已有同名文件占用）
    // ============================================================

    fun mkdir(ctx: Pager, onDone: (ok: Boolean, msg: String) -> Unit) {
        val name = mkdirDraft.trim()
        val err = FilesModel.validateFolderName(name)
        if (err != null) {
            onDone(false, err)
            return
        }
        if (actionBusy) return
        actionBusy = true
        val body = JSONObject().apply { put("path", FilesModel.joinPath(path, name)) }
        postJson(ctx, "$base/api/files/mkdir", body) { ok, code, reason ->
            actionBusy = false
            if (ok) {
                mkdirOpen = false
                mkdirDraft = ""
                onDone(true, "已创建文件夹 $name")
                refresh(ctx)
            } else {
                onDone(false, "创建失败: ${reason.ifEmpty { "HTTP $code" }}")
            }
        }
    }

    // ============================================================
    // 删除（POST /api/files/delete；目录递归；受保护文件服务端 403）
    // ============================================================

    /** 第一次点「删除」：只武装该条目的二次确认（按钮变为「再次点击确认删除」）。 */
    fun armRemove(entry: FilesEntry) {
        confirmDelete = FilesModel.joinPath(path, entry.name)
    }

    /** 第二次点击才真正删除。403（受保护文件）单独给出口径。 */
    fun remove(ctx: Pager, entry: FilesEntry, onDone: (ok: Boolean, msg: String) -> Unit) {
        if (actionBusy) return
        val rel = FilesModel.joinPath(path, entry.name)
        confirmDelete = ""
        actionBusy = true
        val body = JSONObject().apply { put("path", rel) }
        postJson(ctx, "$base/api/files/delete", body) { ok, code, reason ->
            actionBusy = false
            if (ok) {
                onDone(true, "已删除 ${entry.name}")
                refresh(ctx)
            } else {
                onDone(false, when (code) {
                    403 -> "删除被拒绝（受保护文件）: ${reason.ifEmpty { "服务端不允许删除该路径" }}"
                    else -> "删除失败: ${reason.ifEmpty { "HTTP $code" }}"
                })
            }
        }
    }

    // ============================================================
    // 上传（选文件 → POST /api/files/upload?path=<当前目录/文件名>）
    // ============================================================

    /**
     * 选一个本地上传文件（accept 不限）并传到当前目录：
     * - 空文件名 = 用户取消，或平台不支持——借一次 [uploadPickedFile] 的桩回调区分：
     *   不支持平台会带可操作文案（toast error），可上传平台返回空正文（toast 中性提示）；
     * - 409（同名已存在，服务端要求显式 overwrite）等错误**原样透出服务端 error 文案**，
     *   不做静默覆盖、不做花哨交互——需要覆盖时先删旧文件再传；
     * - 成功后刷新清单并 toast；回调 `(msg, level)` 由页面接 [ToastState] 展示。
     */
    fun upload(ctx: Pager, onDone: (msg: String, level: String) -> Unit) {
        if (uploading) return
        val s = ++seq
        pickUploadFile(ctx, accept = "") { name, _ ->
            if (s != seq) return@pickUploadFile
            if (name.isEmpty()) {
                uploadPickedFile(ctx, "") { _, _, stubBody ->
                    if (s != seq) return@uploadPickedFile
                    if (stubBody.isNotEmpty()) onDone(stubBody, "error") else onDone("已取消选择文件", "info")
                }
                return@pickUploadFile
            }
            val err = FilesModel.validateUploadName(name)
            if (err != null) {
                onDone(err, "error")
                return@pickUploadFile
            }
            uploading = true
            val rel = FilesModel.joinPath(path, name)
            val url = echoMuseAuthenticatedUrl(
                "$base/api/files/upload?path=${echoMuseUrlComponent(rel)}",
                token,
            )
            uploadPickedFile(ctx, url) { ok, code, body ->
                if (s != seq) return@uploadPickedFile
                uploading = false
                if (ok && code in 200..299) {
                    onDone("已上传 $rel", "ok")
                    refresh(ctx)
                } else {
                    val fallback = when {
                        code == 409 -> "同名文件已存在（如需覆盖请先删除旧文件）"
                        code == 413 -> "文件过大，服务端拒绝接收"
                        code == 401 || code == 403 -> "HTTP $code（请确认管理令牌）"
                        code > 0 -> "HTTP $code"
                        else -> body.ifEmpty { "网络错误" }
                    }
                    onDone("上传失败: " + FilesModel.errorText(body, fallback), "error")
                }
            }
        }
    }

    /** JSON POST 公共通道（Bearer 鉴权 + seq 守卫），口径同 EchoMuseState.post。 */
    private fun postJson(
        ctx: Pager,
        url: String,
        body: JSONObject,
        onDone: (ok: Boolean, code: Int?, reason: String) -> Unit,
    ) {
        val s = ++seq
        network(ctx).httpRequest(url, true, body, echoMuseHeaders(token)) { data, success, errorMsg, resp ->
            if (s != seq) return@httpRequest
            val code = resp.statusCode
            val ok = success && (code == null || code in 200..299)
            val reason = data.text("error").takeIf { it.isNotEmpty() }
                ?: data.text("message").takeIf { it.isNotEmpty() }
                ?: errorMsg
            onDone(ok, code, reason)
        }
    }
}
