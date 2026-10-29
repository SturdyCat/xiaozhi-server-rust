package com.xiaozhi.admin

import kotlinx.datetime.Instant
import kotlinx.datetime.TimeZone
import kotlinx.datetime.toLocalDateTime
import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.decodeFromJsonElement
import kotlinx.serialization.json.jsonObject

/**
 * 「文件管理」页的**纯逻辑**（服务端存储目录浏览 / 上传 / 删除的客户端镜像规则）。
 *
 * 本文件不 import Kuikly/平台 API（同 [EchoMuseWakeWord] 的口径），只用 commonMain 的两件
 * 通用库：kotlinx-datetime 做公历换算、kotlinx-serialization 解回包（此前是一份手写的
 * 140 行 JSON 解析器，见文件尾的说明）。由普通 JVM 测试模块 `:adminCoreTests` 直接引用
 * 同一份生产源码回归（路径校验、清单解析、尺寸/时间格式化都是页面上"看起来对不对"的最后
 * 防线，必须可测）。
 *
 * 服务端契约（FROZEN，server 侧同批实现）：
 * - `GET /api/files?path=<rel>` → `{"root":"/abs","path":"<rel 回显>","entries":[{"name","dir","size","modified","protected"}]}`；
 * - 相对路径规则（客户端 [validateRelPath] 与服务端一致地拒绝）：以 `/` 开头、含 `\` 或 NUL、
 *   出现空段（连续/结尾 `/`）、`.` / `..` 段；
 * - 尺寸/修改时间为服务端给的原始数值，人话化在本文件完成（不依赖平台日期库，
 *   [mtimeText] 用显式时区偏移保证测试确定性）。
 *
 * 状态与渲染见 [FilesState] / FilesPage.kt，平台选文件桥见 UploadFilePicker.kt。
 */

/** EchoMuse 自定义唤醒词模型在服务端的存放目录（设备端 openwakeword 的模型目录名）。 */
const val OWW_MODELS_DIR = "oww-models"

/** 文件夹名称长度上限（与服务端 mkdir 校验一致）。 */
const val FILES_NAME_MAX_LENGTH = 64

/**
 * 服务端存储目录里的一个条目（字段全部不可变快照；缺省字段安全兜底，不做增量更新）。
 *
 * `@Serializable` 只描述**线格式**（服务端契约 FROZEN，见文件头）：`protected` 是 Kotlin
 * 关键字，用 [SerialName] 映射；全部字段带默认值，字段缺失时取默认值而不是抛异常。
 */
@Serializable
data class FilesEntry(
    /** 空名条目由 [FilesModel.parseListing] 丢弃（服务端不该给，给了就不该渲染成空行）。 */
    val name: String = "",
    val dir: Boolean = false,
    val size: Long = 0L,
    /** 修改时间（unix 秒；服务端未给为 0 → 展示为空）。 */
    val modified: Long = 0L,
    /** 受保护文件（如 data.db）：展示「保护」徽标且不给删除按钮（服务端也会 403 拒绝）。 */
    @SerialName("protected") val isProtected: Boolean = false,
)

/** `GET /api/files` 的清单快照（字段缺省即空，与条目的兜底口径一致）。 */
data class FilesListing(
    /** 服务端存储根目录的绝对路径（页面顶部"文件到底存哪了"直接可见）。 */
    val root: String,
    /** 本次请求的相对路径回显（空串 = 根目录）。 */
    val path: String,
    val entries: List<FilesEntry>,
)

object FilesModel {

    // ============================================================
    // 路径规则（与服务端同规则镜像，客户端先拦一道、服务端仍是权威）
    // ============================================================

    /**
     * 校验一个**非空相对路径**（上传目标 = 当前目录 + 文件名；mkdir 目标同理）。
     * 返回错误文案；null = 合法。规则：拒绝空串、以 `/` 开头、反斜杠、NUL、
     * 空路径段（连续/结尾 `/`）与 `.` / `..` 段。
     */
    fun validateRelPath(path: String): String? = when {
        path.isEmpty() -> "路径不能为空"
        path.startsWith('/') -> "路径不能以 / 开头"
        path.contains('\\') -> "路径不能包含反斜杠"
        path.contains('\u0000') -> "路径包含非法字符"
        path.split('/').any { it.isEmpty() } -> "路径包含空的路径段"
        path.split('/').any { it == "." || it == ".." } -> "路径不能包含 . 或 .. 段"
        else -> null
    }

    /**
     * 校验**上传用的文件名**（落在当前目录的一层，不允许再带 `/` 下钻）。
     */
    fun validateUploadName(name: String): String? = when {
        name.isEmpty() -> "未选择文件"
        name.contains('/') -> "文件名不能包含 /"
        else -> validateRelPath(name)
    }

