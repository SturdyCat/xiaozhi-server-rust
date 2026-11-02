package com.xiaozhi.admin

import kotlinx.serialization.json.*
import kotlin.test.*

class ConfigDraftChangesTest {
    private fun obj(text: String) = Json.parseToJsonElement(text) as JsonObject

    @Test fun reportsOnlyChangedLeavesAndPreservesOtherSections() {
        val before = obj("""{"asr":{"model":"old","lang":"auto"},"llm":{"model":"same"}}""")
        val after = obj("""{"asr":{"model":"new","lang":"auto"},"llm":{"model":"same"}}""")
        assertEquals(listOf("asr.model"), changedConfigPaths(before, after))
    }
    @Test fun secretsNeverAppearInSummaryPaths() {
        val before = obj("""{"llm":{"api_key":"secret-before"}}""")
        val after = obj("""{"llm":{"api_key":"secret-after"}}""")
        val paths = changedConfigPaths(before, after)
        assertEquals(listOf("llm.api_key"), paths)
        assertFalse(paths.toString().contains("secret"))
    }
    @Test fun insertionRemovalNullAndNestedFieldChangesAreDetected() {
        val before = obj("""{"a":{"keep":1,"remove":3},"b":null}""")
        val after = obj("""{"a":{"keep":1,"new":2},"b":false}""")
        assertEquals(setOf("a.remove", "a.new", "b"), changedConfigPaths(before, after).toSet())
    }
    @Test fun arrayIsOneActionableFieldNotValueDump() {
        val before = obj("""{"nlu":{"rules":[{"secret":"old"}]}}""")
        val after = obj("""{"nlu":{"rules":[{"secret":"new"}]}}""")
        assertEquals(listOf("nlu.rules"), changedConfigPaths(before, after))
    }
    @Test fun sameObjectsAndSameArraysDoNotTriggerConfirmation() {
        val data = obj("""{"memory":{"recall":4},"routes":[1,2]}""")
        assertEquals(emptyList(), changedConfigPaths(data, obj(data.toString())))
    }
}
