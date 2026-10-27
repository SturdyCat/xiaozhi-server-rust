package com.xiaozhi.admin

import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertNull
import kotlin.test.assertTrue

/**
 * 「文件」页纯逻辑回归：路径校验（与服务端同规则镜像）、清单解析安全兜底、
 * 尺寸/时间格式化（显式时区偏移保证确定性）、唤醒词模型文件名规则。
 */
class FilesModelTest {

    // ============================================================
    // 路径校验
    // ============================================================

    @Test fun relPathRejectsServerRules() {
        assertNull(FilesModel.validateRelPath("oww-models"))
        assertNull(FilesModel.validateRelPath("a/b/x.onnx"))
        // 空串 / 以 / 开头 / 反斜杠 / NUL
        assertTrue(FilesModel.validateRelPath("") != null)
        assertTrue(FilesModel.validateRelPath("/abs") != null)
        assertTrue(FilesModel.validateRelPath("a\\b") != null)
        assertTrue(FilesModel.validateRelPath("a\u0000b") != null)
        // 空段（连续/结尾 /）与 . / .. 段
        assertTrue(FilesModel.validateRelPath("a//b") != null)
        assertTrue(FilesModel.validateRelPath("a/b/") != null)
        assertTrue(FilesModel.validateRelPath("a/./b") != null)
        assertTrue(FilesModel.validateRelPath("a/../b") != null)
        assertTrue(FilesModel.validateRelPath("..") != null)
        assertTrue(FilesModel.validateRelPath(".") != null)
    }

    @Test fun uploadNameIsSingleSegment() {
        assertNull(FilesModel.validateUploadName("model.onnx"))
        assertTrue(FilesModel.validateUploadName("") != null)
        assertTrue(FilesModel.validateUploadName("a/b.onnx") != null)
        assertTrue(FilesModel.validateUploadName("/etc/passwd") != null)
        assertTrue(FilesModel.validateUploadName("..") != null)
    }

    @Test fun folderNameBoundedAndSingleSegment() {
        assertNull(FilesModel.validateFolderName("oww-models-2"))
        assertTrue(FilesModel.validateFolderName("") != null)
        assertTrue(FilesModel.validateFolderName("a/b") != null)
        assertTrue(FilesModel.validateFolderName("..") != null)
        assertEquals(
            "文件夹名称不能超过 64 个字符",
            FilesModel.validateFolderName("x".repeat(65)),
        )
        assertNull(FilesModel.validateFolderName("x".repeat(64)))
    }

    @Test fun segmentsJoinAndParent() {
        assertEquals(emptyList(), FilesModel.pathSegments(""))
        assertEquals(listOf("a", "b", "c"), FilesModel.pathSegments("a/b/c"))
        assertEquals(listOf("a", "b"), FilesModel.pathSegments("a//b/"))
        assertEquals("x", FilesModel.joinPath("", "x"))
        assertEquals("a/x", FilesModel.joinPath("a", "x"))
        assertEquals("", FilesModel.parentOf("a"))
        assertEquals("a", FilesModel.parentOf("a/b"))
        assertEquals("a/b", FilesModel.parentOf("a/b/c"))
    }

    @Test fun crumbTrailBuildsCumulativeRelPaths() {
        assertEquals(
            listOf("a" to "a", "b" to "a/b", "c" to "a/b/c"),
            FilesModel.crumbTrail("a/b/c"),
        )
        assertEquals(emptyList(), FilesModel.crumbTrail(""))
    }

    // ============================================================
    // 清单解析（安全兜底）
    // ============================================================

    @Test fun parseListingReadsWireShape() {
        val raw = """
            {"root":"/data/oww","path":"oww-models","entries":[
              {"name":"hey_jarvis_v0.1.onnx","dir":false,"size":1048576,"modified":1730000000,"protected":false},
              {"name":"sub","dir":true,"size":0,"modified":1730000001,"protected":false},
              {"name":"data.db","dir":false,"size":10,"modified":1730000002,"protected":true}
            ]}
        """.trimIndent()
        val listing = FilesModel.parseListing(raw)
        assertEquals("/data/oww", listing.root)
        assertEquals("oww-models", listing.path)
        assertEquals(3, listing.entries.size)
        val model = listing.entries[0]
        assertEquals("hey_jarvis_v0.1.onnx", model.name)
        assertFalse(model.dir)
        assertEquals(1048576L, model.size)
        assertEquals(1730000000L, model.modified)
        assertFalse(model.isProtected)
        assertTrue(listing.entries[1].dir)
        assertTrue(listing.entries[2].isProtected)
    }

