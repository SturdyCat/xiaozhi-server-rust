package com.xiaozhi.admin

/** Pure Kotlin BLE form validation/codec. No platform, Pager, JSON or experimental codec API. */
internal fun bleNormalizeAddress(raw: String): String? {
    val value = raw.trim()
    val parts = value.split(':')
    if (parts.size != 6 || parts.any { it.length != 2 || it.any { c -> bleHexDigit(c) < 0 } }) return null
    return parts.joinToString(":").lowercase()
}

/** Reject fractional, non-finite, string, boolean and out-of-range JSON values; never truncate. */
internal fun bleStrictInt(value: Any?, min: Int, max: Int): Int? {
    val number = (value as? Number)?.toDouble() ?: return null
    if (min > max || !number.isFinite() || number % 1.0 != 0.0 || number < min.toDouble() || number > max.toDouble()) return null
    return number.toInt()
}

internal fun bleParseHandle(raw: String): Int? {
    val value = raw.trim()
    if (value.isEmpty() || value.any { it !in '0'..'9' }) return null
    return value.toIntOrNull()?.takeIf { it in 1..65535 }
}

internal fun bleHexToBytes(raw: String, maxBytes: Int = 512): ByteArray? {
    val value = raw.filterNot { it == ' ' || it == '\t' || it == '\r' || it == '\n' }
    if (maxBytes < 1 || value.isEmpty() || value.length % 2 != 0 || value.length / 2 > maxBytes) return null
    if (value.any { bleHexDigit(it) < 0 }) return null
    return ByteArray(value.length / 2) { i ->
        ((bleHexDigit(value[i * 2]) shl 4) or bleHexDigit(value[i * 2 + 1])).toByte()
    }
}

private const val BLE_BASE64 = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/"
private const val BLE_HEX = "0123456789abcdef"

private fun bleHexDigit(c: Char): Int = when (c) {
    in '0'..'9' -> c - '0'
    in 'a'..'f' -> c - 'a' + 10
    in 'A'..'F' -> c - 'A' + 10
    else -> -1
}

private fun bleEncodeBase64(bytes: ByteArray): String = buildString {
    var offset = 0
    while (offset < bytes.size) {
        val a = bytes[offset].toInt() and 255
        val b = if (offset + 1 < bytes.size) bytes[offset + 1].toInt() and 255 else 0
        val c = if (offset + 2 < bytes.size) bytes[offset + 2].toInt() and 255 else 0
        append(BLE_BASE64[a shr 2])
        append(BLE_BASE64[((a and 3) shl 4) or (b shr 4)])
        append(if (offset + 1 < bytes.size) BLE_BASE64[((b and 15) shl 2) or (c shr 6)] else '=')
        append(if (offset + 2 < bytes.size) BLE_BASE64[c and 63] else '=')
        offset += 3
    }
}

internal fun bleHexToBase64(raw: String): String? = bleHexToBytes(raw)?.let(::bleEncodeBase64)

/** Strict standard, padded, canonical base64; an empty read value is a valid empty byte string. */
internal fun bleBase64ToHex(raw: String): String? {
    if (raw.isEmpty()) return ""
    if (raw.length % 4 != 0) return null
    val bytes = mutableListOf<Byte>()
    for (offset in raw.indices step 4) {
        val quartet = raw.substring(offset, offset + 4)
        val last = offset + 4 == raw.length
        val a = BLE_BASE64.indexOf(quartet[0])
        val b = BLE_BASE64.indexOf(quartet[1])
        val c = BLE_BASE64.indexOf(quartet[2])
        val d = BLE_BASE64.indexOf(quartet[3])
        if (a < 0 || b < 0) return null
        if (quartet[2] == '=') {
            if (!last || quartet[3] != '=' || (b and 15) != 0) return null
        } else if (c < 0) return null
        if (quartet[3] == '=') {
            if (!last || (quartet[2] != '=' && (c and 3) != 0)) return null
        } else if (d < 0 || quartet[2] == '=') return null
        bytes.add(((a shl 2) or (b shr 4)).toByte())
        if (c >= 0) bytes.add((((b and 15) shl 4) or (c shr 2)).toByte())
        if (d >= 0) bytes.add((((c and 3) shl 6) or d).toByte())
    }
    return buildString {
        bytes.forEach {
            val value = it.toInt() and 255
            append(BLE_HEX[value shr 4])
            append(BLE_HEX[value and 15])
        }
    }
}

internal data class BleWriteIntent(
    val deviceId: String,
    val sessionId: String,
    val address: String,
    val addressType: Int,
    val handle: Int,
    val valueBase64: String,
    val response: Boolean,
)

/** A collision-free, length-prefixed tuple, not a cryptographic hash or authentication token. */
internal fun bleWriteFingerprint(intent: BleWriteIntent): String = listOf(
    intent.deviceId, intent.sessionId, intent.address, intent.addressType.toString(),
    intent.handle.toString(), intent.valueBase64, intent.response.toString(),
).joinToString("") { "${it.length}:$it" }

/** Empty session identities are not wildcards. BLE mutation must fail closed with old servers. */
internal fun bleSameIdentity(
    expectedDevice: String,
    expectedSession: String,
    actualDevice: String,
    actualSession: String,
): Boolean = expectedDevice.isNotEmpty() && expectedSession.isNotEmpty() &&
    expectedDevice == actualDevice && expectedSession == actualSession

internal fun bleGattSucceeded(type: String, ok: Boolean?): Boolean =
    type == "slots" && ok != false || type == "result" && ok == true

/** All fields must still equal the captured intent at the second explicit confirmation. */
internal fun bleWriteStillConfirmed(captured: BleWriteIntent?, current: BleWriteIntent?): Boolean =
    captured != null && current != null && captured.sessionId.isNotEmpty() &&
        bleWriteFingerprint(captured) == bleWriteFingerprint(current)

internal fun bleProperties(mask: Int): String = listOf(
    0x01 to "broadcast", 0x02 to "read", 0x04 to "write-no-response", 0x08 to "write",
    0x10 to "notify", 0x20 to "indicate", 0x40 to "signed-write", 0x80 to "extended",
).filter { mask and it.first != 0 }.joinToString(" | ") { it.second }.ifEmpty { "无已知属性" }
