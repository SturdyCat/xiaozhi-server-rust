package com.xiaozhi.admin

import com.tencent.kuikly.core.base.PagerScope
import com.tencent.kuikly.core.nvi.serialization.json.JSONObject
import com.tencent.kuikly.core.pager.Pager
import com.tencent.kuikly.core.reactive.collection.ObservableList
import com.tencent.kuikly.core.reactive.handler.observable
import com.tencent.kuikly.core.reactive.handler.observableList
import com.tencent.kuikly.core.timer.setTimeout

/**
 * 「OTA 固件」页的数据源：`GET /api/ota/firmware`（清单）+ 上传 / 删除（管理动作）。
 *
 * 回答的是"这个服务端现在托管了哪些固件、哪台设备会拿到哪一个"：
 * - 服务端按 **功能(project) + 型号(model)** 二维托管，可选 `board` 做更细粒度覆盖；
 * - 设备 OTA 时上报 `application.name` / `chip_model_name` / `board.type`，**同功能同型号**
 *   且版本更新才下发升级地址——不同型号/不同功能的包绝不互发（发错包 = 变砖）。
 *
 * ## 型号/版本不用手填
 * ESP-IDF app 镜像自带元数据（`esp_app_desc_t` + 镜像头 `chip_id`），**服务端上传时自动解析**
 * 出 功能/型号/版本/IDF 版本/编译时间。因此表单里那几个字段是"可选覆盖"（留空即自动识别），
 * 只有非 ESP-IDF 镜像或字段为空时才需要手填。
 *
 * ## 上传 = 选包（平台桥）+ 二进制 POST
 * 固件是 1.5~4MB 的裸 bin：走 `NetworkModule` 的**二进制通道**（`httpRequestBinary`，
 * `ByteArray` 直通，不做 JSON/base64 包装）；web 端由平台桥用 `fetch` 直发 `File`。
 * 选包见 [pickFirmwareFile] / [uploadPickedFirmware]（macOS 原生选择器 / web `<input type=file>`）。
 *
 * 拆成独立文件的两条理由同 [McpToolsState]：`AGENTS.md` §5.10 文件规模约定；
 * 它是**可写动作 + 只读清单**，与配置表单混在一起会让"保存配置会不会动到固件"难以判断
 *（答案：不会——固件托管不在 `/api/config` 里）。
 */
class OtaState(private val scope: PagerScope) {

    /** 已托管条目（服务端已按上传时间倒序）。 */
    val entries: ObservableList<OtaEntryRow> by scope.observableList()

    /** 托管目录（服务端给的绝对路径，"固件到底存哪了"直接可见）。 */
    var dir by scope.observable("")

    /** 计数摘要（未加载时为空串）。 */
    var summary by scope.observable("")

    /** 加载失败的就地错误文案（首次加载失败 → 就地错误 + 「重试」，不弹 toast）。 */
    var loadError by scope.observable("")

    var loading by scope.observable(false)
    var loaded by scope.observable(false)

    // ===== 上传表单（四个字段都是"可选覆盖"：留空 = 用包内元数据）=====
    /** 是否展开"手动指定"覆盖字段（默认收起：ESP-IDF 包根本不需要手填）。 */
    var manual by scope.observable(false)
    var project by scope.observable("")
    var model by scope.observable("")
    var board by scope.observable("")
    var version by scope.observable("")

    /** 已选固件包的文件名（空 = 还没选）。 */
    var pickedName by scope.observable("")

    /** 已选固件包大小（字节；按钮旁显示"已选 xxx.bin（2.1 MB）"）。 */
    var pickedSize by scope.observable(0L)

    /** 选包失败 / 平台不支持时的就地提示。 */
    var pickError by scope.observable("")

    var uploading by scope.observable(false)

    /** 三态按钮的完成态："" 正常 / "ok" 成功 / "error" 失败（约 2s 后由本类复位）。 */
    var uploadResult by scope.observable("")

    /** 正在等待确认删除的条目 key（空 = 无；行内二次确认，避免误删固件）。 */
    var confirmKey by scope.observable("")

    /** 平台侧是否已持有选好的文件（macOS 是字节、web 是 File 句柄）。 */
    private var pickedReady = false

    /** 上次使用的服务端基址 / 鉴权 token：`refresh` / 上传 / 删除复用。 */
    private var base = ""
    private var token = ""

    /** 操作序号：迟到的回调按序号丢弃（防"切换服务器/重新上传后被拽回旧结果"）。 */
    private var seq = 0

    /** 鉴权查询串（token 为空 → 空串；服务端 expected_token 为空时不校验）。
     *  `first` = 该 URL 还没有任何查询参数（决定分隔符用 `?` 还是 `&`）。 */
    private fun authQuery(first: Boolean): String =
        if (token.isEmpty()) "" else (if (first) "?token=" else "&token=") + token

    /** 是否已选好包（三态按钮的可点条件；读 observable 字段，闭包内调用才响应式）。 */
    val hasPicked: Boolean get() = pickedName.isNotEmpty()

