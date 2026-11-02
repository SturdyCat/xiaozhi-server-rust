package com.xiaozhi.admin

import kotlinx.serialization.json.*
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFailsWith
import kotlin.test.assertFalse
import kotlin.test.assertSame
import kotlin.test.assertTrue

class WorkflowEditorModelTest {
    private fun objectOf(text: String) = Json.parseToJsonElement(text).jsonObject
    private fun binding(id: String, position: String = "start", order: Int = 0) =
        WorkflowBinding(id, "plugin.$id", position, true, order)

    private fun metadata(config: JsonObject = workflowConfigJson(emptyList()), version: Int = 1) = buildJsonObject {
        put("version", version)
        put("config", config)
    }

    @Test fun metadataParsesRevisionModePluginsConditionsAndBindings() {
        val row = binding("wake")
        val root = buildJsonObject {
            metadata(workflowConfigJson(listOf(row))).forEach { (key, value) -> put(key, value) }
            put("revision", "rev-17")
            put("mode", "remote")
            put("plugins", Json.parseToJsonElement("""[{
                "id":"wake","label":"唤醒","owner_position":"start",
                "allowed_positions":["start","end"],"config_paths":["wakeup"],
                "repeatable":true,"on_error_options":["continue","abort"],
                "condition_kinds":["device"]
            }]"""))
            put("condition_kinds", Json.parseToJsonElement("""[{
                "kind":"device","label":"设备条件","fields":[
                    {"key":"type","label":"类型","type":"select","options":["esp","echo"],"required":true},
                    {"key":"name"}
                ]
            }]"""))
        }
        val parsed = parseWorkflowMetadata(root.toString())
        assertEquals(1, parsed.version)
        assertEquals("rev-17", parsed.revision)
        assertEquals("remote", parsed.mode)
        assertEquals(listOf(row), parsed.bindings)
        assertEquals(WorkflowPluginOption("wake", "唤醒", "start", listOf("start", "end"),
            listOf("wakeup"), true, listOf("continue", "abort"), listOf("device")), parsed.plugins.single())
        val condition = parsed.conditions.single()
        assertEquals("device", condition.kind)
        assertEquals("设备条件", condition.label)
        assertEquals(WorkflowConditionField("type", "类型", "select", listOf("esp", "echo"), true), condition.fields[0])
        assertEquals(WorkflowConditionField("name", "name", "string", emptyList()), condition.fields[1])
    }

    @Test fun metadataOptionalFieldsUseWireDefaults() {
        val root = buildJsonObject {
            metadata().forEach { (key, value) -> put(key, value) }
            put("plugins", Json.parseToJsonElement("""[{"id":"wake"}]"""))
        }
        val parsed = parseWorkflowMetadata(root.toString())
        assertEquals("", parsed.revision)
        assertEquals("local", parsed.mode)
        assertTrue(parsed.conditions.isEmpty())
        assertTrue(parsed.bindings.isEmpty())
        assertEquals("wake", parsed.plugins.single().label)
        assertEquals(listOf("continue"), parsed.plugins.single().onErrorOptions)
        assertFalse(parsed.plugins.single().repeatable)
    }

    @Test fun metadataRejectsUnknownOrMissingEnvelopeVersion() {
        for (version in listOf(0, 2, -1, Int.MAX_VALUE)) {
            assertFailsWith<IllegalArgumentException> { parseWorkflowMetadata(metadata(version = version).toString()) }
        }
        assertFailsWith<IllegalArgumentException> {
            parseWorkflowMetadata("""{"config":{"version":1,"bindings":[]}}""")
        }
    }

    @Test fun metadataAlsoRejectsUnsupportedNestedConfigVersion() {
        val config = objectOf("""{"version":2,"bindings":[]}""")
        assertFailsWith<IllegalArgumentException> { parseWorkflowMetadata(metadata(config).toString()) }
    }

    @Test fun metadataRejectsNonObjectRootAndMissingConfig() {
        assertFailsWith<IllegalStateException> { parseWorkflowMetadata("[]") }
        assertFailsWith<IllegalStateException> { parseWorkflowMetadata("""{"version":1}""") }
    }

    @Test fun bindingsRejectUnknownAndMissingVersions() {
        for (text in listOf("""{"version":2,"bindings":[]}""", """{"bindings":[]}""")) {
            assertFailsWith<IllegalArgumentException> { parseWorkflowBindings(objectOf(text)) }
        }
    }

    @Test fun bindingsRejectMissingListAndNonObjectRows() {
        assertFailsWith<IllegalStateException> { parseWorkflowBindings(objectOf("""{"version":1}""")) }
        assertFailsWith<IllegalStateException> { parseWorkflowBindings(objectOf("""{"version":1,"bindings":[42]}""")) }
    }

    @Test fun malformedConditionNeverBecomesAlways() {
        for (condition in listOf("null", "true", "[]", "\"always\"")) {
            val config = objectOf("""{"version":1,"bindings":[{"id":"a","plugin_id":"test","position_id":"start","condition":$condition}]}""")
            assertFailsWith<IllegalStateException> { parseWorkflowBindings(config) }
        }
    }

