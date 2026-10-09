package com.xiaozhi.admin

import com.tencent.kuikly.core.annotations.Page
import com.tencent.kuikly.core.base.ViewBuilder
import com.tencent.kuikly.core.base.ViewContainer
import com.tencent.kuikly.core.directives.velse
import com.tencent.kuikly.core.directives.vif
import com.tencent.kuikly.core.module.Module
import com.tencent.kuikly.core.pager.Pager
import com.tencent.kuikly.core.reactive.handler.observable
import com.tencent.kuikly.core.views.Text
import com.tencent.kuikly.core.views.View

/**
 * 管理后台根页面（router）——macOS 单窗口壳。
 *
 * 启动流程：先显示「连接服务器」页（ConnectState，输入地址 → GET /api/config 读配置 →
 * 自动 WS 连接 → 进入主壳）；主壳内 sidebar + 内容区在同一 Pager 内用可观察状态
 * selectedSection 切换「测试台 / 配置」两个 section，不 push 新窗口。
 * 两个 section 均为官方 Tabs + PageList 标签页布局（tabbedPanel）：
 * 测试台四个 tab（连接 / ASR 识别 / TTS 合成 / LLM 对话），每个 tab 左侧功能卡、
 * 右侧对应配置卡（改动即存表单，配合顶栏「保存配置」一键写回）；配置页六个 tab 保持全量配置汇总。
 * 视觉走现代 macOS 原生风（Apple HIG），见 AdminTheme.kt 的 token 与组件。
 *
 * 跨端说明：
 * - ConfigFormState / ConfigPage / AdminTheme 在 commonMain（无 macOS 依赖），本壳仅 macOS 注册；
 * - web（h5App）由 server 同域托管、只提供配置功能（ConfigPage 相对路径），不走本壳与连接页；
 * - XiaoZhiModule 由本 Pager 经 createExternalModules() 注册，供 TestBenchState 使用。
 */
@Page("router")
class AdminShell : Pager() {

    val conn = ConnectState(this)
    val bench = TestBenchState(this)
    val form = ConfigFormState(this)

    /** 能力总览的数据源（`GET /api/plugins`，只读）。 */
    val plugins = PluginMetaState(this)
    var selectedSection by observable("testbench")

    /** 顶部 toast（跨端统一实现：状态 + 渲染见 commonMain 的 ToastState / ToastHost）。 */
    val toast = ToastState(this)

    /** 顶部弹出一条提示（成功=ok / 失败=error / 提示=info），3 秒后自动消失。 */
    fun showToast(message: String, level: String = "info") = toast.show(message, level)

    override fun createExternalModules(): Map<String, Module> {
        return mapOf(XiaoZhiModule.MODULE_NAME to XiaoZhiModule())
    }

    override fun pageDidAppear() {
        super.pageDidAppear()
        // 启动连接流程：读记忆地址 → （有记忆则）自动 读配置 + 连 WS；配置表单由该流程填充
        // （不再用相对路径 form.load：macOS 无同域，相对 URL 请求必败）
        conn.onLaunch(this)
    }

