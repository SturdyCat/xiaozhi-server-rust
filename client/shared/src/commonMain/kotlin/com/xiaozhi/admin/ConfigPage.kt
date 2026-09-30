package com.xiaozhi.admin

import com.tencent.kuikly.core.annotations.Page
import com.tencent.kuikly.core.base.ViewBuilder
import com.tencent.kuikly.core.base.ViewContainer
import com.tencent.kuikly.core.pager.Pager
import com.tencent.kuikly.core.reactive.handler.observable
import com.tencent.kuikly.core.views.Scroller
import com.tencent.kuikly.core.views.View

/**
 * 管理后台 · 配置页（config），位于 shared 公共层，跨端复用（web / Android / iOS / OHOS）。
 *
 * 作为 macOS 单窗口壳（AdminShell.kt 的 @Page("router")）内「配置」section 的兜底独立入口：
 * 其它平台没有侧边栏壳，仍可用本页单独打开配置。
 * 逻辑全部下沉到 ConfigFormState（load/save/renderForm），本页只负责壳 + 生命周期。
 */
@Page("config")
class ConfigPage : Pager() {

    val form = ConfigFormState()

    override fun pageDidAppear() {
        super.pageDidAppear()
        form.load(this)
    }

    override fun body(): ViewBuilder {
        // Kuikly DSL 约定：嵌套容器闭包内无法隐式访问 Pager 成员（@DslMarker），
        // 用局部 val ctx 捕获 Pager 取状态；组件扩展则在目标容器闭包内非限定调用。
        val ctx = this
        // 宽屏响应式：以 lambda 传给 renderForm，在 cardGrid 的 vif 闭包内读取 pageViewWidth
        // （响应式字段），窗口 resize 跨过 900 阈值时自动 双列 ⇄ 单列 切换。
        // ⚠️ 不能在 body 顶层一次性求值成 Boolean——构建期求值不会随 resize 重算。
        val wide = { pagerData.pageViewWidth >= 900f }
        return {
            attr {
                flex(1f)
                flexDirectionColumn()
                backgroundColor(AdminColors.windowBg)
            }

            // ⚠️ 非限定调用（接收者=当前容器），组件才能挂进正确的父容器
            largeTitleBar({ "配置" }, trailing = {
                primaryButton(
                    if (ctx.form.saving) "保存中…" else "保存配置",
                    enabled = !ctx.form.saving,
                ) { ctx.form.save(ctx) }
            })

            Scroller {
                attr {
                    flex(1f)
                    // groupedCard 不再自带左右外边距，页面留白由 Scroller 统一提供
                    paddingLeft(AdminSpace.xl)
                    paddingRight(AdminSpace.xl)
                    paddingTop(AdminSpace.xl)
                    paddingBottom(AdminSpace.xxxl)
                }
                // 非限定调用：接收者=Scroller，卡片才能挂进 Scroller
                renderForm(ctx.form, wide)
            }
        }
    }
}
