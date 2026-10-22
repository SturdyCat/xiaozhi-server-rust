package com.xiaozhi.admin

/**
 * 「文件管理」页的**纯逻辑**（服务端存储目录浏览 / 上传 / 删除的客户端镜像规则）。
 *
 * 本文件**零外部依赖**（不 import Kuikly/平台 API，同 [EchoMuseWakeWord] 的口径）：
 * 由普通 JVM 测试模块 `:adminCoreTests` 直接引用同一份生产源码回归（路径校验、清单解析、
 * 尺寸/时间格式化都是页面上"看起来对不对"的最后防线，必须可测）。
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

/** 服务端存储目录里的一个条目（字段全部不可变快照；缺省字段安全兜底，不做增量更新）。 */
data class FilesEntry(
    val name: String,
    val dir: Boolean = false,
    val size: Long = 0L,
    /** 修改时间（unix 秒；服务端未给为 0 → 展示为空）。 */
    val modified: Long = 0L,
    /** 受保护文件（如 data.db）：展示「保护」徽标且不给删除按钮（服务端也会 403 拒绝）。 */
    val isProtected: Boolean = false,
)

/** `GET /api/files` 的清单快照。 */
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
     * unix 秒 → `"yyyy-MM-dd HH:mm"`（commonMain 无日期库， Civil 日历换算见 [civilFromDays]）。
     *
     * 与平台 `formatEpochSeconds`（EchoMuse 时间戳的本地格式化）同锚点：`secs <= 0` 视为
     * "服务端未给" 返回空串。本地时区由调用方以 [zoneOffsetSeconds]（当前 UTC 偏移，秒）
     * 显式传入——纯函数不读系统时钟，测试传固定偏移即可复现。
     */
    fun mtimeText(epochSecs: Long, zoneOffsetSeconds: Int = 0): String {
        if (epochSecs <= 0L) return ""
        val local = epochSecs + zoneOffsetSeconds
        val days = local.floorDiv(SECONDS_PER_DAY)
        val secsOfDay = local.mod(SECONDS_PER_DAY)
        val (year, month, day) = civilFromDays(days)
        fun p2(v: Long): String = v.toString().padStart(2, '0')
        return "$year-${p2(month.toLong())}-${p2(day.toLong())} " +
            "${p2(secsOfDay / 3600L)}:${p2(secsOfDay % 3600L / 60L)}"
    }

    /** 天数（自 1970-01-01）→ (年, 月, 日)。Howard Hinnant 的 civil_from_days 算法。 */
    private fun civilFromDays(daysSinceEpoch: Long): Triple<Long, Int, Int> {
        val z = daysSinceEpoch + 719468L
        val era = (if (z >= 0) z else z - 146096L) / 146097L
        val doe = z - era * 146097L
        val yoe = (doe - doe / 1460L + doe / 36524L - doe / 146096L) / 365L
        val doy = doe - (365L * yoe + yoe / 4L - yoe / 100L)
        val mp = (5L * doy + 2L) / 153L
        val day = doy - (153L * mp + 2L) / 5L + 1L
        val month = if (mp < 10L) mp + 3L else mp - 9L
        val year = yoe + era * 400L + (if (month <= 2L) 1L else 0L)
        return Triple(year, month.toInt(), day.toInt())
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
    // 清单解析（raw JSON → 强类型；见文件尾的极简 JSON 解码说明）
    // ============================================================

    /**
     * 解析 `GET /api/files` 回包原文。任何字段缺失/类型不符都安全兜底
     * （空名条目跳过、畸形 JSON → 空清单），不让单个坏条目毁掉整页。
     */
    fun parseListing(raw: String): FilesListing {
        val obj = runCatching { parseJsonValue(raw) }.getOrNull() as? Map<*, *>
            ?: return FilesListing("", "", emptyList())
        val entriesRaw = obj["entries"] as? List<*> ?: emptyList<Any?>()
        val entries = entriesRaw.mapNotNull { item ->
            val e = item as? Map<*, *> ?: return@mapNotNull null
            val name = e["name"] as? String ?: ""
            if (name.isEmpty()) return@mapNotNull null
            FilesEntry(
                name = name,
                dir = e["dir"] == true,
                size = (e["size"] as? Number)?.toLong() ?: 0L,
                modified = (e["modified"] as? Number)?.toLong() ?: 0L,
                isProtected = e["protected"] == true,
            )
        }
        return FilesListing(
            root = obj["root"] as? String ?: "",
            path = obj["path"] as? String ?: "",
            entries = entries,
        )
    }

    /** 从错误回包原文提取 `error`（或 `message`）文案；解析不出时回退 [fallback]。 */
    fun errorText(raw: String, fallback: String): String {
        val obj = runCatching { parseJsonValue(raw) }.getOrNull() as? Map<*, *>
            ?: return fallback
        val text = (obj["error"] as? String)?.takeIf { it.isNotEmpty() }
            ?: (obj["message"] as? String)?.takeIf { it.isNotEmpty() }
            ?: return fallback
        return text
    }

    /** 条目的次级说明（右对齐 caption）：文件 = "1.5 MB · 2024-10-27 11:33"，目录只给时间。 */
    fun entryMeta(entry: FilesEntry, zoneOffsetSeconds: Int): String = listOfNotNull(
        if (entry.dir) null else sizeText(entry.size),
        mtimeText(entry.modified, zoneOffsetSeconds).ifEmpty { null },
    ).joinToString(" · ")

    private const val SECONDS_PER_DAY = 86400L
}

