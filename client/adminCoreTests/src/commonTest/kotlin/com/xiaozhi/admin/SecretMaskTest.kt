package com.xiaozhi.admin

import kotlin.test.Test
import kotlin.test.assertEquals

/**
 * 密钥打码口径回归（labeledSecretField 隐藏态的显示字符串）：
 * 头尾各 3 字符保留、中间 `*` 占位（个数 = 被遮字符数）；短密钥整体打码；空串原样。
 */
class SecretMaskTest {

    @Test
    fun longSecretKeepsHeadAndTailThreeCharacters() {
        // 22 字符：保留 "sk-" 与 "xyz"，中间 16 个字符逐位换成 *
        assertEquals("sk-" + "*".repeat(16) + "xyz", maskSecret("sk-abcdef1234567890xyz"))
    }

    @Test
    fun sevenCharacterSecretStillShowsHeadAndTail() {
        // 最短的"头尾各 3 不重叠"长度：中间恰好 1 个 *
        assertEquals("abc*efg", maskSecret("abcdefg"))
    }

    @Test
    fun shortSecretsAreFullyMasked() {
        // 长度 ≤ 6：头尾保留会重叠甚至裸露全部内容 → 全部打码，一个原字符都不显示
        assertEquals("******", maskSecret("aToken")) // 6 字符
        assertEquals("*****", maskSecret("12345")) // 5 字符
        assertEquals("*", maskSecret("x")) // 1 字符
    }

    @Test
    fun emptySecretStaysEmptySoPlaceholderShows() {
        assertEquals("", maskSecret(""))
    }

    @Test
    fun maskingIsDeterministicAndLengthPreserving() {
        val secret = "AIzaSyD-1234567890abcdefGHI"
        val masked = maskSecret(secret)
        assertEquals(secret.length, masked.length)
        assertEquals(secret.take(3), masked.take(3))
        assertEquals(secret.takeLast(3), masked.takeLast(3))
        assertEquals(masked, maskSecret(secret))
    }

    @Test
    fun customKeepWidensTheRevealedWindow() {
        // 12 字符、keep=4：头尾各 4 保留，中间 4 个 *
        assertEquals("abcd****wxyz", maskSecret("abcdefghwxyz", keep = 4))
    }
}
