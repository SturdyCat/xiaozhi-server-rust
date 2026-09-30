package com.xiaozhi.admin

import com.tencent.kuikly.core.annotations.Page
import com.tencent.kuikly.core.base.ViewBuilder
import com.tencent.kuikly.core.base.ViewContainer
import com.tencent.kuikly.core.directives.velse
import com.tencent.kuikly.core.directives.vif
import com.tencent.kuikly.core.module.Module
import com.tencent.kuikly.core.pager.Pager
import com.tencent.kuikly.core.reactive.handler.observable
import com.tencent.kuikly.core.views.Scroller
import com.tencent.kuikly.core.views.Text
import com.tencent.kuikly.core.views.View

/**
 * 管理后台根页面（router）——macOS 单窗口壳。
 *
 * 启动流程：先显示「连接服务器」页（ConnectState，输入地址 → GET /api/config 读配置 →
 * 自动 WS 连接 → 进入主壳）；主壳内 sidebar + 内容区在同一 Pager 内用可观察状态
 * selectedSection 切换「概览 / 测试台 / 配置」三个 section，不 push 新窗口。
 * 视觉走现代 macOS 原生风（Apple HIG），见 AdminTheme.kt 的 token 与组件。
 *
 * 跨端说明：
 * - ConfigFormState / ConfigPage / AdminTheme 在 commonMain（无 macOS 依赖），本壳仅 macOS 注册；
 * - web（h5App）由 server 同域托管、只提供配置功能（ConfigPage 相对路径），不走本壳与连接页；
 * - XiaoZhiModule 由本 Pager 经 createExternalModules() 注册，供 TestBenchState 使用。
 */
@Page("router")
class AdminShell : Pager() {

    val conn = ConnectState()
    val bench = TestBenchState()
    val form = ConfigFormState()
    var selectedSection by observable("home")
    var toast by observable("")

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
        // 宽屏响应式：以 lambda 形式传给各 section，在 cardGrid 的 vif 闭包内读取 pageViewWidth
        // （响应式字段），窗口 resize 跨过 900 阈值时自动 双列 ⇄ 单列 切换。
        // ⚠️ 不能在 body 顶层一次性求值成 Boolean——构建期求值不会随 resize 重算。
        val twoColumn = { pagerData.pageViewWidth >= 900f }
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

