package com.xiaozhi.admin.contracts

/**
 * 纯 UI 契约：无 Kuikly 上下文、无网络、无 JSON、无插件宿主、无原生桥。
 *
 * ⚠️ 本文件必须**保持零 import**：`:adminCoreTests` 通过 `kotlin.include` 白名单单独编译它，
 * 而该模块的类路径上没有 Kuikly（也不需要）。新增内容若引入依赖，须先确认不破坏这条约束。
 */
sealed class Availability {
    object Enabled : Availability()
    data class Disabled(val reason: String) : Availability() {
        init { require(reason.isNotBlank()) { "Unavailable actions need an explanation" } }
    }
}

/** Three operation phases; a completed operation has a separate success/failure outcome. */
sealed class ActionState {
    object Idle : ActionState()
    data class Running(val requestId: Long) : ActionState()
    data class Finished(val outcome: ActionOutcome) : ActionState()
}

sealed class ActionOutcome {
    data class Success(val message: String = "") : ActionOutcome()
    data class Failure(val message: String, val retryable: Boolean = true) : ActionOutcome()
}

fun canInvokeAction(availability: Availability, state: ActionState): Boolean =
    availability is Availability.Enabled && state !is ActionState.Running

/** Selection state is not an async action, plugin runtime phase, or Boolean override. */
enum class CheckState {
    Unchecked, Checked, Indeterminate;

    fun toggled(): CheckState = if (this == Checked) Unchecked else Checked
    fun booleanOrNull(): Boolean? = when (this) {
        Unchecked -> false
        Checked -> true
        Indeterminate -> null
    }
}

/** Null means inherit, not unchecked. Never use CheckState to serialize device overrides. */
enum class OverrideChoice {
    Inherit, ForceOn, ForceOff;

    fun booleanOrNull(): Boolean? = when (this) {
        Inherit -> null
        ForceOn -> true
        ForceOff -> false
    }
}

enum class IconToken {
    Save, Delete, Menu, ChevronDown, ChevronUp, ChevronRight, Play, Stop, Refresh,
    Check, Close, Settings, Microphone, Waveform, Text, Intent, Model, Speaker, Info,
    Wifi, Bluetooth, Power, Copy, Funnel, Folder, Document,
}

/** Read-only node tests only. Does not replace BLE write/session replay protections. */
class NodeRequestTicket internal constructor(
    val generation: Long,
    val requestId: Long,
    val actionId: String,
)

/**
 * Owned by the context-thread controller. Server/token/target changes invalidate the scope.
 * A stale HTTP response or result-reset timer must not finish/reset a newer operation.
 * Invalidation does not claim that an already-sent remote operation has been cancelled.
 */
class NodeRequestGate {
    private var generation = 0L
    private var sequence = 0L
    private val active = mutableMapOf<String, Long>()
    private val latest = mutableMapOf<String, Long>()

    fun begin(actionId: String): NodeRequestTicket? {
        require(actionId.isNotBlank())
        if (active.containsKey(actionId)) return null
        val id = ++sequence
        active[actionId] = id
        latest[actionId] = id
        return NodeRequestTicket(generation, id, actionId)
    }

    fun isActive(ticket: NodeRequestTicket): Boolean =
        ticket.generation == generation && active[ticket.actionId] == ticket.requestId

    fun isLatest(ticket: NodeRequestTicket): Boolean =
        ticket.generation == generation && latest[ticket.actionId] == ticket.requestId

    fun finish(ticket: NodeRequestTicket): Boolean {
        if (!isActive(ticket)) return false
        active.remove(ticket.actionId)
        return true
    }

    fun invalidate() {
        generation++
        active.clear()
        latest.clear()
    }
}
