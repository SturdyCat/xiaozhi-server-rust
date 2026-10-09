package com.xiaozhi.admin

import com.tencent.kuikly.core.annotations.Page
import com.tencent.kuikly.core.base.ViewBuilder
import com.tencent.kuikly.core.base.ViewContainer
import com.tencent.kuikly.core.directives.velse
import com.tencent.kuikly.core.directives.vif
import com.tencent.kuikly.core.pager.Pager
import com.tencent.kuikly.core.reactive.handler.observable
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

    val form = ConfigFormState(this)

    /** 能力总览的数据源（`GET /api/plugins`，只读，不随表单回传）。 */
    val plugins = PluginMetaState(this)

    /** 顶部 toast（跨端统一实现：状态 + 渲染见 commonMain 的 ToastState / ToastHost）。 */
    val toast = ToastState(this)

    fun showToast(message: String, level: String = "info") = toast.show(message, level)

    override fun pageDidAppear() {
        super.pageDidAppear()
        form.load(this)
        plugins.load(this)
    }

    override fun body(): ViewBuilder {
        // Kuikly DSL 约定：嵌套容器闭包内无法隐式访问 Pager 成员（@DslMarker），
        // 用局部 val ctx 捕获 Pager 取状态；组件扩展则在目标容器闭包内非限定调用。
        val ctx = this
        // pageItem 尺寸（官方 PageList 要求显式设置，见 tabbedPanel 注释）：
        // 宽 = 窗口宽；高 = 窗口高 - 标题栏(64) - 分隔(1) - tab栏(44)。
        // ⚠️ lambda 定义在 body 顶层：DslMarker 会屏蔽 return 块内嵌套闭包对 Pager 成员的隐式访问。
        val pageWidth = { pagerData.pageViewWidth }
        val pageHeight = { pagerData.pageViewHeight - 109f }
        return {
            attr {
                flex(1f)
                flexDirectionColumn()
                backgroundColor(AdminColors.windowBg)
            }

            // ⚠️ 非限定调用（接收者=当前容器），组件才能挂进正确的父容器
            largeTitleBar({ "配置" }, trailing = {
                // 过期写入（409）后：草稿仍在本地，给一个「重新加载」出口（对齐 DSH 的冲突文案口径）
                vif({ ctx.form.conflict }) {
                    secondaryButton("重新加载") {
                        ctx.form.reload(ctx)
                    }
                    View { attr { width(AdminSpace.sm) } }
                }
                // 三态按钮：saving 时菊花 + 灰底 + 拦截点击，保存期间不会重复提交。
                primaryButton(
                    "保存配置",
                    loading = { ctx.form.saving },
                    loadingText = "保存中…",
                ) {
                    ctx.form.save(ctx)
                    // 保存结果经统一 toast 顶部弹出（成功=绿 / 失败=红）
                    ctx.showToast(ctx.form.statusMsg, ctx.form.statusLevel)
                }
            })

            // 顶部 toast（跨端统一），3 秒自动消失
            ToastHost(ctx.toast)

            // 配置标签页（tabbedPanel：官方 Tabs+PageList；页签由 ConfigSections 常量表驱动）
            renderForm(
                ctx.form,
                ctx.plugins,
                ctx,
                pageWidth = pageWidth,
                pageHeight = pageHeight,
            )
        }
    }
}