    @Test fun bindingsRejectDuplicateIdsEvenAcrossDifferentPositions() {
        val config = workflowConfigJson(listOf(binding("same"), binding("same", "end")))
        assertFailsWith<IllegalArgumentException> { parseWorkflowBindings(config) }
        assertFailsWith<IllegalArgumentException> { parseWorkflowMetadata(metadata(config).toString()) }
    }

    @Test fun bindingsRejectBlankIdentityFields() {
        val row = binding("valid")
        for (invalid in listOf(row.copy(id = " "), row.copy(pluginId = ""), row.copy(positionId = "\t"))) {
            assertFailsWith<IllegalArgumentException> { parseWorkflowBindings(workflowConfigJson(listOf(invalid))) }
        }
    }

    @Test fun minimalBindingUsesEnabledOrderConditionAndErrorDefaults() {
        val config = objectOf("""{"version":1,"bindings":[{"id":"a","plugin_id":"plugin.a","position_id":"start"}]}""")
        assertEquals(listOf(binding("a")), parseWorkflowBindings(config))
    }

    @Test fun conditionRetainsUnknownNestedFieldsAndTypesThroughRoundTripAndMove() {
        val condition = objectOf("""{
            "kind":"future-condition","nested":{"all":[true,7,null,{"name":"原样"}]},
            "threshold":0.125,"literal":"a && b","optional":null
        }""")
        val row = binding("a", order = 10).copy(condition = condition, enabled = false, onError = "abort")
        val config = workflowConfigJson(listOf(row, binding("b", order = 20)))
        val parsed = parseWorkflowBindings(config)
        assertEquals(row, parsed.first())
        assertEquals(condition, parsed.first().toJson()["condition"])
        assertEquals(config, workflowConfigJson(parsed))
        assertEquals(condition, moveWorkflowBinding(parsed, "a", 1).first().condition)
        assertEquals(condition, parseWorkflowMetadata(metadata(config).toString()).bindings.first().condition)
    }

    @Test fun moveOnlyReordersPeersAtTheSamePosition() {
        val first = binding("a", order = 5)
        val foreign = binding("other", "end", 7)
        val second = binding("b", order = 15)
        val third = binding("c", order = 25)
        val baseline = listOf(first, foreign, second, third)
        val moved = moveWorkflowBinding(baseline, "a", 1)
        assertEquals(listOf("b", "a", "c"), moved.filter { it.positionId == "start" }.sortedBy { it.order }.map { it.id })
        assertEquals(listOf(10, 7, 0, 20), moved.map { it.order })
        assertSame(foreign, moved[1])
        assertEquals(baseline.map { it.id }, moved.map { it.id })
        assertEquals(listOf(first, foreign, second, third), baseline)
    }

    @Test fun moveUpAndTiedOrdersUseStableIdOrdering() {
        val rows = listOf(binding("b", order = 8), binding("a", order = 8), binding("c", order = 9))
        val moved = moveWorkflowBinding(rows, "b", -1)
        assertEquals(listOf("b", "a", "c"), moved.sortedBy { it.order }.map { it.id })
        assertEquals(listOf(0, 10, 20), moved.map { it.order })
    }

    @Test fun invalidMovesAreIdentityNoOps() {
        val rows = listOf(binding("a"), binding("b", order = 10), binding("alone", "end"))
        for ((id, direction) in listOf("missing" to 1, "a" to -1, "b" to 1,
            "a" to 0, "a" to 2, "b" to -2, "alone" to 1, "alone" to -1)) {
            assertSame(rows, moveWorkflowBinding(rows, id, direction))
        }
        val empty = emptyList<WorkflowBinding>()
        assertSame(empty, moveWorkflowBinding(empty, "missing", 1))
    }

    @Test fun unchangedSummaryIsEmpty() {
        val rows = listOf(binding("a"))
        assertEquals("", workflowChangeSummary(rows, rows.toList()))
        assertEquals("", workflowChangeSummary(emptyList(), emptyList()))
    }

    @Test fun summaryCountsAddedRemovedAndChangedIdsOnce() {
        val baseline = listOf(binding("remove"), binding("change"), binding("keep"))
        val current = listOf(binding("change").copy(enabled = false, order = 30, onError = "abort"),
            binding("keep"), binding("new1"), binding("new2"))
        assertEquals("新增 2 项，移除 1 项，调整 1 项", workflowChangeSummary(baseline, current))
    }

    @Test fun summaryIncludesConditionAndPositionChanges() {
        val row = binding("a")
        val condition = objectOf("""{"kind":"device","type":"echo"}""")
        for (changed in listOf(row.copy(condition = condition), row.copy(positionId = "end"),
            row.copy(pluginId = "another"), row.copy(order = 10))) {
            assertEquals("调整 1 项", workflowChangeSummary(listOf(row), listOf(changed)))
        }
        assertEquals("新增 1 项", workflowChangeSummary(emptyList(), listOf(row)))
        assertEquals("移除 1 项", workflowChangeSummary(listOf(row), emptyList()))
    }
}
