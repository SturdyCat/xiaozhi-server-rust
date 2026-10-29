package com.xiaozhi.admin.contracts

import com.xiaozhi.admin.state.stateVar

/**
 * 字段帮助（ⓘ）状态契约 —— 组件层与状态层之间唯一的耦合点。
 *
 * ## 为什么在 contracts 里
 *
 * `kit/`（GearUI 零件库）要渲染 ⓘ 徽标与帮助弹层，但它不能反向依赖 `ui/`（T12 要删 `ui/`），
 * 也不该依赖某个具体状态类。所以「读三个值 + 两个动作」抽成接口放在契约层：
 *
 * ```
 * contracts.AdminFieldHelpState  ←──  kit（只读接口，经 LocalFieldHelp 下发）
 *          ▲
 *          │ 实现
 * contracts.FieldHelpState       ←──  AdminPage / ConfigPage（各持一份，页面级唯一）
 * ```
 *
 * ## 语义（与旧 `ui/FieldHelp.kt` 的 `FieldHelpState` 完全一致）
 *
 * - 一次写入三值（`title` / `text` / `visible`）——避免"先显示旧正文再换标题"的中间态闪烁；
 * - 同一时刻只有一个弹层（页面级唯一实例，徽标只负责写，不负责持有）；
 * - `visible` 是显隐的**唯一真源**，弹层自身不存本地可见状态。
 *
 * ## 与旧实现的唯一差异
 *
 * 旧类构造参数是 `PagerScope`（因为 `observable` 必须建在 Pager 上）；Compose 快照状态是
 * 普通堆对象，不需要 Pager，故构造参数消失，改成无参 `FieldHelpState()`。
 * 三个字段的**类型、默认值、读写语义、方法签名一字未改**。
 */
interface AdminFieldHelpState {
    /** 弹层是否显示（唯一真源）。 */
    val visible: Boolean

    /** 弹层标题 = 字段中文名。 */
    val title: String

    /** 弹层正文 = 字段作用详解（可多行）。 */
    val text: String

    /** 徽标点击入口：一次写入三值，避免中间态闪烁。 */
    fun show(title: String, text: String)

    /** 关闭弹层（蒙层点击 / 「知道了」）。 */
    fun dismiss()
}

/**
 * 字段帮助状态的默认实现：三个 Compose 快照字段。
 *
 * 页面持有实例（`val fieldHelp = FieldHelpState()`）；字段写入即刻进快照系统，
 * GearUI 组件在组合期读到即会随写入重组——这正是旧 `observable` 做不到的事。
 */
class FieldHelpState : AdminFieldHelpState {

    /** 是否显示帮助弹层。 */
    override var visible by stateVar(false)

    /** 弹层标题 = 字段中文名。 */
    override var title by stateVar("")

    /** 弹层正文 = 字段作用详解（可多行）。 */
    override var text by stateVar("")

    override fun show(title: String, text: String) {
        this.title = title
        this.text = text
        visible = true
    }

    override fun dismiss() {
        visible = false
    }
}

/**
 * 页面提供字段帮助状态的入口（原 `ui/FieldHelp.kt:37` 的接口，T12 清理步骤 2 迁入契约层）。
 *
 * ## 为什么与 [FieldHelpState] 同文件
 *
 * 接口（组件层只读视图）与默认实现（页面持有的快照状态）是同一份契约的两面，
 * 拆成两个文件只会让"改一个字段要开两处"。接口**零 Kuikly 类型**（只有 Boolean/String
 * 与方法签名），放 contracts 不引入任何渲染依赖；旧 DSL 渲染侧（`infoBadge` /
 * `fieldHelpDialog` / `fieldHelpState`）已随 Compose 迁移失去全部调用方，
 * `ui/FieldHelp.kt` 其余部分随之删除，不再搬移。
 *
 * 实现方：[AdminPage] / [ConfigPage]（`val fieldHelp = FieldHelpState()`，页面级唯一，
 * 经 `kit/GearFieldHelp.kt` 的 `LocalFieldHelp` 下发给组件层）。
 */
interface FieldHelpHost {
    /** 本页的字段帮助状态（弹层显隐/标题/正文的唯一真源）。 */
    val fieldHelp: FieldHelpState
}