    /**
     * 校验**新建文件夹名**：1～64 字符且整体是一个合法路径段（不再允许 `/` 等分隔符）。
     */
    fun validateFolderName(name: String): String? = when {
        name.isEmpty() -> "请输入文件夹名称"
        name.length > FILES_NAME_MAX_LENGTH -> "文件夹名称不能超过 $FILES_NAME_MAX_LENGTH 个字符"
        name.contains('/') -> "文件夹名称不能包含 /"
        else -> validateRelPath(name)
    }

    /** 相对路径 → 目录段列表（面包屑/拼接用；空段过滤，空路径 = 空列表）。 */
    fun pathSegments(path: String): List<String> = path.split('/').filter { it.isNotEmpty() }

    /** 目录段 → 面包屑轨迹：段名 + 截至该段的相对路径（`"a/b"` → `[("a","a"),("b","a/b")]`）。 */
    fun crumbTrail(path: String): List<Pair<String, String>> {
        val trail = mutableListOf<Pair<String, String>>()
        var acc = ""
        for (seg in pathSegments(path)) {
            acc = joinPath(acc, seg)
            trail.add(seg to acc)
        }
        return trail
    }

    /** 拼接子路径（dir 为空串 = 根目录，直接用 name，不产生前导 `/`）。 */
    fun joinPath(dir: String, name: String): String = if (dir.isEmpty()) name else "$dir/$name"

    /** 父目录（`"a/b"` → `"a"`；`"a"` → `""` 即根目录）。 */
    fun parentOf(path: String): String {
        val segs = pathSegments(path)
        if (segs.isEmpty()) return ""
        var acc = ""
        for (seg in segs.dropLast(1)) acc = joinPath(acc, seg)
        return acc
    }

    // ============================================================
    // 展示格式化
    // ============================================================

    /** 字节数 → 人话（`0 B` 特例；KB/MB/GB 保留 1 位小数，四舍五入）。 */
    fun sizeText(bytes: Long): String = when {
        bytes <= 0L -> "0 B"
        bytes < 1024L -> "$bytes B"
        bytes < 1024L * 1024L -> "${decimal(bytes, 1024L)} KB"
        bytes < 1024L * 1024L * 1024L -> "${decimal(bytes, 1024L * 1024L)} MB"
        else -> "${decimal(bytes, 1024L * 1024L * 1024L)} GB"
    }

    /** bytes/unit 保留 1 位小数（四舍五入），如 1536/1024 → "1.5"。 */
    private fun decimal(bytes: Long, unit: Long): String {
        val tenth = (bytes * 10L + unit / 2L) / unit
        return "${tenth / 10L}.${tenth % 10L}"
    }

    /**
     * unix 秒 → `"yyyy-MM-dd HH:mm"`（公历换算交给 kotlinx-datetime）。
     *
     * 与 [formatEpochSeconds]（EchoMuse 时间戳的本地格式化）同锚点：`secs <= 0` 视为
     * "服务端未给" 返回空串。本地时区由调用方以 [zoneOffsetSeconds]（当前 UTC 偏移，秒）
     * 显式传入——纯函数不读系统时钟，测试传固定偏移即可复现。
     *
     * 实现上先把偏移加到时间戳，再按 UTC 折算本地时刻：与「按指定时区折算」完全等价，
     * 且不像 `UtcOffset(秒)`/`FixedOffsetTimeZone` 那样有 ±18 小时合法性校验（那是给
     * **真实时区**设的约束，而这里的入参本就是任意偏移量）。
     */
    fun mtimeText(epochSecs: Long, zoneOffsetSeconds: Int = 0): String {
        if (epochSecs <= 0L) return ""
        val dt = Instant.fromEpochSeconds(epochSecs + zoneOffsetSeconds)
            .toLocalDateTime(TimeZone.UTC)
        fun p2(v: Int): String = v.toString().padStart(2, '0')
        return "${dt.year}-${p2(dt.monthNumber)}-${p2(dt.dayOfMonth)} ${p2(dt.hour)}:${p2(dt.minute)}"
    }

    // ============================================================
    // 唤醒词模型文件
    // ============================================================

    /** 是否唤醒词模型文件（openwakeword 分类器，`.onnx`，大小写不敏感）。 */
    fun isModelFile(name: String): Boolean = name.endsWith(".onnx", ignoreCase = true)

    /** 模型文件名 → 模型主干（去掉 `.onnx` 扩展名；设备按 `<stem>.onnx` 加载）。非模型名原样返回。 */
    fun modelStem(fileName: String): String =
        if (isModelFile(fileName)) fileName.dropLast(".onnx".length) else fileName