    // ============================================================
    // 读取
    // ============================================================

    /** 拉取托管清单。失败只置 `loadError`，不影响配置表单本身。 */
    fun load(ctx: Pager, baseUrl: String = "", authToken: String = "") {
        if (loading) return
        base = baseUrl
        token = authToken
        loading = true
        loadError = ""
        network(ctx).requestGet("${baseUrl}/api/ota/firmware${authQuery(true)}", JSONObject()) { data, success, errorMsg, resp ->
            loading = false
            val code = resp.statusCode
            if (!success || (code != null && code !in 200..299)) {
                loadError = "固件清单加载失败: ${if (errorMsg.isNotEmpty()) errorMsg else "HTTP $code"}"
                return@requestGet
            }
            apply(data)
        }
    }

    /** 按上次的基址重新拉取（页内「刷新」按钮 / 上传、删除成功后）。 */
    fun refresh(ctx: Pager) = load(ctx, base, token)

    private fun apply(data: JSONObject) {
        entries.clear()
        dir = data.optString("dir", "")
        val arr = data.optJSONArray("entries")
        if (arr == null) {
            loadError = "固件清单为空（服务端返回异常）"
            return
        }
        for (i in 0 until arr.length()) {
            val e = arr.optJSONObject(i) ?: continue
            entries.add(
                OtaEntryRow(
                    project = e.optString("project", ""),
                    model = e.optString("model", ""),
                    board = e.optString("board", ""),
                    version = e.optString("version", ""),
                    size = e.optLong("size") ?: 0L,
                    sha256 = e.optString("sha256", ""),
                    idfVer = e.optString("idf_ver", ""),
                    builtAt = e.optString("built_at", ""),
                    legacy = e.optBoolean("legacy", false),
                ),
            )
        }
        summary = if (entries.isEmpty()) {
            "尚未托管任何固件"
        } else {
            "共 ${entries.size} 个固件 · ${entries.map { it.model }.distinct().size} 种型号 · " +
                "${entries.map { it.project }.distinct().size} 个功能"
        }
        loaded = true
        loadError = ""
    }

    // ============================================================
    // 选包 / 上传 / 删除
    // ============================================================

    /** 选一个本地固件包（平台桥：macOS 原生选择器 / web 文件输入）。 */
    fun pick(ctx: Pager) {
        pickError = ""
        pickFirmwareFile(ctx) { name, size ->
            if (name.isEmpty()) {
                pickedReady = false
                pickedName = ""
                pickedSize = 0L
                pickError = "未选择文件（或当前平台不支持选择本地文件：请用 macOS App / 网页端上传）"
            } else {
                pickedReady = true
                pickedName = name
                pickedSize = size
                uploadResult = ""
                pickError = ""
            }
        }
    }

    /**
     * 上传已选固件（`POST /api/ota/firmware`，body = 裸 bin）：
     * 进入时 [uploading]=true → 按钮「上传中…」并拦截重复点击（三态第一态）；
     * 完成后 [uploadResult] 置 "ok"/"error"（约 2s 复位），[onDone] 在**接口返回之后**回调
     *（ok, msg）由页面弹顶部 toast——不能在调用处紧随其后取状态：那时请求还没回来。
     */
    fun upload(ctx: Pager, onDone: (ok: Boolean, msg: String) -> Unit) {
        if (!pickedReady || uploading) {
            onDone(false, "请先选择固件包（.bin）")
            return
        }
        uploading = true
        uploadResult = ""
        val s = ++seq
        // 四个覆盖字段留空即不发：服务端从包内元数据解析（型号/版本自动识别）
        val params = buildList {
            if (project.isNotEmpty()) add("project=$project")
            if (model.isNotEmpty()) add("model=$model")
            if (board.isNotEmpty()) add("board=$board")
            if (version.isNotEmpty()) add("version=$version")
        }
        val query = if (params.isEmpty()) authQuery(true) else "?${params.joinToString("&")}${authQuery(false)}"
        uploadPickedFirmware(ctx, "${base}/api/ota/firmware$query") { ok, code, body ->
            if (s != seq) return@uploadPickedFirmware // 已被后一次操作取代，丢弃迟到回调
            uploading = false
            val json = runCatching { JSONObject(body) }.getOrNull()
            val serverMsg = json?.optString("message", "") ?: ""
            if (ok && code in 200..299) {
                val p = json?.optString("project", "") ?: ""
                val m = json?.optString("model", "") ?: ""
                val v = json?.optString("version", "") ?: ""
                val fromPkg = json?.optString("metadata_from", "") == "package"
                val msg = buildString {
                    append("已上传 $p · $m · $v")
                    append(if (fromPkg) "（型号/版本由包内元数据自动识别）" else "（型号/版本来自手填）")
                }
                uploadResult = "ok"
                pickedReady = false
                pickedName = ""
                pickedSize = 0L
                onDone(true, msg)
                refresh(ctx)
            } else if (!ok && code == 0 && body.isEmpty()) {
                // 用户取消了文件选择：不是错误，中性提示
                uploadResult = ""
                onDone(false, "已取消选择固件包")
            } else if (code == 401 || code == 403) {
                // 服务端配了 expected_token：macApp 连接时已自动带 token；网页端需用 ?token= 打开
                uploadResult = "error"
                onDone(
                    false,
                    "上传被拒绝（401）：服务端配置了 server.expected_token。" +
                        "网页端请用 http://<host>/?token=<token>&page_name=config 打开本页；" +
                        "macApp 请确认「连接服务器」读到的配置里带 token",
                )
            } else {
                val msg = "上传失败: " + serverMsg.ifEmpty {
                    if (body.isNotEmpty()) body.take(200) else "HTTP $code"
                }
                uploadResult = "error"
                onDone(false, msg)
            }
            val rs = seq
            scope.setTimeout(UPLOAD_RESULT_HOLD_MS) { if (rs == seq) uploadResult = "" }
        }
    }

