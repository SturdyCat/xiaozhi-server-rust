package com.xiaozhi.admin

import kotlin.test.Test
import kotlin.test.assertContentEquals
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertNotEquals
import kotlin.test.assertNull
import kotlin.test.assertTrue

/** Pure control-plane tests: no Kuikly renderer, network, or physical BLE writes. */
class EchoMuseBleCodecTest {
    @Test
    fun addressesRequireSixHexPairsAndNormalizeCase() {
        assertEquals("aa:bb:cc:dd:ee:ff", bleNormalizeAddress(" AA:BB:cc:DD:ee:FF "))
        for (input in listOf("", "aabbccddeeff", "aa-bb-cc-dd-ee-ff", "aa:bb:cc:dd:ee", "aa:bb:cc:dd:ee:gg", "a:bb:cc:dd:ee:ff", "aa:bb:cc:dd:ee:ff:00")) {
            assertNull(bleNormalizeAddress(input), input)
        }
    }

    @Test
    fun attHandlesAreStrictDecimalAndNeverZero() {
        assertEquals(1, bleParseHandle("1"))
        assertEquals(65535, bleParseHandle("65535"))
        for (input in listOf("", "0", "65536", "999999999999999999999", "-1", "+1", "1.0", "0x10", "１", "١", "1;2", "1e2")) {
            assertNull(bleParseHandle(input), input)
        }
    }

    @Test
    fun hexInputSupportsAsciiWhitespaceAndUnsignedBytes() {
        assertContentEquals(byteArrayOf(0, 1, 127, 128.toByte(), 255.toByte()), bleHexToBytes("00 01\t7f\r\n80 ff"))
        assertEquals("AAE=", bleHexToBase64("00 01"))
        assertEquals("//8=", bleHexToBase64("FF ff"))
        assertEquals("AQAA", bleHexToBase64("01 00 00"))
    }

    @Test
    fun invalidAndAmbiguousHexNeverProducesWriteBytes() {
        for (input in listOf("", " \t\n", "0", "000", "0x01", "01,02", "01:02", "GG", "ＦＦ", "01\u00a002", "01\u200302")) {
            assertNull(bleHexToBytes(input), input)
            assertNull(bleHexToBase64(input), input)
        }
    }

    @Test
    fun writeSizeLimitIsDecodedBytesNotDisplayLength() {
        assertEquals(512, bleHexToBytes("ff ".repeat(512))?.size)
        assertNull(bleHexToBytes("ff".repeat(513)))
        assertContentEquals(byteArrayOf(1), bleHexToBytes("01", maxBytes = 1))
        assertNull(bleHexToBytes("0102", maxBytes = 1))
    }

    @Test
    fun readBase64RequiresCanonicalStandardEncoding() {
        assertEquals("", bleBase64ToHex("")) // A valid empty characteristic read.
        assertEquals("00", compact(bleBase64ToHex("AA==")))
        assertEquals("0102", compact(bleBase64ToHex("AQI=")))
        assertEquals("FFFFFF", compact(bleBase64ToHex("////")))
        for (input in listOf("A", "AAA", "AQ", "AQ=", "AQ===", "AB==", "AAF=", "AA==junk", "AA==\n", "____", "AA A")) {
            assertNull(bleBase64ToHex(input), input)
        }
    }

    @Test
    fun allByteValuesRoundTripThroughWireBase64() {
        val hex = (0..255).joinToString("") { it.toString(16).padStart(2, '0') }.uppercase()
        val wire = bleHexToBase64(hex)
        assertTrue(wire != null)
        assertEquals(hex, compact(bleBase64ToHex(wire)))
    }

    @Test
    fun everyFinalGroupLengthRoundTrips() {
        for (size in 1..32) {
            val hex = "AB".repeat(size)
            val wire = bleHexToBase64(hex)
            assertTrue(wire != null)
            assertEquals(hex, compact(bleBase64ToHex(wire)))
        }
    }

    private fun intent() = BleWriteIntent(
        deviceId = "dot", sessionId = "session-a", address = "aa:bb:cc:dd:ee:ff",
        addressType = 1, handle = 3, valueBase64 = "AQ==", response = true,
    )

