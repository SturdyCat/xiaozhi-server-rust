package com.xiaozhi.admin

import com.xiaozhi.admin.ui.*
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFailsWith
import kotlin.test.assertNotEquals
import kotlin.test.assertNotNull
import kotlin.test.assertNull
import kotlin.test.assertTrue

class PipelineFlowTest {
    @Test fun mainStagesRemainOrderedWithFeaturesAtEverySupportedAnchor() {
        val anchors = listOf(PipelineAnchor.Start) + VoiceStageId.entries.flatMap {
            listOf(PipelineAnchor.Before(it), PipelineAnchor.After(it))
        } + PipelineAnchor.End
        val features = anchors.mapIndexed { index, anchor ->
            FlowFeatureDescriptor("feature-$index", "Feature $index", "test", anchor, "audio", IconToken.Info)
        }
        val flow = PipelineFlow.build(features)
        assertEquals(VoiceStageId.entries.toList(), flow.mapNotNull { it.coreStage })
        assertEquals(features.size + 5, flow.size)
        assertEquals("feature-0", flow.first().id)
        assertEquals(features.last().id, flow.last().id)
        for (stage in VoiceStageId.entries) {
            val index = flow.indexOfFirst { it.coreStage == stage }
            assertEquals(PipelineAnchor.Before(stage), flow[index - 1].anchor)
            assertEquals(PipelineAnchor.After(stage), flow[index + 1].anchor)
        }
    }

    @Test fun sharedConfigurationDoesNotCollapseNodeIdentity() {
        val recall = assertNotNull(PipelineFlow.selectionForNode("memory.recall"))
        val writeback = assertNotNull(PipelineFlow.selectionForNode("memory.writeback"))
        assertEquals("memory", recall.sectionId)
        assertEquals(recall.sectionId, writeback.sectionId)
        assertNotEquals(recall, writeback)
        assertEquals("voice", PipelineFlow.tabFor(recall))
        assertEquals("voice", PipelineFlow.tabFor(writeback))
        assertEquals("agent", PipelineFlow.tabFor(ConfigSelection("memory")))
        assertEquals("service", PipelineFlow.tabFor(ConfigSelection("audio")))
        assertEquals("voice", PipelineFlow.tabFor(assertNotNull(PipelineFlow.selectionForNode("output"))))
        assertEquals("service", PipelineFlow.tabFor(ConfigSelection("audio", "memory.recall")))
        assertNull(PipelineFlow.selectionForNode("unknown"))
    }

    @Test fun featuresHaveConfigAndTestBindingsWithoutFabricatedRuntimeReadiness() {
        for (feature in PipelineFlow.features) {
            assertTrue(ConfigSections.all.any { it.id == feature.sectionId })
            assertTrue(feature.configPaths.isNotEmpty())
            assertTrue(feature.tests.isNotEmpty())
            assertTrue(feature.tests.all { it.availability(null) is Availability.Disabled })
        }
        assertEquals("wakeup", PipelineFlow.primaryForSection("wakeup")?.id)
        assertEquals("llm", PipelineFlow.primaryForSection("llm")?.id)
        assertEquals("memory.recall", PipelineFlow.primaryForSection("memory")?.id)
        assertEquals(listOf("audio", "playback"), PipelineFlow.features.single { it.id == "output" }.configPaths)
    }

    @Test fun duplicateIdsFailAndAttachmentOrderIsDeterministic() {
        val early = FlowFeatureDescriptor("early", "Early", "", PipelineAnchor.Before(VoiceStageId.Tts), "audio", IconToken.Info, order = 1)
        val late = early.copy(id = "late", order = 2)
        val flow = PipelineFlow.build(listOf(late, early))
        val tts = flow.indexOfFirst { it.coreStage == VoiceStageId.Tts }
        assertEquals(listOf("early", "late"), flow.subList(tts - 2, tts).map { it.id })
        assertFailsWith<IllegalArgumentException> { PipelineFlow.build(listOf(early, early)) }
        assertFailsWith<IllegalArgumentException> { PipelineFlow.build(listOf(early.copy(id = "tts"))) }
    }
}
