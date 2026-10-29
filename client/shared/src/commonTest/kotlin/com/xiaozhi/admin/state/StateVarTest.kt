package com.xiaozhi.admin.state

import androidx.compose.runtime.snapshots.Snapshot
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNotEquals
import kotlin.test.assertTrue

/**
 * 状态机制的**纯逻辑**验证（T05 垂直切片的可执行部分）。
 *
 * ## 这里证明什么、不证明什么
 *
 * 证明（在 JVM 单测里直接跑，无真机、无组合环境）：
 *
 * 1. `stateVar` 委托的读/写语义与旧 `observable` 逐项等价（同类型、同默认值、写后可读）；
 * 2. 字段的**读**会被 Compose 快照系统登记（== `@Composable` 能建立依赖，值变才重组）；
 * 3. 字段的**写**会被快照系统登记为"写过的状态对象"（== 读它的组合会被标记失效）；
 * 4. `stateListOf()` 的列表加减改删同样进快照系统，且 `MutableList` 接口面完整。
 *
 * 不证明：**视觉上的重组效果**（"改了 greeting，界面真的重画了"）。那需要一个跑起来的
 * 组合环境（四端宿主 + 页面），本环境不具备，属「未执行/需本地验证」项——
 * 代码级等价性在这里钉死，渲染见 T06 起的 Compose 壳层。
 *
 * ## 与真实状态类的对应关系
 *
 * [WakeupProbe] 与 `ConfigWakeupState.kt` 的 `WakeupSectionState` 三个字段同名、同类型、
 * 同默认值（`enabled="true"` / `greeting="我在"` / `persist="true"`）。真实类需要 `PagerScope`
 * 与 Kuikly 的 `JSONObject`（JVM 单测里没有），故这里用同形探针验证**机制**本身；
 * 迁移后的真实字段用的是同一个 `stateVar` 委托，差别只在构造参数。
 */
class StateVarTest {

    /** 与 `WakeupSectionState` 三个字段同形（T05 的垂直切片对象）。 */
    private class WakeupProbe {
        var enabled by stateVar("true")
        var greeting by stateVar("我在")
        var persist by stateVar("true")
    }

    @Test
    fun fieldDefaultsMatchTheLegacyDeclarations() {
        val probe = WakeupProbe()
        // 默认值在构造期求值一次：与旧 `by scope.observable("我在")` 完全一致
        assertEquals("true", probe.enabled)
        assertEquals("我在", probe.greeting)
        assertEquals("true", probe.persist)
    }

    @Test
    fun writeIsReadBackAndOtherFieldsAreUntouched() {
        val probe = WakeupProbe()
        probe.greeting = "我在呢"
        assertEquals("我在呢", probe.greeting)
        // 单字段独立：改一个不影响另一个（旧机制下也是各自一个 ObservableProperty）
        assertEquals("true", probe.enabled)
        assertEquals("true", probe.persist)
    }

    @Test
    fun fieldIsRegisteredInTheSnapshotSystemOnReadAndWrite() {
        val probe = WakeupProbe()
        val readObjects = mutableSetOf<Any>()
        val writtenObjects = mutableSetOf<Any>()

        Snapshot.observe(
            readObserver = { readObjects += it },
            writeObserver = { writtenObjects += it },
        ) {
            val before = probe.greeting
            probe.greeting = "我在呀"
            assertNotEquals(before, probe.greeting)
        }

        // 读被登记 → Compose 的 @Composable 读同一个字段时会建立依赖
        assertTrue(
            readObjects.isNotEmpty(),
            "读取 stateVar 字段必须登记到快照系统，否则 Compose 建立不了依赖（不会重组）",
        )
        // 写被登记 → 依赖它的组合会被标记失效
        assertTrue(
            writtenObjects.isNotEmpty(),
            "写入 stateVar 字段必须登记到快照系统，否则 Compose 不会失效重算",
        )
        // 读到的和写进去的是同一个状态对象（不是两份副本）
        assertEquals(readObjects, writtenObjects, "读与写必须落在同一个快照状态对象上")
    }

    @Test
    fun listMutationsAreRecordedAndMutableListSurfaceIsComplete() {
        val rows = stateListOf<String>()
        val writtenObjects = mutableSetOf<Any>()

        Snapshot.observe(writeObserver = { writtenObjects += it }) {
            rows.add("a")
            rows.add("b")
            rows.add(0, "c")
            rows[1] = "d"
            rows.removeAt(2)
        }

        assertEquals(listOf("c", "d"), rows.toList())
        assertEquals(2, rows.size)
        assertTrue(rows.contains("d"))
        assertTrue(
            writtenObjects.isNotEmpty(),
            "列表变更必须登记到快照系统，否则 Compose 里的列表读不会刷新",
        )

        // 旧 ObservableList 的常用面：清空 + 批量替换（refresh() 的写法）
        rows.clear()
        rows.addAll(listOf("x", "y"))
        assertEquals(2, rows.size)
        assertEquals("y", rows[1])
    }

    @Test
    fun delegatedListFieldKeepsIdentityAcrossWrites() {
        // 真实状态类里列表是 `val rows: MutableList<T> = stateListOf()`：
        // 实例身份不变、内容可变，与旧 `val rows: ObservableList<T> by observableList()` 一致。
        val holder = object {
            val rows: MutableList<Int> = stateListOf()
        }
        val identity = holder.rows
        holder.rows.add(1)
        holder.rows.add(2)
        assertTrue(identity === holder.rows, "列表字段是 val：多次读取必须返回同一个实例")
        assertEquals(listOf(1, 2), holder.rows.toList())
    }
}
