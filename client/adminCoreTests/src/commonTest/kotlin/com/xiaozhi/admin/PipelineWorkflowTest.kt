package com.xiaozhi.admin

import com.xiaozhi.admin.ui.*
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFailsWith
import kotlin.test.assertFalse
import kotlin.test.assertNotEquals
import kotlin.test.assertNotNull
import kotlin.test.assertNull
import kotlin.test.assertTrue

class PipelineWorkflowTest {
    @Test fun horizontalWorkflowHasFiveStagesAndSixSelectableDotsInStableOrder() {
        val positions = PipelineWorkflow.positions
        assertEquals(
            listOf("start", "vad", "link.vad.asr", "asr", "link.asr.nlu", "nlu", "link.nlu.llm", "llm", "link.llm.tts", "tts", "end"),
            positions.map { it.id },
        )
        assertEquals(VoiceStageId.entries.toList(), positions.mapNotNull { it.stage })
        assertEquals(5, positions.count { it.isCore })
        assertEquals(6, positions.count { !it.isCore })
        assertTrue(positions.all { it.label.isNotBlank() })
        positions.forEachIndexed { index, position -> assertEquals(index % 2 == 1, position.isCore) }
    }

    @Test fun allTwelveRealNodesHaveExactlyOneHomeWithoutChangingTheirBindings() {
        val projected = PipelineWorkflow.positions.flatMap { it.nodes }
        assertEquals(12, projected.size)
        assertEquals(PipelineFlow.all, projected)
        assertEquals(projected.size, projected.map { it.id }.distinct().size)
        assertEquals(listOf("input", "wakeup"), PipelineWorkflow.forId("start")?.nodes?.map { it.id })
        assertEquals(listOf("command"), PipelineWorkflow.forId("link.asr.nlu")?.nodes?.map { it.id })
        assertEquals(listOf("soul", "memory.recall"), PipelineWorkflow.forId("link.nlu.llm")?.nodes?.map { it.id })
        assertEquals(listOf("output", "memory.writeback"), PipelineWorkflow.forId("end")?.nodes?.map { it.id })
        for (node in PipelineFlow.all) {
            val position = assertNotNull(PipelineWorkflow.forNode(node.id))
            assertEquals(node, position.nodes.single { it.id == node.id })
            assertEquals(position, PipelineWorkflow.resolve(node.selection))
            assertEquals("voice", PipelineFlow.tabFor(node.selection))
        }
        assertEquals(listOf("audio", "playback"), PipelineFlow.features.single { it.id == "output" }.configPaths)
    }

    @Test fun selectingEveryPositionRoundTripsThroughTheSameSelectionModel() {
        for (position in PipelineWorkflow.positions) {
            val selection = assertNotNull(PipelineWorkflow.selectionForPosition(position.id))
            assertEquals(position.id, selection.workflowPositionId)
            assertEquals(position, PipelineWorkflow.resolve(selection))
            assertEquals("voice", PipelineFlow.tabFor(selection))
            val first = position.nodes.firstOrNull()
            if (first != null) {
                assertEquals(first.sectionId, selection.sectionId)
                assertEquals(first.id, selection.nodeId)
            } else {
                assertEquals(ConfigSelection("workflow", null, position.id), selection)
            }
        }
    }

    @Test fun emptyLinksAreSelectableButContainNoInventedConfigurationTestsOrCapabilities() {
        val empty = PipelineWorkflow.positions.filter { it.nodes.isEmpty() }
        assertEquals(listOf("link.vad.asr", "link.llm.tts"), empty.map { it.id })
        for (position in empty) {
            assertFalse(position.isCore)
            assertNull(position.stage)
            assertTrue(position.nodes.flatMap { it.tests }.isEmpty())
            assertTrue(position.nodes.mapNotNull { it.capabilityId }.isEmpty())
            val selection = assertNotNull(PipelineWorkflow.selectionForPosition(position.id))
            assertNull(selection.nodeId)
            assertTrue(ConfigSections.all.none { it.id == selection.sectionId })
        }
        assertEquals(12, PipelineFlow.all.size)
    }