    override fun body(): ViewBuilder {
        // Kuikly DSL 约定：View{} 等组件是 ViewContainer 的扩展函数，挂到「词法作用域最近的接收者」。
        // 嵌套容器闭包内无法隐式访问 Pager 成员（@DslMarker），故用局部 val ctx 捕获 Pager；
        // 但 AdminTheme 组件与 section 渲染扩展必须在目标容器闭包内「非限定」调用，
        // 让接收者=目标容器（若写 ctx.xxx() 会把节点挂到根容器，布局逃逸）。
        val ctx = this
        // 宽屏响应式：以 lambda 形式传给各 section，在 twoPane 的 vif 闭包内读取 pageViewWidth
        // （响应式字段），窗口 resize 跨过 900 阈值时自动 并排 ⇄ 堆叠 切换。
        // ⚠️ 不能在 body 顶层一次性求值成 Boolean——构建期求值不会随 resize 重算。
        // ⚠️ 这些 lambda 必须定义在 body 顶层（return 块外）：DslMarker 会屏蔽嵌套 builder
        // 闭包内对 Pager 成员的隐式接收者访问（pagerData 报 "cannot be called in this context"）。
        val twoColumn = { pagerData.pageViewWidth >= 900f }
        // pageItem 尺寸（官方 PageList 要求显式设置，见 tabbedPanel 注释）：
        // 宽 = 窗口宽 - 侧边栏(240) - 分隔(1)；高 = 窗口高 - 顶避让(36) - 标题栏(64) - 标题分隔(1) - tab栏(44)
        val contentPageWidth = { pagerData.pageViewWidth - 241f }
        val contentPageHeight = { pagerData.pageViewHeight - 145f }
        return {
            attr {
                flex(1f)
                backgroundColor(AdminColors.windowBg)
            }

            // ===== 分支一：启动连接页（stage=connect；成功/跳过后切 main）=====
            vif({ ctx.conn.stage == "connect" }) {
                renderConnect(ctx.conn, ctx)
            }
            velse {
                // ===== 分支二：主壳（侧边栏 + 内容区）=====
                View {
                    attr {
                        flex(1f)
                        flexDirectionRow()
                    }

                    // ===== 侧边栏（导航项直接作为起点，无 App 名称标题）=====
                    // ⚠️ paddingTop = 标题栏避让区：macOS 红黄绿窗口按钮浮在窗口左上角（约 y=12、高 12），
                    //    本壳隐藏了系统导航栏（SceneDelegate→KuiklyRenderViewController setNavigationBarHidden:YES），
                    //    故内容直接铺满窗口；预留 36pt 顶距让首个导航项避开窗口按钮，避免重叠。
                    View {
                        attr {
                            width(240f)
                            flexDirectionColumn()
                            backgroundColor(AdminColors.sidebarBg)
                            paddingTop(36f)
                        }
                        // ⚠️ 非限定调用：接收者=侧边栏 View，导航项才能挂进侧边栏（经 ctx. 调用会逃逸到根容器）
                        // ⚠️ 选中态传 lambda（在 attr/vif 闭包内读取 observable），点击后高亮才能响应式更新
                        sidebarItem("测试台", { ctx.selectedSection == "testbench" }) { ctx.selectedSection = "testbench" }
                        // 「配置」项：form.dirty 时右侧显示小橙点（warning 色）
                        sidebarItem("配置", { ctx.selectedSection == "config" }, showDot = { ctx.form.dirty }) { ctx.selectedSection = "config" }
                        // 底部弹性占位 + 切换服务器（回到启动连接页，本地/远程调试切换入口）+ 版本信息
                        View { attr { flex(1f) } }
                        sidebarItem("切换服务器", { false }) { ctx.conn.backToConnect(ctx) }
                        Text {
                            attr {
                                fontSize(AdminType.micro)
                                color(AdminColors.textTertiary)
                                marginLeft(16f)
                                marginBottom(16f)
                                text("v1.0 · macOS")
                            }
                        }
                    }

                    // 侧边栏右边 1px 分隔
                    View {
                        attr {
                            width(1f)
                            backgroundColor(AdminColors.divider)
                        }
                    }

                    // ===== 内容区 =====
                    // ⚠️ 同侧边栏，预留 36pt 顶距避让 macOS 窗口按钮，使顶栏与侧边栏顶端对齐、不压住红黄绿。
                    // section 内容为 tabbedPanel（Tabs+PageList，flex(1)），不再套页面级 Scroller——
                    // 每个 tab 页内自带纵向 Scroller。
                    View {
                        attr {
                            flex(1f)
                            flexDirectionColumn()
                            paddingTop(36f)
                        }

                        // 顶栏标题/右侧操作区随 section 响应式切换：
                        // 标题经 largeTitleBar 的 title lambda 在 attr 闭包内读取 selectedSection；
                        // 测试台各 tab 内嵌了对应配置卡，「保存配置」两个 section 都要可用。
                        largeTitleBar(
                            title = {
                                if (ctx.selectedSection == "config") "配置" else "测试台"
                            },
                            trailing = {
                                vif({ ctx.selectedSection == "config" && ctx.form.conflict }) {
                                    // 过期写入（409）：草稿保留，给一个「重新加载」出口
                                    secondaryButton("重新加载") {
                                        ctx.form.reload(ctx, ctx.conn.baseUrl)
                                    }
                                    View { attr { width(AdminSpace.sm) } }
                                }
                                vif({ ctx.selectedSection == "config" || ctx.selectedSection == "testbench" }) {
                                    // 三态按钮：saving 时菊花 + 灰底 + 拦截点击（loading 在 attr/event 闭包内实时读取，保存期间不会重复提交）。
                                    primaryButton(
                                        "保存配置",
                                        loading = { ctx.form.saving },
                                        loadingText = "保存中…",
                                    ) {
                                        ctx.form.save(ctx, ctx.conn.baseUrl)
                                        ctx.showToast(ctx.form.statusMsg, ctx.form.statusLevel)
                                    }
                                }
                            },
                        )

                        // 顶部 toast（跨端统一：成功=绿 / 失败=红 / 信息=中性），3 秒自动消失
                        ToastHost(ctx.toast)

                        // section 切换：vif 闭包内读取 selectedSection（observable），切换时重建对应 tabbedPanel
                        //（tab 选中态等存于各状态类的 TabUiState，重建不丢）
                        vif({ ctx.selectedSection == "testbench" }) {
                            renderBench(
                                ctx.bench, ctx.form, ctx, twoColumn,
                                pageWidth = contentPageWidth,
                                pageHeight = contentPageHeight,
                            )
                        }
                        vif({ ctx.selectedSection == "config" }) {
                            renderForm(
                                ctx.form,
                                ctx.plugins,
                                ctx,
                                pageWidth = contentPageWidth,
                                pageHeight = contentPageHeight,
                            )
                        }
                    }
                }
            }
        }
    }
}

