package com.xiaozhi.admin

import com.tencent.kuikly.core.base.ViewContainer
import com.tencent.kuikly.core.directives.vif
import com.tencent.kuikly.core.pager.Pager
import com.tencent.kuikly.core.reactive.handler.observable
import com.tencent.kuikly.core.timer.setTimeout
import com.tencent.kuikly.core.views.Text
import com.tencent.kuikly.core.views.View

/**
 * 顶部 toast 数据：level 决定配色——ok=绿（成功）/error=红（失败）/info=中性（提示）。
 *
 * 跨端统一（commonMain）：macOS 壳（AdminShell）与 web 配置页（ConfigPage）共用同一套
 * 状态、配色与渲染，不再各写一套（此前 macOS 在壳内、web 在 DOM 里各实现一次）。
 */
data class ToastData(val msg: String, val level: String = "info")

/**
 * 顶部 toast 状态（跨端）：持有当前 toast 与 3 秒自动消失定时器。
 * 由 Pager 创建（val toast = ToastState(this)），经 [ToastHost] 渲染。
 */
class ToastState(private val scope: Pager) {
    var current by scope.observable<ToastData?>(null)
    private var seq = 0

    /** 顶部弹出一条提示并 3 秒后自动消失；连续调用以最新一条为准（旧定时器失效，不会误清最新 toast）。 */
    fun show(message: String, level: String = "info") {
        val s = ++seq
        current = ToastData(message, level)
        scope.setTimeout(3000) { if (seq == s) current = null }
    }
}

/**
 * 顶部 toast 横幅（跨端）：放在页面顶部（标题栏下方）渲染；
 * 按 level 上色——ok 绿（accentTint）/error 红（dangerTint）/info 中性（insetBg）。
 *
 * 用法：在容器闭包内**非限定**调用 `ToastHost(toast)`（接收者=该容器，节点才能正确挂入）。
 */
fun ViewContainer<*, *>.ToastHost(state: ToastState) {
    vif({ state.current != null }) {
        val t = state.current!!
        View {
            attr {
                flexDirectionRow()
                alignItemsCenter()
                paddingLeft(AdminSpace.xl)
                paddingRight(AdminSpace.xl)
                paddingTop(AdminSpace.sm)
                paddingBottom(AdminSpace.sm)
                backgroundColor(
                    when (t.level) {
                        "ok" -> AdminColors.accentTintBg
                        "error" -> AdminColors.dangerTintBg
                        else -> AdminColors.insetBg
                    },
                )
            }
            Text {
                attr {
                    fontSize(AdminType.caption)
                    fontWeightMedium()
                    color(
                        when (t.level) {
                            "ok" -> AdminColors.accentTintText
                            "error" -> AdminColors.dangerTintText
                            else -> AdminColors.textSecondary
                        },
                    )
                    text(t.msg)
                }
            }
        }
    }
}
