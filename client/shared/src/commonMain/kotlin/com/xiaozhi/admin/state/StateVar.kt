package com.xiaozhi.admin.state

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.snapshots.SnapshotStateList
import androidx.compose.runtime.setValue
import kotlin.properties.ReadWriteProperty
import kotlin.reflect.KProperty

/**
 * Compose 快照版的字段委托 —— 状态类里 `var x by stateVar(初始值)` 的唯一出处。
 *
 * ## 为什么不用 `PagerScope.observable`
 *
 * `core.reactive.handler` 包里的 `observable` 驱动的是 **Kuikly DSL 的 attr 重算**，
 * Compose 的组合（recomposition）完全看不见它：写进去的值不会让任何 `@Composable` 重组。
 * 2.30.0 起该 API 已标 `@Deprecated`（"use PagerScope.observable(mode, value)"），
 * 本项目在其上留了 411 处声明，全量改造即 U2。
 *
 * ## 与旧机制逐项等价
 *
 * | 维度 | 旧 `by scope.observable(v)` | 新 `by stateVar(v)` |
 * |---|---|---|
 * | 声明类型 | 由 `v` 推断 | **不变** |
 * | 默认值 | 求值一次、构造期 | **不变**（同样在构造期） |
 * | 读 | 返回当前值 | 返回 `MutableState.value`，**同时登记快照读取** |
 * | 写 | 覆盖 | 写 `MutableState.value`，**同时通知快照** |
 * | 生命周期 | 页面生命周期 | **不变**（委托随状态实例存活） |
 *
 * ## 为什么不包 `remember`
 *
 * 这些字段声明在**普通类**（`ConfigFormState` / `OpsState` …）里，实例由页面构造一次、
 * 整页生命周期内身份不变，不需要（也不能）进组合。`remember` 只能用在 `@Composable` 内。
 *
 * ⚠️ 字段名、声明类型、默认值、方法签名一律不动；唯一允许的类型改动是
 * `ObservableList<T>` → `MutableList<T>`（见 [stateListOf]）。
 */
fun <T> stateVar(initial: T): ReadWriteProperty<Any?, T> = StateVarDelegate(initial)

/**
 * Kuikly `observableList()` 的 Compose 等价物。
 *
 * `SnapshotStateList<T>` 实现 `MutableList<T>`，且每次读/写都进快照系统 —— 正是 Compose 需要的。
 * 旧代码对 `ObservableList` 的 `add/removeAt/clear/set/indexOf/size/iterator` 用法在其上全部有效。
 *
 * ⚠️ 声明处写成 `val rows: MutableList<T> = stateListOf()`（暴露可变接口）或
 * `val rows = stateListOf<T>()`（推断为 `SnapshotStateList<T>`）都对；
 * 前者与原 `val rows: ObservableList<T> by scope.observableList()` 的可调用面一致。
 */
fun <T> stateListOf(): SnapshotStateList<T> = mutableStateListOf()

/** [stateVar] 的委托实现：一个字段一个 [mutableStateOf]，不再持有 `PagerScope`。 */
private class StateVarDelegate<T>(initial: T) : ReadWriteProperty<Any?, T> {

    private val state = mutableStateOf(initial)

    override fun getValue(thisRef: Any?, property: KProperty<*>): T = state.value

    override fun setValue(thisRef: Any?, property: KProperty<*>, value: T) {
        state.value = value
    }
}