// ============================================================
// 极简 JSON 解码（仅供本文件解析 /api/files 与错误回包）
// ============================================================

/**
 * 为什么在这里手写一份 JSON 解码：本文件按约定零依赖（adminCoreTests 普通 JVM 模块没有
 * Kuikly classpath），而清单解析的"安全兜底"行为必须在生产源码上测试、不能在测试里复刻一份。
 * 只服务本文件的固定回包形状（对象/数组/字符串转义/数字/布尔/null），畸形输入抛
 * [IllegalArgumentException]，由调用方 `runCatching` 兜底。
 */
private fun parseJsonValue(text: String): Any? = JsonReader(text).readValue()

private class JsonReader(private val s: String) {
    private var i = 0

    fun readValue(): Any? {
        skipWhitespace()
        if (i >= s.length) fail("unexpected end of input")
        return when (s[i]) {
            '{' -> {
                i++; readObject()
            }
            '[' -> {
                i++; readArray()
            }
            '"' -> readString()
            't' -> {
                literal("true"); true
            }
            'f' -> {
                literal("false"); false
            }
            'n' -> {
                literal("null"); null
            }
            else -> readNumber()
        }
    }

    private fun readObject(): LinkedHashMap<String, Any?> {
        val obj = LinkedHashMap<String, Any?>()
        if (peek() == '}') {
            i++; return obj
        }
        while (true) {
            if (peek() != '"') fail("object key expected")
            val key = readString()
            if (peek() != ':') fail("':' expected")
            i++
            obj[key] = readValue()
            when (peek()) {
                ',' -> i++
                '}' -> {
                    i++; return obj
                }
                else -> fail("',' or '}' expected")
            }
        }
    }

    private fun readArray(): ArrayList<Any?> {
        val arr = ArrayList<Any?>()
        if (peek() == ']') {
            i++; return arr
        }
        while (true) {
            arr.add(readValue())
            when (peek()) {
                ',' -> i++
                ']' -> {
                    i++; return arr
                }
                else -> fail("',' or ']' expected")
            }
        }
    }

    private fun readString(): String {
        i++ // 开引号（peek 已确认是 '"'）
        val sb = StringBuilder()
        while (true) {
            if (i >= s.length) fail("unterminated string")
            when (val c = s[i]) {
                '"' -> {
                    i++; return sb.toString()
                }
                '\\' -> {
                    i++; sb.append(readEscape())
                }
                else -> {
                    sb.append(c); i++
                }
            }
        }
    }

    private fun readEscape(): Char {
        if (i >= s.length) fail("bad escape")
        return when (val c = s[i++]) {
            '"' -> '"'
            '\\' -> '\\'
            '/' -> '/'
            'b' -> '\b'
            'f' -> '\u000C'
            'n' -> '\n'
            'r' -> '\r'
            't' -> '\t'
            'u' -> {
                if (i + 4 > s.length) fail("bad unicode escape")
                val hex = s.substring(i, i + 4)
                i += 4
                hex.toIntOrNull(16)?.toChar() ?: fail("bad unicode escape")
            }
            else -> fail("bad escape")
        }
    }

    private fun readNumber(): Any {
        val start = i
        if (i < s.length && (s[i] == '-' || s[i] == '+')) i++
        var isDouble = false
        while (i < s.length && (s[i].isDigit() || s[i] == '.' || s[i] == 'e' || s[i] == 'E')) {
            if (s[i] == '.' || s[i] == 'e' || s[i] == 'E') isDouble = true
            i++
        }
        val raw = s.substring(start, i)
        if (raw.isEmpty()) fail("number expected")
        return if (isDouble) {
            raw.toDoubleOrNull() ?: fail("bad number '$raw'")
        } else {
            raw.toLongOrNull() ?: raw.toDoubleOrNull() ?: fail("bad number '$raw'")
        }
    }

    private fun literal(word: String) {
        if (!s.startsWith(word, i)) fail("'$word' expected")
        i += word.length
    }

    private fun peek(): Char {
        skipWhitespace()
        if (i >= s.length) fail("unexpected end of input")
        return s[i]
    }

    private fun skipWhitespace() {
        while (i < s.length && (s[i] == ' ' || s[i] == '\t' || s[i] == '\n' || s[i] == '\r')) i++
    }

    private fun fail(what: String): Nothing = throw IllegalArgumentException("JSON parse error at $i: $what")
}