                    // ===== 侧边栏（启动即进入主界面：不显示 App 名称标题，导航项直接作为起点） =====
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
                        sidebarItem("概览", { ctx.selectedSection == "home" }) { ctx.selectedSection = "home" }
                        sidebarItem("测试台", { ctx.selectedSection == "testbench" }) { ctx.selectedSection = "testbench" }
                        // 「配置」项：form.dirty 时右侧显示小橙点（warning 色）
                        sidebarItem("配置", { ctx.selectedSection == "config" }, showDot = { ctx.form.dirty }) { ctx.selectedSection = "config" }
                        // 「状态」项：置灰禁用，点击无反应
                        sidebarItem("状态", { ctx.selectedSection == "status" }, enabled = false) { }
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
                    View {
                        attr {
                            flex(1f)
                            flexDirectionColumn()
                            paddingTop(36f)
                        }

                        // 顶栏标题/右侧操作区随 section 响应式切换：
                        // 标题经 largeTitleBar 的 title lambda 在 attr 闭包内读取 selectedSection；
                        // 右侧操作区统一传入，内部用 vif 按 section 显隐（构建期 when 一次性求值不会更新）。
                        largeTitleBar(
                            title = {
                                when (ctx.selectedSection) {
                                    "testbench" -> "测试台"
                                    "config" -> "配置"
                                    else -> "概览"
                                }
                            },
                            trailing = {
                                vif({ ctx.selectedSection == "config" }) {
                                    primaryButton(
                                        if (ctx.form.saving) "保存中…" else "保存配置",
                                        enabled = !ctx.form.saving,
                                    ) {
                                        ctx.form.save(ctx, ctx.conn.baseUrl)
                                        ctx.toast = ctx.form.statusMsg
                                    }
                                }
                                vif({ ctx.selectedSection == "testbench" }) {
                                    statusBadge(ctx.bench.connectionState, connectionLabel(ctx.bench.connectionState))
                                }
                            },
                        )

                        Scroller {
                            attr {
                                flex(1f)
                                paddingLeft(AdminSpace.xl)
                                paddingRight(AdminSpace.xl)
                                paddingTop(AdminSpace.xxl)
                                paddingBottom(AdminSpace.xxxl)
                            }
                            // ⚠️ 非限定调用（接收者=Scroller），卡片/表单才能挂进 Scroller
                            vif({ ctx.selectedSection == "home" }) { homeSection(ctx, twoColumn) }
                            vif({ ctx.selectedSection == "testbench" }) { renderBench(ctx.bench, ctx, twoColumn) }
                            vif({ ctx.selectedSection == "config" }) { renderForm(ctx.form, twoColumn) }
                        }

                        // 底部 toast（全局状态提示）
                        vif({ ctx.toast.isNotEmpty() }) {
                            View {
                                attr {
                                    height(44f)
                                    flexDirectionRow()
                                    alignItemsCenter()
                                    paddingLeft(AdminSpace.xl)
                                    backgroundColor(AdminColors.accentTintBg)
                                }
                                Text {
                                    attr {
                                        fontSize(AdminType.caption)
                                        color(AdminColors.textAccent)
                                        text(ctx.toast)
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // 概览 section 渲染：见文件底部 ViewContainer.homeSection(shell) 扩展
}

// ============================================================
// 文件级：连接状态文案 / 侧边栏项 / 概览 section
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

/**
 * 概览：4 张卡——连接状态 / 最近识别 / TTS 快捷 / 快捷入口。
 * 宽屏（twoColumn）：双列 cardGrid；窄屏：纵向单列。
 *
 * ⚠️ 必须在目标容器（Scroller）闭包内**非限定**调用 `homeSection(ctx, twoColumn)`：
 * Kuikly 的 View{} DSL 静态绑定到词法作用域最近的 ViewContainer 接收者，
 * 以成员函数经 ctx 调用会把卡片挂到 Pager 根容器，导致布局逃逸。
 * AdminCard 的 content（ViewBuilder，带接收者）在 groupedCard 卡片容器内执行，节点挂进卡片。
 */
fun ViewContainer<*, *>.homeSection(shell: AdminShell, wide: () -> Boolean) {
    cardGrid(
        wide = wide,
        cards = listOf(
            AdminCard("连接状态") {
                statusBadge(shell.bench.connectionState, connectionLabel(shell.bench.connectionState))
                View { attr { height(AdminSpace.md) } }
                labeledField("server url", { shell.bench.serverUrl }, { shell.bench.serverUrl = it }, "ws://127.0.0.1:8000/api/ws")
                actionRow {
                    primaryButton(if (shell.bench.connected) "断开" else "连接") {
                        if (shell.bench.connected) shell.bench.disconnect(shell) else shell.bench.connect(shell)
                    }
                }
            },
            AdminCard("最近识别") {
                if (shell.bench.asrText.isEmpty()) {
                    Text {
                        attr {
                            fontSize(AdminType.body)
                            color(AdminColors.textTertiary)
                            text("暂无识别记录")
                        }
                    }
                } else {
                    Text {
                        attr {
                            fontSize(AdminType.body)
                            color(AdminColors.textPrimary)
                            text(shell.bench.asrText)
                        }
                    }
                    View { attr { height(AdminSpace.sm) } }
                    secondaryButton("复制") { shell.toast = "已复制识别结果" }
                }
            },
            AdminCard("TTS 快捷") {
                labeledField("合成文字", { shell.bench.ttsText }, { shell.bench.ttsText = it }, "输入要合成的文字", height = 100f)
                actionRow {
                    primaryButton(
                        if (shell.bench.speaking) "合成中…" else "合成并播放",
                        enabled = !shell.bench.speaking,
                    ) { shell.bench.speak(shell) }
                }
            },
            AdminCard("快捷入口") {
                actionRow {
                    secondaryButton("打开测试台") { shell.selectedSection = "testbench" }
                    View { attr { width(AdminSpace.md) } }
                    secondaryButton("打开配置") { shell.selectedSection = "config" }
                }
            },
        ),
    )
}
