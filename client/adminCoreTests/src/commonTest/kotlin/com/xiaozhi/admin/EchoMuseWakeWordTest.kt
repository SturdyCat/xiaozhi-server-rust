package com.xiaozhi.admin

import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertTrue

/**
 * 唤醒词展示/选择纯逻辑回归（对照上游 wilbowes/EchoMuse @858987c 的
 * config.go 默认值、em_oww_assets.py STOCK_MODELS 与 em_oww_models.py
 * display_name 规则）。
 */
class EchoMuseWakeWordTest {
    @Test fun emptyStoredValueMeansDeviceDefaultHeyJarvis() {
        // 设备端 OWW_MODEL 缺省 hey_jarvis_v0.1；管理面没设置过（空/空白）就是它。
        assertEquals("hey_jarvis_v0.1", EchoMuseWakeWord.effectiveModel(""))
        assertEquals("hey_jarvis_v0.1", EchoMuseWakeWord.effectiveModel("   "))
        assertEquals("hey jarvis", EchoMuseWakeWord.displayName(""))
        assertTrue(EchoMuseWakeWord.isStock(""))
    }

    @Test fun displayNameFollowsUpstreamDisplayRule() {
        // 去 _v<数字> 版本后缀 + 下划线转空格。
        assertEquals("hey jarvis", EchoMuseWakeWord.displayName("hey_jarvis_v0.1"))
        assertEquals("alexa", EchoMuseWakeWord.displayName("alexa_v0.1"))
        assertEquals("hey mycroft", EchoMuseWakeWord.displayName("hey_mycroft_v0.1"))
        assertEquals("hey rhasspy", EchoMuseWakeWord.displayName("hey_rhasspy_v0.1"))
        // 无版本后缀的值只转下划线，不猜版本。
        assertEquals("custom word", EchoMuseWakeWord.displayName("custom_word"))
    }

    @Test fun stockSetMatchesUpstreamStockModels() {
        // 上游 em_oww_assets.py STOCK_MODELS（timer/weather 是意图模型，不在内）。
        assertEquals(
            listOf("hey_jarvis_v0.1", "alexa_v0.1", "hey_mycroft_v0.1", "hey_rhasspy_v0.1"),
            EchoMuseWakeWord.STOCK_MODELS,
        )
        assertTrue(EchoMuseWakeWord.isStock("alexa_v0.1"))
        assertFalse(EchoMuseWakeWord.isStock("custom_word"))
    }

    @Test fun customStoredValueSurvivesRoundTrip() {
        // 已存自定义模型名原样保留为生效值（设备被单独预装过的场景）。
        assertEquals("my_model", EchoMuseWakeWord.effectiveModel(" my_model "))
        assertEquals("my model", EchoMuseWakeWord.displayName("my_model"))
    }

    @Test fun optionLabelShowsPhraseAndModelId() {
        assertEquals("hey jarvis（hey_jarvis_v0.1）", EchoMuseWakeWord.optionLabel("hey_jarvis_v0.1"))
        assertEquals("hey jarvis（hey_jarvis_v0.1）", EchoMuseWakeWord.optionLabel(""))
    }

    // ============================================================
    // buildOwwOptions：官方四款 + oww-models/ 已上传模型 + 已存自定义值
    // ============================================================

    @Test fun buildOwwOptionsListsStockFirstWithDefaultMark() {
        val options = EchoMuseWakeWord.buildOwwOptions("", emptyList())
        val expected = EchoMuseWakeWord.STOCK_MODELS.map { model ->
            val mark = if (model == EchoMuseWakeWord.DEFAULT_MODEL) "，默认" else ""
            model to "${EchoMuseWakeWord.optionLabel(model)}$mark"
        }
        assertEquals(expected, options)
    }

    @Test fun buildOwwOptionsAppendsUploadedModelsWithServerPath() {
        val options = EchoMuseWakeWord.buildOwwOptions("", listOf("custom_a.onnx", "B_v2.ONNX", "readme.txt"))
        // 官方四款在前，上传模型按主干追加（非 .onnx 跳过）
        assertEquals(EchoMuseWakeWord.STOCK_MODELS.size + 2, options.size)
        assertEquals("custom_a" to "自定义（custom_a，oww-models/custom_a.onnx）", options[4])
        assertEquals("B_v2" to "自定义（B_v2，oww-models/B_v2.ONNX）", options[5])
    }

    @Test fun buildOwwOptionsDeduplicatesStockCollisionsAndStored() {
        // 上传文件与官方款同名（同主干）→ 不重复；已存自定义值与上传模型同值 → 不重复
        val uploaded = listOf("alexa_v0.1.onnx", "my_model.onnx")
        val options = EchoMuseWakeWord.buildOwwOptions("my_model", uploaded)
        assertEquals(EchoMuseWakeWord.STOCK_MODELS.size + 1, options.size)
        assertTrue(options.none { it.first == "alexa" })
        assertEquals("my_model" to "自定义（my_model，oww-models/my_model.onnx）", options.last())
    }

