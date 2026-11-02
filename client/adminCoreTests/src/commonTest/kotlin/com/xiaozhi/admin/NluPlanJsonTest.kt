package com.xiaozhi.admin

import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonPrimitive

class NluPlanJsonTest {
    @Test
    fun validNestedPayloadIsNotFlattenedOrSemanticallyGuessed() {
        val plan = """{"conditions":[{"kind":"slot_present","slot":"task"}],"steps":[{"api":"devices.call","args":{"tool":"lamp","arguments":{"nested":[1,true,{"text":"{{slots.task}}"}]}}}],"future":{"keep":[1,2]}}"""
        assertEquals("", nluPlanError(plan))
    }

    @Test
    fun clockParametersAndRealSpeechReferencesRoundTripThroughProductionActionJson() {
        val schedules = listOf(
            "\"after\":\"五分钟\"", "\"at\":\"大后天晚上八点\"", "\"interval\":\"两小时\"",
            "\"daily\":\"08:00\"", "\"workdays\":\"下午八点半\"", "\"hourly\":20",
            "\"weekly\":{\"weekday\":1,\"time\":\"08:00\"}",
            "\"monthly\":{\"day\":31,\"time\":\"08:00\"}",
            "\"yearly\":{\"month\":2,\"day\":29,\"time\":\"08:00\"}",
        )
        for (schedule in schedules) {
            val plan = "{\"conditions\":[],\"steps\":[{\"api\":\"tasks.create\",\"args\":{\"text\":\"{{slots.task}}\",\"timezone\":\"Asia/Shanghai\",$schedule}},{\"api\":\"replies.say\",\"args\":{\"text\":\"{{steps.0.speech}}\"}}]}"
            assertEquals("", nluPlanError(plan))
            val result = Json.parseToJsonElement(nluActionJson("{\"extension\":{\"keep\":[true,1]}}", "plan", "", plan)).jsonObject
            assertEquals(Json.parseToJsonElement(plan), result["plan"])
            assertEquals("", result.getValue("reply").jsonPrimitive.content)
            assertEquals("{{steps.0.speech}}", result.getValue("plan").jsonObject.getValue("steps").jsonArray[1].jsonObject.getValue("args").jsonObject.getValue("text").jsonPrimitive.content)
            assertEquals(Json.parseToJsonElement("{\"keep\":[true,1]}"), result["extension"])
        }
        // Syntax/round-trip tests do not claim the server executed or semantically accepted a schedule.
    }

    @Test
    fun legacyKindSelectionRemovesPlanButInvalidPlanInputNeverReplacesLoadedData() {
        val plan = """{"conditions":[],"steps":[{"api":"tasks.list","args":{}},{"api":"replies.say","args":{"text":"{{steps.0.speech}}"}}]}"""
        val source = "{\"kind\":\"plan\",\"reply\":\"\",\"plan\":$plan,\"extension\":true}"
        for (kind in listOf("reply", "end_session", "mcp:lamp")) {
            val legacy = Json.parseToJsonElement(nluActionJson(source, kind, "好", "{")).jsonObject
            assertTrue("plan" !in legacy, kind)
            assertEquals("true", legacy.getValue("extension").jsonPrimitive.content)
        }
        val invalid = Json.parseToJsonElement(nluActionJson(source, "plan", "", "{")).jsonObject
        assertEquals(Json.parseToJsonElement(plan), invalid["plan"])
        assertTrue(nluPlanError("{").isNotEmpty())
    }

    @Test
    fun invalidSyntaxAndShapeAreRejectedWithoutSubstitutingDefaults() {
        for (plan in listOf("{", "[]", "null", "{}", "{\"steps\":[]}", "{\"steps\":[1]}", "{\"conditions\":{},\"steps\":[{}]}")) {
            assertTrue(nluPlanError(plan).isNotEmpty(), plan)
        }
    }

    @Test
    fun structuralLimitsMatchTheServerEnvelope() {
        val step = """{"api":"replies.say","args":{"text":"你好"}}"""
        val sixteen = List(16) { step }.joinToString(",")
        assertEquals("", nluPlanError("{\"conditions\":[],\"steps\":[$sixteen]}"))
        val seventeen = List(17) { step }.joinToString(",")
        assertTrue(nluPlanError("{\"steps\":[$seventeen]}").isNotEmpty())
        val tooManyConditions = List(17) { "{}" }.joinToString(",")
        assertTrue(nluPlanError("{\"conditions\":[$tooManyConditions],\"steps\":[$step]}").isNotEmpty())
    }

    @Test
    fun capacityUsesUtf8BytesNotCharacterCount() {
        val value = "中".repeat(12000)
        val plan = "{\"steps\":[{\"api\":\"replies.say\",\"args\":{\"text\":\"$value\"}}]}"
        assertTrue(plan.length < 32768)
        assertTrue(nluPlanError(plan).isNotEmpty())
    }
}