    @Test fun parseListingDefaultsMissingFieldsAndSkipsBadEntries() {
        val listing = FilesModel.parseListing(
            """{"entries":[{"dir":true},{"name":""},{"name":"ok.onnx"},"oops",42]}""",
        )
        assertEquals("", listing.root)
        assertEquals("", listing.path)
        assertEquals(listOf(FilesEntry(name = "ok.onnx", dir = false)), listing.entries)
    }

    @Test fun parseListingMalformedJsonYieldsEmpty() {
        val listing = FilesModel.parseListing("""{"entries":[{"name":"broken""")
        assertEquals(FilesListing("", "", emptyList()), listing)
        assertEquals(FilesListing("", "", emptyList()), FilesModel.parseListing("not json at all"))
        assertEquals(FilesListing("", "", emptyList()), FilesModel.parseListing(""))
    }

    @Test fun parseListingUnescapesStrings() {
        val listing = FilesModel.parseListing(
            """{"root":"/a\u4e01b","path":"quote\"dir","entries":[{"name":"a\\b.onnx"}]}""",
        )
        assertEquals("/a丁b", listing.root)
        assertEquals("quote\"dir", listing.path)
        assertEquals("a\\b.onnx", listing.entries.single().name)
    }

    /**
     * 服务端加字段、给显式 `null`：都不该毁掉整条或整页。
     * 前者靠 `ignoreUnknownKeys`，后者靠 `coerceInputValues`（null 落字段默认值）。
     */
    @Test fun parseListingToleratesUnknownAndNullFields() {
        val listing = FilesModel.parseListing(
            """{"root":null,"path":"p","extra":"x","entries":[
                 {"name":"a.onnx","size":null,"modified":null,"mime":"application/octet-stream"}
               ]}""",
        )
        assertEquals("", listing.root)
        assertEquals("p", listing.path)
        assertEquals(listOf(FilesEntry(name = "a.onnx")), listing.entries)
    }

    /** 非字符串的 `root`/`path` 视同未给（旧实现 `as? String` 同口径，不渲染成 "0"/"true"）。 */
    @Test fun parseListingNonStringRootAndPathAreIgnored() {
        val listing = FilesModel.parseListing("""{"root":0,"path":true}""")
        assertEquals("", listing.root)
        assertEquals("", listing.path)
        assertEquals(emptyList(), listing.entries)
    }

    /**
     * 有意收紧（与旧手写解析器的唯一语义差异）：数字字段给成字符串时，该条目被跳过，
     * 而不是像旧实现那样 `(as? Number)?.toLong() ?: 0` 退化成 "0 B"。
     * 服务端契约是 u64/i64 整数——类型漂移应当暴露，见 FilesModel 解码配置的说明。
     */
    /**
     * 数字字段的宽容度（与旧手写解析器唯一的语义差异，两个方向都有）：
     * - `"size":"1024"`（字符串数字）→ 解出 1024，**比旧实现准**（旧实现 `as? Number`
     *   失败后退化成 0，条目显示 "0 B"）；
     * - `1.5` / `"abc"` / `true` → 该条目解码失败被跳过，旧实现是退化成 0 并保留。
     *
     * 服务端契约（FROZEN）是 u64/i64 整数，真出现类型漂移应当暴露而不是显示成 0 字节；
     * 见 FilesModel 解码配置的说明。
     */
    @Test fun parseListingNumericFieldTolerance() {
        val listing = FilesModel.parseListing(
            """{"entries":[
                 {"name":"strnum.onnx","size":"1024"},
                 {"name":"float.onnx","size":1.5},
                 {"name":"alpha.onnx","size":"abc"},
                 {"name":"bool.onnx","size":true}
               ]}""",
        )
        assertEquals(listOf("strnum.onnx"), listing.entries.map { it.name })
        assertEquals(1024L, listing.entries.single().size)
    }

