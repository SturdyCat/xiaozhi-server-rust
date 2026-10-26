package com.xiaozhi.admin

import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertTrue

/**
 * `closed{reason, detail}` → 状态行文案的映射（D1 回归测试）。
 *
 * 为什么测这里而不是测 `EchoMuseConsoleState.onClosedFrame`：状态机的构造要 Kuikly 的
 * `Pager.observable`，而本测试模块（`adminCoreTests`）刻意**不引 Kuikly**（只 `kotlin.include`
 * 纯 Kotlin 源码）。所以把"协议字段 → 中文一句话"的查表搬进 [ConsoleHint.closed]（纯函数），
 * 状态机那边只剩"取 reason/detail → 查表 → 写 observable"。这是**搬出纯逻辑**，不是把状态机拆散。
 *
 * 首例是缺陷本体：dial 超时时服务端发
 * `{"type":"closed","reason":"internal","detail":"设备未在 15 秒内回拨 shell 通道"}`
 * （`server/src/device/echomuse/console.rs` 的 `DETAIL_DIAL_TIMEOUT`），
 * 旧代码的 `when(reason)` 没有 `internal` 分支，落进 `else` 变成
 * 「连接中断（服务端已退出或网络中断）」——把"设备没回拨"说成"网络/服务端问题"，
 * 直接把排查方向带偏；而且 detail 整条被丢弃，用户永远看不到真正的原因。
 */
class EchoMuseConsoleClosedTest {

    // ============================================================
    // D1 本体：internal 必须拼 detail
    // ============================================================

    /** dial 超时：detail 必须原样出现在文案里，且带且**只带一次**前缀。 */
    @Test
    fun internal_with_detail_is_prefixed_and_keeps_detail() {
        val detail = "设备未在 15 秒内回拨 shell 通道"
        val actual = ConsoleHint.closed(ConsoleWire.REASON_INTERNAL, detail)

        assertEquals("控制台内部错误：设备未在 15 秒内回拨 shell 通道", actual)
        // 前缀与服务端 `console_frames.rs::COPY_INTERNAL` 逐字一致（改任一侧都是协议破坏）
        assertEquals("控制台内部错误：", ConsoleCopy.INTERNAL_PREFIX)
        // detail 是"裸原因文本"，只能拼一次前缀；拼两次就是双前缀（最容易在修 bug 时写出来的错）
        assertTrue(actual.startsWith(ConsoleCopy.INTERNAL_PREFIX))
        assertFalse(actual.removePrefix(ConsoleCopy.INTERNAL_PREFIX).startsWith(ConsoleCopy.INTERNAL_PREFIX))
        assertFalse(actual.contains(ConsoleCopy.RECONNECT_LOST), "internal 有 detail 时不该退成通用中断文案")
    }

    /** 占槽在回拨前被撤销：另一个 internal detail 也要透传（不只认 dial 超时那一条）。 */
    @Test
    fun internal_passes_through_any_detail() {
        val detail = "设备回拨通道已关闭（占槽在回拨前被撤销）"
        assertEquals(
            ConsoleCopy.INTERNAL_PREFIX + detail,
            ConsoleHint.closed(ConsoleWire.REASON_INTERNAL, detail),
        )
    }

    /** detail 为空（协议异常）时退成通用文案：宁可说"连接中断"，也不显示光有前缀的残句。 */
    @Test
    fun internal_without_detail_falls_back_to_reconnect_lost() {
        assertEquals(
            ConsoleCopy.RECONNECT_LOST,
            ConsoleHint.closed(ConsoleWire.REASON_INTERNAL, ""),
        )
    }

    // ============================================================
    // replaced（新增分支）
    // ============================================================

    /** 槽被非预期接管：文案必须点明"输出已保留"，用户不该为了看报错再连一次。 */
    @Test
    fun replaced_says_output_is_kept() {
        val actual = ConsoleHint.closed(ConsoleWire.REASON_REPLACED, "占槽被非预期接管")
        assertEquals("会话被其他操作接管，已断开（输出已保留）", actual)
        assertEquals(ConsoleCopy.REPLACED, actual)
        assertFalse(actual.contains(ConsoleCopy.SUPERSEDED), "replaced（槽被接管）≠ superseded（设备重连）")
    }

    // ============================================================
    // 其余 reason 逐个对位（防止改 internal 分支时把别的分支挤错位）
    // ============================================================

