package com.xiaozhi.admin

import com.gearui.components.toast.Toast
import com.tencent.kuikly.core.pager.Pager

/**
 * 顶部 toast 状态（跨端）：**转发 GearUI 的 `Toast`**，不再自己画浮层。
 *
 * ## 为什么换
 *
 * T06 之前这里有一套自绘实现：DSL 绝对定位 + 深色胶囊 + 语义描边 + 自管 3 秒定时器
 * （`scope.setTimeout`）。换成 Compose 壳之后：
 *
 * - GearUI `App` 已经在根容器挂好 `OverlayRoot` + `ToastHost()`，toast 天然**层级最高**、
 *   不占布局高度、带进出动画，且是单例队列（新的一条顶掉旧的）；
 * - 自绘版本要跟着壳层重写一遍（Compose 版），代码量不减、还多一份视觉口径要维护。
 *
 * ## 语义对齐（与旧实现逐项一致）
 *
 * | 维度 | 旧自绘 | 现在（GearUI Toast） |
 * |---|---|---|
 * | 停留时长 | 3000ms（`TOAST_HOLD_MS`） | 3000ms（显式传入，非默认 2000） |
 * | 层级 | `zIndex(10)`，盖在抽屉（8/9）之上 | Overlay `zIndex = 100`，盖住一切 |
 * | 连续调用 | 新的一条替换旧的（`seq` 号让旧定时器失效） | `Toast` 单例：新的一条直接顶掉旧的 |
 * | 语义 | `ok` / `error` / `info` | `Toast.success` / `Toast.error` / `Toast.info` |
 * | 占高度 | 绝对定位，不占 | Overlay，不占 |
 *
 * ⚠️ 构造参数 `scope: Pager` 保留但**不再使用**：22 处 `ToastState(this)` 调用点是
 * T06–T11 的战场，此刻改签名只制造冲突（与 T05 对 19 个状态类的处置同一口径）。
 */
class ToastState(@Suppress("UNUSED_PARAMETER") scope: Pager) {

    /** 停留时长（毫秒）：与旧实现的 `TOAST_HOLD_MS` 同值，改这里即全站生效。 */
    private val holdMs: Long = TOAST_HOLD_MS

    /**
     * 顶部弹出一条提示，[holdMs] 后自动消失。
     *
     * `level` 沿用旧口径：`ok` = 成功（绿）、`error` = 失败（红）、`warn`/`warning` = 警告（橙）、
     * 其余一律中性提示。调用方**不需要**改动既有文案与 level 传参。
     */
    fun show(message: String, level: String = "info") {
        when (level) {
            "ok" -> Toast.success(message, holdMs)
            "error" -> Toast.error(message, holdMs)
            "warn", "warning" -> Toast.warning(message, holdMs)
            else -> Toast.show(message, holdMs)
        }
    }

    /**
     * 立即清空（页面离开时调用；GearUI 的 toast 是全局单例，不清理会带到下一页）。
     *
     * 目前没有调用点——留给 T07–T11 需要"跳页时收掉提示"的场景，保留一个明确入口
     * 比让各页面直接摸 `Toast.clear()` 更可控。
     */
    fun clear() = Toast.clear()
}

/** toast 停留时长（毫秒）：到点自动消失，不需要用户关闭。与旧实现同值。 */
private const val TOAST_HOLD_MS = 3000L
