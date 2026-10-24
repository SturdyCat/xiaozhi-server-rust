package com.xiaozhi.admin

import com.xiaozhi.admin.ui.*
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertNotNull
import kotlin.test.assertNull
import kotlin.test.assertTrue

class NodeTestModelsTest {
    private fun result(action: String = "llm.chat", state: String = "ok") = NodeTestResult(
        "server-id", action, "saved", "revision-A", state,
        NodeTestStage("llm", state, elapsedMs = 42, error = if (state == "ok") "" else "provider unavailable"),
    )

    @Test fun reducerRejectsDuplicatesAndOldCompletionsAfterScopeChange() {
        val ledger = NodeTestLedger()
        val first = assertNotNull(ledger.begin("llm.chat", "first input"))
        assertTrue(ledger.stateFor("llm.chat").operation is ActionState.Running)
        assertTrue(ledger.isActive(first))
        assertNull(ledger.begin("llm.chat", "duplicate"))
        ledger.invalidate()
        assertFalse(ledger.isActive(first))
        val second = assertNotNull(ledger.begin("llm.chat", "new server input"))
        assertFalse(ledger.complete(first, result()))
        assertTrue(ledger.stateFor("llm.chat").operation is ActionState.Running)
        assertTrue(ledger.complete(second, result()))
        assertFalse(ledger.isActive(second))
        assertEquals("new server input", ledger.stateFor("llm.chat").inputSummary)
        assertTrue(ledger.stateFor("llm.chat").operation is ActionState.Finished)
        assertFalse(ledger.complete(second, result()))
    }

    @Test fun differentNodesNeverShareOperationOrFailureState() {
        val ledger = NodeTestLedger()
        val llm = assertNotNull(ledger.begin("llm.chat", "hello"))
        val tts = assertNotNull(ledger.begin("tts.synthesis", "hi"))
        assertTrue(ledger.complete(llm, result(state = "unavailable")))
        assertTrue((ledger.stateFor("llm.chat").operation as ActionState.Finished).outcome is ActionOutcome.Failure)
        assertTrue(ledger.stateFor("tts.synthesis").operation is ActionState.Running)
        assertTrue(ledger.complete(tts, result("tts.synthesis")))
        assertTrue((ledger.stateFor("tts.synthesis").operation as ActionState.Finished).outcome is ActionOutcome.Success)
    }

    @Test fun mismatchedResponseAndUnknownStatesCannotBecomeSuccess() {
        val ledger = NodeTestLedger()
        val ticket = assertNotNull(ledger.begin("llm.chat", "hello"))
        assertTrue(ledger.complete(ticket, result("tts.synthesis")))
        assertEquals("response_mismatch", ledger.stateFor("llm.chat").result?.stage?.errorCode)
        assertFalse(assertNotNull(ledger.stateFor("llm.chat").result).succeeded)
        assertFalse(result(state = "queued").succeeded)
        assertFalse(result(state = "skipped").succeeded)
    }

    @Test fun capabilityIntersectionFailsClosedForUnknownInputsAndMissingClientSupport() {
        assertEquals(TestInputKind.Audio, TestInputKind.fromWire("sample_or_pcm"))
        assertEquals(TestInputKind.Unsupported, TestInputKind.fromWire("future-format"))
        val unknown = NodeTestCapability("llm.chat", true, "", "configured", TestInputKind.Unsupported)
        assertFalse(unknown.serverCapability().available)
        val recording = PipelinePanels.all.flatMap { it.tests }.single { it.id == "asr.recording" }
        assertTrue(recording.availability(ServerTestCapability(recording.id, true)) is Availability.Disabled)
        assertTrue(recording.availability(ServerTestCapability(recording.id, true), ClientTestCapability.entries.toSet(), Availability.Disabled("麦克风权限未授予")) is Availability.Disabled)
        assertTrue(recording.availability(ServerTestCapability(recording.id, true), ClientTestCapability.entries.toSet()) is Availability.Enabled)
    }

    @Test fun inputValidationUsesActualUtf8LimitAndDoesNotSubstituteFakeAudio() {
        assertNull(validateNodeTestInput(TestInputKind.None, "", ""))
        assertNotNull(validateNodeTestInput(TestInputKind.Audio, "hello", ""))
        assertNull(validateNodeTestInput(TestInputKind.Audio, "", "licensed-speech"))
        assertNotNull(validateNodeTestInput(TestInputKind.Text, "  ", ""))
        assertNull(validateNodeTestInput(TestInputKind.Text, "a".repeat(4096), ""))
        assertNotNull(validateNodeTestInput(TestInputKind.Text, "a".repeat(4097), ""))
        assertNotNull(validateNodeTestInput(TestInputKind.Text, "中".repeat(1366), ""))
    }
}