    /** 删除一个托管条目（行内二次确认后调用）。 */
    fun remove(ctx: Pager, row: OtaEntryRow, onDone: (ok: Boolean, msg: String) -> Unit) {
        confirmKey = ""
        val q = "?project=${row.project}&model=${row.model}&board=${row.board}${authQuery(false)}"
        // 客户端 NetworkModule 只暴露 GET/POST，故删除走服务端的 POST 入口
        //（服务端同时保留 DELETE /api/ota/firmware 供 curl 使用）
        network(ctx).requestPost("${base}/api/ota/firmware/delete$q", JSONObject()) { data, success, errorMsg, resp ->
            val code = resp.statusCode
            if (success && (code == null || code in 200..299)) {
                onDone(true, "已删除 ${row.project} · ${row.model} · ${row.version}")
                refresh(ctx)
            } else {
                val serverMsg = data?.optString("message", "") ?: ""
                onDone(false, "删除失败: " + serverMsg.ifEmpty {
                    if (errorMsg.isNotEmpty()) errorMsg else "HTTP $code"
                })
            }
        }
    }

    private companion object {
        /** 上传完成态停留时长（毫秒），到点自动回落到正常态。 */
        const val UPLOAD_RESULT_HOLD_MS = 2000
    }
}

/**
 * 一个托管固件的展示快照（字段都是**不可变快照**——刷新时整表重建，不做增量更新）。
 */
class OtaEntryRow(
    /** 功能名（固件 `project_name`，如 `xiaozhi`）。 */
    val project: String,
    /** 型号（`esp32c3`/`esp32s3`…）。 */
    val model: String,
    /** 板型；空 = 该功能+型号通用。 */
    val board: String,
    val version: String,
    val size: Long,
    val sha256: String,
    val idfVer: String,
    /** 固件编译时刻（包内 `date time`）。 */
    val builtAt: String,
    /** 旧版单固件（无型号信息，不参与匹配，建议删除后按 功能+型号 重新上传）。 */
    val legacy: Boolean,
) {
    /** 行 key（删除确认用；与后端寻址一致：功能+型号+板型）。 */
    val key: String get() = "$project|$model|$board"

    /** 标题：功能 · 型号 · 板型（旧版条目单列说明）。 */
    val title: String
        get() = if (legacy) {
            "旧版单固件（不参与升级）"
        } else {
            "$project · $model · " + if (board.isEmpty()) "该型号通用" else board
        }

    /** 副标题：版本 · 体积 · 编译时间 · IDF · sha256 前 12 位（缺失项自动省略）。 */
    val subtitle: String
        get() = listOfNotNull(
            "版本 $version",
            formatBytes(size),
            builtAt.takeIf { it.isNotEmpty() }?.let { "编译于 $it" },
            idfVer.takeIf { it.isNotEmpty() }?.let { "IDF $it" },
            sha256.takeIf { it.isNotEmpty() }?.let { "sha256 ${it.take(12)}" },
        ).joinToString(" · ")

    /** 会拿到这个包的设备（把匹配规则写清楚，避免管理员靠猜）。 */
    val matchNote: String
        get() = when {
            legacy -> "无型号信息：任何设备都不会拿到它（请按 功能+型号 重新上传）"
            board.isNotEmpty() -> "只发给 功能=$project、型号=$model、板型=$board 的设备"
            else -> "发给 功能=$project、型号=$model 的设备（该型号通用；上传板型专属包可进一步收窄）"
        }
}

/** 字节 → 人话（2.1 MB / 512 KB / 320 B）；MB 保留一位小数。 */
fun formatBytes(bytes: Long): String = when {
    bytes >= 1024L * 1024L -> "${(bytes * 10L / (1024L * 1024L)) / 10.0} MB"
    bytes >= 1024L -> "${bytes / 1024L} KB"
    else -> "$bytes B"
}