    @Test fun buildOwwOptionsKeepsStoredCustomValueWhenNotUploaded() {
        val options = EchoMuseWakeWord.buildOwwOptions(" my_model ", emptyList())
        assertEquals("my_model" to "自定义（my_model，需设备已预装）", options.last())
        // 已存官方值不追加自定义项
        assertEquals(EchoMuseWakeWord.STOCK_MODELS.size, EchoMuseWakeWord.buildOwwOptions("alexa_v0.1", emptyList()).size)
        // 空/空白已存值不追加
        assertEquals(EchoMuseWakeWord.STOCK_MODELS.size, EchoMuseWakeWord.buildOwwOptions("  ", emptyList()).size)
    }

    // ============================================================
    // 官方灵敏度滑杆移植（dashboard.jsx WAKE_T_* / reflectT / wakeTrackFor）
    // ============================================================

    @Test fun thresholdRangeMatchesOfficialConstants() {
        assertEquals(0.1, EchoMuseWakeWord.THRESHOLD_MIN)
        assertEquals(0.975, EchoMuseWakeWord.THRESHOLD_MAX)
        assertEquals(0.025, EchoMuseWakeWord.THRESHOLD_STEP)
        assertEquals(0.5, EchoMuseWakeWord.DEFAULT_THRESHOLD) // config.go OWW_THRESHOLD 缺省
    }

    @Test fun trackProgressRunsPreciseLeftEagerRight() {
        // 官方方向：Precise（高阈值）在左端 progress=0，Eager（低阈值）在右端 progress=1
        assertEquals(0f, EchoMuseWakeWord.trackProgress(0.975))
        assertEquals(1f, EchoMuseWakeWord.trackProgress(0.1))
        assertEquals(0.5f, EchoMuseWakeWord.trackProgress(0.5375)) // (0.975-0.5375)/0.875 = 0.5
        // 默认阈值 0.5 → (0.975-0.5)/0.875 ≈ 0.542857（Precise 偏左）
        assertEquals(0.542857f, EchoMuseWakeWord.trackProgress(0.5), 1e-5f)
    }

    @Test fun offTrackStoredValuesPinToTrackEnds() {
        // 旧面板可写 0.05 / 1.0：读数保真由 state 承担，滑杆把手钉在端点（官方 wakeTrackFor）
        assertEquals(1f, EchoMuseWakeWord.trackProgress(0.05))
        assertEquals(0f, EchoMuseWakeWord.trackProgress(1.0))
        // 非数值回退设备默认
        assertEquals(EchoMuseWakeWord.trackProgress(0.5), EchoMuseWakeWord.trackProgress(Double.NaN))
    }

    @Test fun dragQuantizesToOfficialStep() {
        assertEquals(0.975, EchoMuseWakeWord.quantizeTrack(0f))
        assertEquals(0.1, EchoMuseWakeWord.quantizeTrack(1f))
        // 官方量程内来回映射恒等（步进 0.025 的取值）
        for (t in listOf(0.1, 0.225, 0.5, 0.9, 0.975)) {
            assertEquals(t, EchoMuseWakeWord.quantizeTrack(EchoMuseWakeWord.trackProgress(t)), 1e-9)
        }
        // 非法进度钳到端点
        assertEquals(0.975, EchoMuseWakeWord.quantizeTrack(-3f))
        assertEquals(0.1, EchoMuseWakeWord.quantizeTrack(4f))
        assertEquals(0.975, EchoMuseWakeWord.quantizeTrack(Float.NaN))
    }

    // ============================================================
    // 官方「Wake word detection」模式选项（dashboard.jsx）
    // ============================================================

    @Test fun modeOptionsMatchOfficialSelectWithConditionalShadow() {
        val normal = listOf(
            "on" to "On this Echo（设备本地识别，官方推荐）",
            "off" to "On the controller（控制器侧识别）",
        )
        assertEquals(normal, EchoMuseWakeWord.buildModeOptions("on"))
        assertEquals(normal, EchoMuseWakeWord.buildModeOptions("off"))
        // shadow 是官方诊断档：仅当前已处于 shadow 时显示，且排在最后
        assertEquals(3, EchoMuseWakeWord.buildModeOptions("shadow").size)
        assertEquals("shadow", EchoMuseWakeWord.buildModeOptions("shadow").last().first)
    }

    @Test fun modeNoteCoversOfficialModes() {
        assertTrue(EchoMuseWakeWord.modeNote("on").contains("官方推荐"))
        assertTrue(EchoMuseWakeWord.modeNote("on").contains("不向外发送"))
        assertTrue(EchoMuseWakeWord.modeNote("off").contains("推流"))
        assertTrue(EchoMuseWakeWord.modeNote("shadow").contains("诊断"))
        assertEquals("", EchoMuseWakeWord.modeNote("bogus"))
    }
}
