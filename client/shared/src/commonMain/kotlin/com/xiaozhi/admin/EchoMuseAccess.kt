package com.xiaozhi.admin

import com.xiaozhi.admin.ui.*

import com.tencent.kuikly.core.nvi.serialization.json.JSONObject

/** UTF-8 percent encoding shared by device path segments and GET-only token query parameters. */
internal fun echoMuseUrlComponent(value: String): String = buildString {
    val hex = "0123456789ABCDEF"
    for (byte in value.encodeToByteArray()) {
        val valueByte = byte.toInt() and 0xff
        if (valueByte in 65..90 || valueByte in 97..122 || valueByte in 48..57 ||
            valueByte == 45 || valueByte == 46 || valueByte == 95 || valueByte == 126
        ) append(valueByte.toChar())
        else {
            append('%')
            append(hex[valueByte shr 4])
            append(hex[valueByte and 15])
        }
    }
}

internal fun echoMuseAuthenticatedUrl(url: String, token: String): String =
    if (token.isEmpty()) url else url + (if ('?' in url) "&" else "?") + "token=" + echoMuseUrlComponent(token)

internal fun echoMuseHeaders(token: String): JSONObject = JSONObject().apply {
    put("Content-Type", "application/json")
    if (token.isNotEmpty()) put("Authorization", "Bearer $token")
}