    @Test fun errorTextPrefersErrorThenMessage() {
        assertEquals("boom", FilesModel.errorText("""{"error":"boom"}""", "fallback"))
        assertEquals("msg", FilesModel.errorText("""{"message":"msg"}""", "fallback"))
        assertEquals("fallback", FilesModel.errorText("""{"other":1}""", "fallback"))
        assertEquals("fallback", FilesModel.errorText("garbage", "fallback"))
        assertEquals("fallback", FilesModel.errorText("""{"error":""}""", "fallback"))
    }

    // ============================================================
    // 展示格式化
    // ============================================================

    @Test fun sizeTextHumanBuckets() {
        assertEquals("0 B", FilesModel.sizeText(0L))
        assertEquals("0 B", FilesModel.sizeText(-5L))
        assertEquals("5 B", FilesModel.sizeText(5L))
        assertEquals("1023 B", FilesModel.sizeText(1023L))
        assertEquals("1.0 KB", FilesModel.sizeText(1024L))
        assertEquals("1.5 KB", FilesModel.sizeText(1536L))
        assertEquals("1.0 MB", FilesModel.sizeText(1024L * 1024L))
        assertEquals("5.0 MB", FilesModel.sizeText(5L * 1024L * 1024L))
        assertEquals("1.0 GB", FilesModel.sizeText(1024L * 1024L * 1024L))
        assertEquals("2.5 GB", FilesModel.sizeText(2560L * 1024L * 1024L))
    }

    @Test fun mtimeTextFormatsLocalWithExplicitOffset() {
        // 服务端未给（<=0）→ 空串（同 formatEpochSeconds 锚点）
        assertEquals("", FilesModel.mtimeText(0L))
        assertEquals("", FilesModel.mtimeText(-1L))
        // 1730000000 = 2024-10-27T03:33:20Z
        assertEquals("2024-10-27 03:33", FilesModel.mtimeText(1730000000L, 0))
        assertEquals("2024-10-27 11:33", FilesModel.mtimeText(1730000000L, 8 * 3600))
        // 负偏移跨天回退
        assertEquals("2024-10-26 22:33", FilesModel.mtimeText(1730000000L, -5 * 3600))
    }

    // ============================================================
    // 唤醒词模型文件名
    // ============================================================

    @Test fun modelFileDetectionAndStem() {
        assertTrue(FilesModel.isModelFile("hey_jarvis_v0.1.onnx"))
        assertTrue(FilesModel.isModelFile("CUSTOM.ONNX"))
        assertFalse(FilesModel.isModelFile("model.bin"))
        assertFalse(FilesModel.isModelFile("onnx"))
        assertFalse(FilesModel.isModelFile("a.onnx.txt"))
        assertEquals("hey_jarvis_v0.1", FilesModel.modelStem("hey_jarvis_v0.1.onnx"))
        assertEquals("CUSTOM", FilesModel.modelStem("CUSTOM.ONNX"))
        assertEquals("plain.txt", FilesModel.modelStem("plain.txt"))
    }

    @Test fun owwModelsDirMatchesServerContract() {
        assertEquals("oww-models", OWW_MODELS_DIR)
        assertNull(FilesModel.validateRelPath(OWW_MODELS_DIR))
    }

    @Test fun entryMetaComposesSizeAndLocalTime() {
        val file = FilesEntry(name = "m.onnx", size = 1536L, modified = 1730000000L)
        assertEquals("1.5 KB · 2024-10-27 03:33", FilesModel.entryMeta(file, 0))
        val dir = FilesEntry(name = "sub", dir = true, modified = 1730000000L)
        assertEquals("2024-10-27 03:33", FilesModel.entryMeta(dir, 0))
        // 时间缺失 → 尺寸仍在
        assertEquals("5 B", FilesModel.entryMeta(FilesEntry(name = "f", size = 5L), 0))
    }
}
