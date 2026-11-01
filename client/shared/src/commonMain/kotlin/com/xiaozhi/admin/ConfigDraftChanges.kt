package com.xiaozhi.admin

import kotlinx.serialization.json.JsonObject

/** Only field paths, never values: safe to build a save confirmation even for secret changes. */
fun changedConfigPaths(before: JsonObject, after: JsonObject, prefix: String = ""): List<String> =
    (before.keys + after.keys).flatMap { key ->
        val path = if (prefix.isEmpty()) key else "$prefix.$key"
        val left = before[key]
        val right = after[key]
        when {
            left == right -> emptyList()
            left is JsonObject && right is JsonObject -> changedConfigPaths(left, right, path)
            else -> listOf(path)
        }
    }