// ============================================================
// 文件级：连接状态文案 / 侧边栏项
// ============================================================

fun connectionLabel(state: String): String = when (state) {
    "connected" -> "已连接"
    "connecting" -> "连接中"
    "error" -> "连接错误"
    else -> "未连接"
}

/**
 * 侧边栏项：高 44（iPad 触摸目标 ≥44）、左右内距 12、左右外距 8。
 * 选中 = 亮灰胶囊 sidebarSelectedBg + 文字 accentTintText（品牌绿）+ 左侧 3px accent 竖条；
 * 普通 = 透明底 textPrimary(body 17/500)；disabled 置灰 textTertiary 且无点击；
 * showDot() 为 true 时右侧显示小橙点（warning 色，用于「配置」项 dirty 提示）。
 *
 * ⚠️ selected/showDot 必须是 lambda 且只在 attr/vif 闭包内求值：
 * Kuikly 仅对闭包内的 observable 读取做响应式跟踪；构建期传入的一次性 Boolean
 * 在 selectedSection 变化后不会重算（这就是此前「点击菜单无选中效果」的根因）。
 */
fun ViewContainer<*, *>.sidebarItem(
    label: String,
    isSelected: () -> Boolean,
    enabled: Boolean = true,
    showDot: () -> Boolean = { false },
    onClick: () -> Unit,
) {
    View {
        attr {
            height(44f)
            flexDirectionRow()
            alignItemsCenter()
            marginLeft(AdminSpace.sm)
            marginRight(AdminSpace.sm)
            paddingLeft(AdminSpace.md)
            paddingRight(AdminSpace.md)
            borderRadius(AdminShape.radiusMd)
            backgroundColor(if (isSelected()) AdminColors.sidebarSelectedBg else AdminColors.transparent)
        }
        event { click { if (enabled) onClick() } }
        // 左侧选中指示条（3px 品牌绿竖条，vif 响应式）
        vif({ isSelected() }) {
            View {
                attr {
                    width(3f)
                    height(20f)
                    borderRadius(AdminShape.radiusPill)
                    backgroundColor(AdminColors.accent)
                    marginRight(AdminSpace.sm)
                }
            }
        }
        Text {
            attr {
                flex(1f)
                fontSize(AdminType.body)
                fontWeightMedium()
                color(
                    if (!enabled) {
                        AdminColors.textTertiary
                    } else if (isSelected()) {
                        AdminColors.accentTintText
                    } else {
                        AdminColors.textPrimary
                    },
                )
                text(label)
            }
        }
        vif({ showDot() }) {
            View {
                attr {
                    width(8f)
                    height(8f)
                    borderRadius(AdminShape.radiusPill)
                    backgroundColor(AdminColors.warning)
                }
            }
        }
    }
}