    @Test
    fun every_reason_maps_to_its_own_copy() {
        // detail 一律给非空值：internal 之外的分支必须忽略它，不能被 detail 影响
        val detail = "不该出现在这些文案里"
        assertEquals(ConsoleCopy.IDLE_TIMEOUT, ConsoleHint.closed(ConsoleWire.REASON_IDLE, detail))
        assertEquals(ConsoleCopy.SESSION_LIMIT, ConsoleHint.closed(ConsoleWire.REASON_LIMIT, detail))
        assertEquals(ConsoleCopy.DEVICE_LOST, ConsoleHint.closed(ConsoleWire.REASON_OFFLINE, detail))
        assertEquals(ConsoleCopy.SUPERSEDED, ConsoleHint.closed(ConsoleWire.REASON_SUPERSEDED, detail))
        assertEquals(ConsoleCopy.SHELL_LOST, ConsoleHint.closed(ConsoleWire.REASON_SHELL_LOST, detail))
        // 保留码：服务端一期不发（设计 §0.3），收到当普通中断处理
        assertEquals(ConsoleCopy.RECONNECT_LOST, ConsoleHint.closed(ConsoleWire.REASON_SERVER_SHUTDOWN, detail))
    }

    /** 未知 / 空 reason（服务端加了新码而前端没跟上）退成通用中断，不能崩、不能空白。 */
    @Test
    fun unknown_and_empty_reason_fall_back_to_reconnect_lost() {
        assertEquals(ConsoleCopy.RECONNECT_LOST, ConsoleHint.closed("client-closed", ""))
        assertEquals(ConsoleCopy.RECONNECT_LOST, ConsoleHint.closed("brand-new-reason", ""))
        assertEquals(ConsoleCopy.RECONNECT_LOST, ConsoleHint.closed("", ""))
    }

    /** 除 `internal` 外没有任何一支会把 detail 漏进文案（漏了会显示"不该出现在这些文案里"）。 */
    @Test
    fun only_internal_uses_detail() {
        val probe = "PROBE-DETAIL"
        val reasons = listOf(
            ConsoleWire.REASON_IDLE,
            ConsoleWire.REASON_LIMIT,
            ConsoleWire.REASON_OFFLINE,
            ConsoleWire.REASON_SUPERSEDED,
            ConsoleWire.REASON_SHELL_LOST,
            ConsoleWire.REASON_REPLACED,
            ConsoleWire.REASON_SERVER_SHUTDOWN,
        )
        for (reason in reasons) {
            assertFalse(
                ConsoleHint.closed(reason, probe).contains(probe),
                "reason=$reason 的文案不该包含 detail",
            )
        }
        assertTrue(ConsoleHint.closed(ConsoleWire.REASON_INTERNAL, probe).contains(probe))
    }

    // ============================================================
    // 字面量漂移守卫（与服务端 `CloseReason::code()` 逐字对齐）
    // ============================================================

    /**
     * `closed.reason` 八个值 + 仅审计用的 `client-closed`：`ConsoleWire` 里的常量必须逐个对上，
     * 且**互不相同**（两个 reason 撞成同一个字符串，前端就会把两种失败显示成同一句话）。
     */
    @Test
    fun reason_literals_match_server_close_reason_codes() {
        val reasons = listOf(
            ConsoleWire.REASON_IDLE to "idle-timeout",
            ConsoleWire.REASON_LIMIT to "session-limit",
            ConsoleWire.REASON_OFFLINE to "device-offline",
            ConsoleWire.REASON_SUPERSEDED to "superseded",
            ConsoleWire.REASON_SHELL_LOST to "device-shell-lost",
            ConsoleWire.REASON_REPLACED to "replaced",
            ConsoleWire.REASON_SERVER_SHUTDOWN to "server-shutdown",
            ConsoleWire.REASON_INTERNAL to "internal",
        )
        for ((actual, expected) in reasons) assertEquals(expected, actual)
        assertEquals(reasons.size, reasons.map { it.first }.toSet().size, "reason 字面量不许重复")
    }

    /** `closed.reason=internal` 与 `error.code=internal` 共用 `"internal"`（服务端两处同值）。 */
    @Test
    fun internal_literal_is_shared_by_error_code_and_close_reason() {
        assertEquals("internal", ConsoleWire.CODE_INTERNAL)
        assertEquals(ConsoleWire.CODE_INTERNAL, ConsoleWire.REASON_INTERNAL)
        // `device-offline` 同理（error.code 与 closed.reason 同名，语义不同，别合并分支）
        assertEquals("device-offline", ConsoleWire.CODE_DEVICE_OFFLINE)
        assertEquals(ConsoleWire.CODE_DEVICE_OFFLINE, ConsoleWire.REASON_OFFLINE)
    }
}