    @Test fun legacyRoutesAndSharedSectionsKeepTheirDistinctNodeIdentity() {
        val recall = ConfigSelection("memory", "memory.recall")
        val writeback = ConfigSelection("memory", "memory.writeback")
        assertEquals("link.nlu.llm", PipelineWorkflow.resolve(recall)?.id)
        assertEquals("end", PipelineWorkflow.resolve(writeback)?.id)
        assertNotEquals(PipelineWorkflow.resolve(recall), PipelineWorkflow.resolve(writeback))
        assertEquals("start", PipelineWorkflow.resolve(ConfigSelection("audio", "input"))?.id)
        assertEquals("end", PipelineWorkflow.resolve(ConfigSelection("audio", "output"))?.id)
        for (section in listOf("memory", "audio", "llm", "server")) {
            val selection = ConfigSelection(section)
            assertNull(PipelineWorkflow.resolve(selection))
            assertEquals(ConfigSections.tabOf(section), PipelineFlow.tabFor(selection))
        }
    }

    @Test fun mismatchedUnknownAndForgedRoutesDoNotResolveToRealConfiguration() {
        assertNull(PipelineWorkflow.forId("unknown"))
        assertNull(PipelineWorkflow.forNode("unknown"))
        assertNull(PipelineWorkflow.selectionForPosition("unknown"))
        for (selection in listOf(
            ConfigSelection("audio", "memory.recall"),
            ConfigSelection("memory", "memory.recall", "end"),
            ConfigSelection("audio", "input", "unknown"),
            ConfigSelection("audio", "unknown", "start"),
            ConfigSelection("workflow", "unknown", "link.vad.asr"),
            ConfigSelection("audio", null, "link.vad.asr"),
            ConfigSelection("workflow", null, "start"),
            ConfigSelection("workflow", null, "vad"),
            ConfigSelection("workflow", null, "unknown"),
        )) assertNull(PipelineWorkflow.resolve(selection), "$selection must not fabricate a valid route")
    }

    @Test fun adjacentBeforeAfterAndBoundaryAnchorsAggregateInExistingFlowOrder() {
        val anchors = listOf(PipelineAnchor.Start) + VoiceStageId.entries.flatMap {
            listOf(PipelineAnchor.Before(it), PipelineAnchor.After(it))
        } + PipelineAnchor.End
        val features = anchors.mapIndexed { index, anchor ->
            FlowFeatureDescriptor("feature-$index", "Feature $index", "", anchor, "audio", IconToken.Info)
        }
        val flow = PipelineFlow.build(features)
        val positions = PipelineWorkflow.build(flow)
        assertEquals(flow, positions.flatMap { it.nodes })
        assertEquals(listOf(PipelineAnchor.Start, PipelineAnchor.Before(VoiceStageId.Vad)), positions.first().nodes.map { it.anchor })
        assertEquals(listOf(PipelineAnchor.After(VoiceStageId.Tts), PipelineAnchor.End), positions.last().nodes.map { it.anchor })
        for (index in 0 until VoiceStageId.entries.lastIndex) {
            val left = VoiceStageId.entries[index]
            val right = VoiceStageId.entries[index + 1]
            val link = positions.single { it.id == "link.${left.id}.${right.id}" }
            assertEquals(listOf(PipelineAnchor.After(left), PipelineAnchor.Before(right)), link.nodes.map { it.anchor })
        }
    }

    @Test fun projectionRejectsDuplicateMissingAndUnanchoredRealNodes() {
        val nodes = PipelineFlow.all
        assertFailsWith<IllegalArgumentException> { PipelineWorkflow.build(nodes + nodes.first()) }
        assertFailsWith<IllegalArgumentException> { PipelineWorkflow.build(nodes.filterNot { it.coreStage == VoiceStageId.Vad }) }
        assertFailsWith<IllegalArgumentException> { PipelineWorkflow.build(nodes + nodes.first().copy(id = "orphan", anchor = null)) }
    }
}