    @Test
    fun writeConfirmationIncludesEveryControlField() {
        val original = intent()
        val fingerprint = bleWriteFingerprint(original)
        assertEquals(fingerprint, bleWriteFingerprint(original.copy()))
        val changed = listOf(
            original.copy(deviceId = "dot-other"), original.copy(sessionId = "session-b"),
            original.copy(address = "aa:bb:cc:dd:ee:00"), original.copy(addressType = 0),
            original.copy(handle = 4), original.copy(valueBase64 = "Ag=="),
            original.copy(response = false),
        )
        for (value in changed) assertNotEquals(fingerprint, bleWriteFingerprint(value), value.toString())
    }

    @Test
    fun writeConfirmationTupleDoesNotHaveDelimiterCollisions() {
        assertNotEquals(
            bleWriteFingerprint(intent().copy(deviceId = "a|b", sessionId = "c")),
            bleWriteFingerprint(intent().copy(deviceId = "a", sessionId = "b|c")),
        )
        assertNotEquals(
            bleWriteFingerprint(intent().copy(deviceId = "a:2", sessionId = "bc")),
            bleWriteFingerprint(intent().copy(deviceId = "a", sessionId = "2:bc")),
        )
    }

    @Test
    fun onlyExactNonemptySessionAndDeviceIdentityCanAcceptResults() {
        assertTrue(bleSameIdentity("dot", "one", "dot", "one"))
        assertFalse(bleSameIdentity("dot", "one", "dot", "two"))
        assertFalse(bleSameIdentity("dot", "one", "other", "one"))
        assertFalse(bleSameIdentity("dot", "", "dot", ""))
        assertFalse(bleSameIdentity("", "one", "", "one"))
        assertFalse(bleSameIdentity("dot", "one", "dot", ""))
        assertFalse(bleSameIdentity("dot", "", "dot", "one"))
    }

    @Test
    fun httpSuccessIsNotDeviceSuccessAndSlotsHasNoOkFlag() {
        assertTrue(bleGattSucceeded("result", true))
        assertTrue(bleGattSucceeded("slots", null))
        assertFalse(bleGattSucceeded("result", false))
        assertFalse(bleGattSucceeded("result", null))
        assertFalse(bleGattSucceeded("slots", false))
        assertFalse(bleGattSucceeded("notify", true))
        assertFalse(bleGattSucceeded("", true))
    }

    @Test
    fun changedDraftOrSessionInvalidatesCapturedWriteConfirmation() {
        val original = intent()
        assertTrue(bleWriteStillConfirmed(original, original.copy()))
        assertFalse(bleWriteStillConfirmed(null, original))
        assertFalse(bleWriteStillConfirmed(original, null))
        assertFalse(bleWriteStillConfirmed(original, original.copy(valueBase64 = "Ag==")))
        assertFalse(bleWriteStillConfirmed(original, original.copy(sessionId = "new-session")))
        assertFalse(bleWriteStillConfirmed(original, original.copy(handle = 4)))
        assertFalse(bleWriteStillConfirmed(original, original.copy(response = false)))
        assertFalse(bleWriteStillConfirmed(original.copy(sessionId = ""), original.copy(sessionId = "")))
    }

    @Test
    fun discoveredNumbersNeverTruncateToDifferentHandlesOrAddressTypes() {
        assertEquals(1, bleStrictInt(1, 0, 1))
        assertEquals(1, bleStrictInt(1.0, 0, 1))
        assertEquals(65535, bleStrictInt(65535L, 1, 65535))
        assertNull(bleStrictInt(0.5, 0, 1))
        assertNull(bleStrictInt(1.9, 1, 65535))
        assertNull(bleStrictInt(-1, 0, 1))
        assertNull(bleStrictInt(65536, 1, 65535))
        assertNull(bleStrictInt(Long.MAX_VALUE, 1, 65535))
        assertNull(bleStrictInt(Double.NaN, 1, 65535))
        assertNull(bleStrictInt(Double.POSITIVE_INFINITY, 1, 65535))
        assertNull(bleStrictInt("1", 1, 65535))
        assertNull(bleStrictInt(true, 0, 1))
        assertNull(bleStrictInt(null, 0, 1))
    }

    @Test
    fun characteristicPropertiesDistinguishResponseAndNoResponseWrites() {
        assertEquals("read", bleProperties(0x02))
        assertEquals("write-no-response", bleProperties(0x04))
        assertEquals("write", bleProperties(0x08))
        assertEquals("notify | indicate", bleProperties(0x30))
    }

    private fun compact(value: String?): String? = value?.replace(" ", "")?.uppercase()
}
