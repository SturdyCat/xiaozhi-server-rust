package com.xiaozhi.admin

import com.xiaozhi.admin.ui.*
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertFailsWith
import kotlin.test.assertNotNull
import kotlin.test.assertNull
import kotlin.test.assertTrue

class UiContractsTest {
    @Test fun availabilityAndOperationAreIndependent() {
        val available = Availability.Enabled
        val states = listOf(ActionState.Idle, ActionState.Finished(ActionOutcome.Success()), ActionState.Finished(ActionOutcome.Failure("network")))
        for (state in states) {
            assertTrue(canInvokeAction(available, state))
            assertFalse(canInvokeAction(Availability.Disabled("offline"), state))
        }
        assertFalse(canInvokeAction(available, ActionState.Running(1)))
        assertFalse(canInvokeAction(Availability.Disabled("offline"), ActionState.Running(1)))
        assertFailsWith<IllegalArgumentException> { Availability.Disabled("") }
    }

    @Test fun mixedSelectionIsNotFalseOrAnInheritedOverride() {
        assertEquals(false, CheckState.Unchecked.booleanOrNull())
        assertEquals(true, CheckState.Checked.booleanOrNull())
        assertNull(CheckState.Indeterminate.booleanOrNull())
        assertEquals(CheckState.Checked, CheckState.Indeterminate.toggled())
        assertEquals(CheckState.Unchecked, CheckState.Checked.toggled())
        assertNull(OverrideChoice.Inherit.booleanOrNull())
        assertEquals(true, OverrideChoice.ForceOn.booleanOrNull())
        assertEquals(false, OverrideChoice.ForceOff.booleanOrNull())
    }

    @Test fun duplicateRequestsAndOldResultTimersCannotAffectNewRun() {
        val gate = NodeRequestGate()
        val first = assertNotNull(gate.begin("llm.chat"))
        assertNull(gate.begin("llm.chat"))
        assertTrue(gate.finish(first))
        assertTrue(gate.isLatest(first))
        assertFalse(gate.finish(first))
        val second = assertNotNull(gate.begin("llm.chat"))
        assertFalse(gate.isLatest(first))
        assertFalse(gate.finish(first))
        assertTrue(gate.isActive(second))
        assertTrue(gate.finish(second))
    }

    @Test fun scopeChangesRejectLateCallbacksAndDoNotBlockOtherNodes() {
        val gate = NodeRequestGate()
        val llm = assertNotNull(gate.begin("llm.chat"))
        val tts = assertNotNull(gate.begin("tts.synthesis"))
        assertTrue(gate.isActive(llm))
        assertTrue(gate.isActive(tts))
        gate.invalidate()
        assertFalse(gate.isLatest(llm))
        assertFalse(gate.finish(tts))
        val next = assertNotNull(gate.begin("tts.synthesis"))
        assertTrue(next.requestId > tts.requestId)
        assertFalse(gate.finish(tts))
        assertTrue(gate.finish(next))
    }

    @Test fun pipelineOrderAndCapabilityBindingsAreExplicit() {
        assertEquals(listOf("vad", "asr", "nlu", "llm", "tts"), PipelinePanels.all.map { it.stage.id })
        assertEquals("VAD → ASR → NLU → LLM → TTS", PipelinePanels.flowLabel)
        assertEquals("nlu", PipelinePanels.forSection("command")?.capabilityId)
        assertEquals("nlu", PipelinePanels.forSection("nlu")?.capabilityId)
        assertEquals("llm", PipelinePanels.forSection("memory")?.capabilityId)
        assertNull(PipelinePanels.forSection("echomuse"))
        assertEquals(PanelRendererKey.entries.toSet(), PipelinePanels.all.map { it.renderer }.toSet())
    }

    @Test fun webHidesLocalHardwareButKeepsSharedTests() {
        val tests = PipelinePanels.all.flatMap { it.tests }
        val web = tests.filter { it.visibleTo(emptySet()) }.map { it.id }
        assertEquals(listOf("vad.sample", "asr.sample", "nlu.classify", "llm.chat", "tts.synthesis"), web)
        val mac = tests.filter { it.visibleTo(ClientTestCapability.entries.toSet()) }
        assertTrue(mac.map { it.id }.containsAll(web))
        for (test in tests.filter { it.id in web }) {
            assertTrue(test.availability(ServerTestCapability(test.id, true)) is Availability.Enabled)
            assertTrue(test.availability(null) is Availability.Disabled)
            assertEquals(Availability.Disabled("missing model"), test.availability(ServerTestCapability(test.id, false, "missing model")))
            assertTrue(test.availability(ServerTestCapability("unknown", true)) is Availability.Disabled)
        }
    }

    @Test fun navigationKeepsStableSectionsAndOneStageBinding() {
        for (panel in PipelinePanels.all) {
            assertTrue(ConfigSections.all.any { it.id == panel.sectionId })
        }
        assertEquals("voice", ConfigSections.tabOf("vad"))
        assertEquals("nlu", ConfigSections.all.single { it.id == "nlu" }.capabilityId)
        assertEquals("fullchain", ConfigSections.all.single { it.id == "aiui" }.capabilityId)
        assertEquals("vad", ConfigSections.firstSectionIn("voice"))
    }

    /**
     * 导航拆分的硬约束：「语音流程」与「全局配置」两个菜单必须**正好划分**全部 section。
     * 每个 section 要么有流程节点（语音流程里能改），要么落在全局配置菜单；不允许两边都有，
     * 也不允许两边都没有（后者等于配置项在界面上失联）。
     */
    @Test fun everySectionBelongsToExactlyOneMenu() {
        val workflowSections = ConfigSections.all.filter { PipelineFlow.primaryForSection(it.id) != null }.map { it.id }
        val globalSections = ConfigSections.global.map { it.id }
        assertEquals(ConfigSections.all.map { it.id }.toSet(), (workflowSections + globalSections).toSet())
        assertTrue(workflowSections.intersect(globalSections.toSet()).isEmpty())
        // 全局配置菜单只剩没有流程位置的 section：流程阶段与连接点分支都在语音流程里
        assertTrue(VoiceStageId.entries.none { stage -> stage.id in globalSections })
        assertTrue(globalSections.containsAll(listOf("overview", "presets", "aiui", "server", "echomuse", "mcp", "ota")))
        assertTrue(ConfigSections.isGlobal("server") && !ConfigSections.isGlobal("llm") && !ConfigSections.isGlobal("soul"))
        assertEquals(ConfigSections.global.size, ConfigSections.globalGroups.sumOf { it.second.size })
    }
}