    // ============================================================
    // 清单解析（raw JSON → 强类型；见文件尾的解码配置说明）
    // ============================================================

    /**
     * 解析 `GET /api/files` 回包原文。任何字段缺失/类型不符都安全兜底
     * （空名条目跳过、畸形 JSON → 空清单），不让单个坏条目毁掉整页。
     *
     * 三层兜底对应三种失败面，顺序不能换：
     * 1. 顶层不是合法 JSON 对象（截断、纯文本、顶层是数组）→ 整页空清单；
     * 2. `entries` 缺省或不是数组 → 只有 `entries` 为空，`root`/`path` 照常展示；
     * 3. 单个条目解码失败（如 `"oops"`、`42`）→ 只丢这一条。
     *
     * `root`/`path` 只认**字符串**（数字/布尔/null 一律当未给）：旧实现是 `as? String`，
     * 保持同一口径，避免把 `"root": 0` 渲染成 `"0"`。
     */
    fun parseListing(raw: String): FilesListing = runCatching {
        val obj = wireJson.parseToJsonElement(raw).jsonObject
        val entries = (obj["entries"] as? JsonArray)?.mapNotNull { item ->
            runCatching { wireJson.decodeFromJsonElement<FilesEntry>(item) }.getOrNull()
                ?.takeIf { it.name.isNotEmpty() }
        } ?: emptyList()
        FilesListing(root = obj.stringOrEmpty("root"), path = obj.stringOrEmpty("path"), entries = entries)
    }.getOrDefault(FilesListing("", "", emptyList()))

    /**
     * 从错误回包原文提取 `error`（或 `message`）文案；解析不出时回退 [fallback]。
     * 空串视同未给（服务端常用 `{"error":""}` 表示"没话说"），继续看下一个键。
     */
    fun errorText(raw: String, fallback: String): String {
        val obj = runCatching { wireJson.parseToJsonElement(raw).jsonObject }.getOrNull()
            ?: return fallback
        return obj.stringOrEmpty("error").ifEmpty { obj.stringOrEmpty("message") }.ifEmpty { fallback }
    }

    /** 条目的次级说明（右对齐 caption）：文件 = "1.5 MB · 2024-10-27 11:33"，目录只给时间。 */
    fun entryMeta(entry: FilesEntry, zoneOffsetSeconds: Int): String = listOfNotNull(
        if (entry.dir) null else sizeText(entry.size),
        mtimeText(entry.modified, zoneOffsetSeconds).ifEmpty { null },
    ).joinToString(" · ")
}

// ============================================================
// JSON 解码配置（仅供本文件解析 /api/files 与错误回包）
// ============================================================

/**
 * 这里此前是一份手写的 140 行递归下降解析器，注释写着"本文件按约定零依赖（adminCoreTests
 * 普通 JVM 模块没有 Kuikly classpath）"。引入 kotlinx-serialization 后这个理由不再成立：
 * 它是 commonMain 的纯 Kotlin 库，普通 JVM 测试模块能用、五端也不必各写一份，于是换成
 * "线格式数据类 + Json"（数据类见文件头的 [FilesEntry]）。
 *
 * 三个开关各有明确来源，改动前先想清楚会不会吃掉兜底语义：
 * - `ignoreUnknownKeys`：服务端随时可能给条目加字段（如 `mime`），客户端不该因为不认识的
 *   键就整页空；
 * - `coerceInputValues`：`null` 落到字段默认值——服务端对"没有值"有两种写法（省略键、
 *   显式 `null`），旧实现两种都兜住，这里保持；
 * - **不开** `isLenient`：宽松模式会接受单引号、裸键名等非 JSON，那会让"回包其实是错误页
 *   HTML"这类故障被静默吞成空清单。
 *
 * ⚠️ 与旧实现有两处**双向**差异（都由 `FilesModelTest.parseListingNumericFieldTolerance` 钉住）：
 * - `"size":"1024"` 这类字符串数字会被正确解出，旧实现 `as? Number` 失败后退化成 0（显示 "0 B"）；
 * - `1.5` / `"abc"` / `true` 这类非整数会让**该条目**解码失败被跳过，旧实现退化成 0 并保留。
 * 服务端契约（FROZEN）是 u64/i64 整数，真出现类型漂移应当暴露而不是显示成 0 字节。
 */
private val wireJson = Json {
    ignoreUnknownKeys = true
    coerceInputValues = true
}

/** 只认字符串原值（数字/布尔/null 视同未给，与旧实现 `as? String` 同一口径）。 */
private fun JsonObject.stringOrEmpty(key: String): String =
    (this[key] as? JsonPrimitive)?.takeIf { it.isString }?.content ?: ""

