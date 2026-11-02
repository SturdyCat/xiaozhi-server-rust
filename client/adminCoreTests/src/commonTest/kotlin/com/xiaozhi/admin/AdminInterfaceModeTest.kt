package com.xiaozhi.admin

import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNotEquals

class AdminInterfaceModeTest {
    @Test fun absentPreferenceDefaultsToSimple() {
        assertEquals(AdminInterfaceMode.Simple, AdminInterfaceMode.fromStored(""))
    }

    @Test fun corruptPreferenceDefaultsToSimple() {
        assertEquals(AdminInterfaceMode.Simple, AdminInterfaceMode.fromStored("unknown"))
    }

    @Test fun bothModesRoundTripWithoutConfiguration() {
        for (mode in AdminInterfaceMode.entries) {
            assertEquals(mode, AdminInterfaceMode.fromStored(mode.storedValue))
        }
    }

    @Test fun preferencesAreServerScoped() {
        assertNotEquals(interfaceModePreferenceKey("http://a:8000"), interfaceModePreferenceKey("http://b:8000"))
        assertNotEquals(interfaceModePreferenceKey("http://a:8000"), interfaceModePreferenceKey("http://a:8001"))
        assertNotEquals(interfaceModePreferenceKey(""), interfaceModePreferenceKey("http://a:8000"))
    }

    @Test fun equivalentTrailingSlashAndWhitespaceSharePreference() {
        assertEquals(interfaceModePreferenceKey("http://a:8000"), interfaceModePreferenceKey(" http://a:8000/ "))
        assertEquals(interfaceModePreferenceKey(""), interfaceModePreferenceKey("  "))
    }

    @Test fun credentialsQueryAndFragmentNeverEnterPreferenceKey() {
        val clean = interfaceModePreferenceKey("https://example.com/Admin")
        assertEquals(clean, interfaceModePreferenceKey("HTTPS://user:secret@EXAMPLE.COM/Admin/?token=private#sensitive"))
        assertEquals(clean, interfaceModePreferenceKey("https://other:password@example.com/Admin?token=changed"))
        assertEquals(interfaceModePreferenceKey(""), interfaceModePreferenceKey("?token=private#fragment"))
    }

    @Test fun schemeAndAuthorityAreCaseInsensitiveButBasePathsRemainDistinct() {
        assertEquals(interfaceModePreferenceKey("https://example.com/Admin"), interfaceModePreferenceKey("HTTPS://EXAMPLE.COM/Admin/"))
        assertNotEquals(interfaceModePreferenceKey("https://example.com/Admin"), interfaceModePreferenceKey("https://example.com/admin"))
        assertNotEquals(interfaceModePreferenceKey("https://example.com/one"), interfaceModePreferenceKey("https://example.com/two"))
        assertNotEquals(interfaceModePreferenceKey("http://example.com"), interfaceModePreferenceKey("https://example.com"))
    }

    @Test fun ipv6AndSchemeRelativeAddressesAreSanitized() {
        assertEquals(interfaceModePreferenceKey("https://[fe80::a]:8000/base"), interfaceModePreferenceKey("HTTPS://user:secret@[FE80::A]:8000/base/?token=private"))
        assertEquals(interfaceModePreferenceKey("//example.com/base"), interfaceModePreferenceKey("//user:secret@EXAMPLE.COM/base#private"))
    }

    @Test fun preferenceNamespaceDoesNotUseConfigurationRevision() {
        assertEquals("xz.admin.interfaceMode.v1:same-origin", interfaceModePreferenceKey(""))
    }
}
