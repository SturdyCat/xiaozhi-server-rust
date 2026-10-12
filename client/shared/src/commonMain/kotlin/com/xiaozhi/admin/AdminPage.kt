package com.xiaozhi.admin

import com.tencent.kuikly.core.annotations.Page
import com.tencent.kuikly.core.base.ViewBuilder
import com.tencent.kuikly.core.directives.vif
import com.tencent.kuikly.core.pager.Pager
import com.tencent.kuikly.core.reactive.handler.observable
import com.tencent.kuikly.core.views.Text
import com.tencent.kuikly.core.views.View

/**
 * 管理后台根页面（`page_name=admin`）——**跨端统一的多页管理台**。
 *
 * ## 为什么新增这一页
 *
 * 在本页之前，**web（h5App）产物里只有一个 `@Page("config")`**：打开 `http://<host>:8000/`
 * 看到的就是一个配置编辑器，无法回答"有几台设备在线、刚才谁在说话、上游报没报错"。
 * 带侧边栏的壳（`AdminShell`）原先只存在于 `macosArm64Main`，[shared/build.gradle.kts]
 * 里明写"测试功能，web 不暴露"。参考项目 `xiaozhi-esp32-server` 的 `manager-web` 则是
 * 「首页看板 + 设备管理 + 模型配置 + …」的多页结构，本页把其中与本项目能力匹配的部分补上。
 *
 * ## 结构
 *
 * ```
 * ┌ 侧边栏(240) ┬ 内容区 ─────────────────────────┐
 * │ 概览        │ largeTitleBar(标题 + 右侧动作)   │
 * │ 设备        │ Scroller / tabbedPanel          │
 * │ 会话        │                                 │
 * │ 对话记录    │                                 │
 * │ 用量        │                                 │
 * │ 延迟        │                                 │
 * │ 配置  ●     │                                 │
 * └─────────────┴─────────────────────────────────┘
 * ```
 *
 * 「配置」沿用既有的 `renderForm`（14 个 tab 的 Tabs+PageList），因此本页**没有**取代
 * [ConfigPage]：`page_name=config` 仍可单独打开（单页模式，便于嵌入式/调试场景）。
 *
 * ## 与 macOS `AdminShell` 的关系
 *
 * 两者并列而不合并：macOS 壳额外挂着「测试台」与「切换服务器」（依赖 macosArm64Main 的
 * `TestBenchState` / `XiaoZhiModule`，web 端不编译）。两壳**共用**同一套侧边栏原语
 * （[sidebarItem]）与导航常量（[AdminNav]），因此视觉与信息层级不会漂移。
 *
 * ## Kuikly DSL 约束（踩过的坑，勿改）
 *
 * - 组件是 `ViewContainer` 的扩展，必须挂在"词法作用域最近的接收者"上：嵌套闭包里
 *   无法隐式访问 Pager 成员（`@DslMarker`），故用局部 `val ctx = this` 捕获状态，
 *   而渲染扩展（`renderOpsXxx` / `renderForm`）必须在**目标容器闭包内非限定调用**
 *   （写成 `ctx.renderXxx(...)` 会把节点挂到根容器 → 布局逃逸）。
 * - 尺寸相关 lambda 必须定义在 `body()` 顶层（`return` 块之外）：`@DslMarker` 同样会屏蔽
 *   嵌套 builder 闭包对 `pagerData` 的访问。
 */
@Page("admin")
class AdminPage : Pager() {

    /** 配置表单（复用既有实现：同一份 [ConfigFormState] 驱动「配置」section）。 */
    val form = ConfigFormState(this)

    /** 能力总览的数据源（`GET /api/plugins`，只读）。 */
    val plugins = PluginMetaState(this)

    /** 「MCP 工具」（`GET /api/mcp/tools`，只读）。 */
    val mcpTools = McpToolsState(this)

    /** 「OTA 固件」（`GET /api/ota/firmware` + 上传/删除）。 */
    val ota = OtaState(this)

    /** 顶部 toast（跨端统一实现，见 [ToastHost]）。 */
    val toast = ToastState(this)

    // ---- 运维面数据源（新增，对应服务端 `app/registry` 的只读投影）----
    val stats = StatsState(this)
    val devices = DevicesState(this)
    val sessions = SessionsState(this)
    val conversations = ConversationsState(this)
    val usage = UsageState(this)

    /** 「延迟」大盘（`GET /api/metrics`，P0-5 事件总线订阅者的只读投影）。 */
    val metrics = MetricsState(this)

    /** 当前选中的导航项（[AdminNav] 的 id）。 */
    var selectedSection by observable(AdminNav.DEFAULT)

    /** 顶部弹出一条提示（成功=ok / 失败=error / 提示=info），3 秒后自动消失。 */
    fun showToast(message: String, level: String = "info") = toast.show(message, level)

    override fun pageDidAppear() {
        super.pageDidAppear()
        // web 端由 server 同域托管，基址留空（相对路径）；鉴权 token 由 URL 参数带进来
        //（`?token=xxx&page_name=admin`，与 /api/ws 的浏览器兜底口径一致）。
        val token = pagerData.params.optString("token", "")
        form.load(this)
        plugins.load(this)
        mcpTools.load(this)
        ota.load(this, "", token)
        // 运维面六路数据：并行发出，各自的 loading/loadError 独立（一个端点挂了不影响其他页面）
        stats.load(this)
        devices.load(this)
        sessions.load(this)
        conversations.load(this)
        usage.load(this)
        metrics.load(this)
    }

    override fun body(): ViewBuilder {
        val ctx = this
        // H5 是同域托管的网页（没有 macOS 红黄绿窗口按钮），不需要顶部避让；
        // 若 macOS 直接打开本页（`?page_name=admin`），保留 36pt 避开窗口按钮。
        val isH5 = pagerData.params.optString("is_H5", "").isNotEmpty()
        val topInset = if (isH5) 0f else 36f
        return {
            attr {
                flex(1f)
                flexDirectionColumn()
                backgroundColor(AdminColors.windowBg)
            }

            View {
                attr {
                    flex(1f)
                    flexDirectionRow()
                }

                // ===== 侧边栏 =====
                View {
                    attr {
                        width(240f)
                        flexDirectionColumn()
                        backgroundColor(AdminColors.sidebarBg)
                        paddingTop(topInset)
                    }
                    // ⚠️ 非限定调用：接收者必须是侧边栏 View，导航项才会挂进侧边栏
                    // ⚠️ 选中态传 lambda（在 attr/vif 闭包内读取 observable），点击后高亮才响应式
                    // 导航表是编译期常量、不会变，用普通 for 即可（`vfor` 是给 ObservableList 的）
                    for (item in AdminNav.all) {
                        sidebarItem(
                            item.label,
                            { ctx.selectedSection == item.id },
                            showDot = { item.id == "config" && ctx.form.dirty },
                        ) {
                            ctx.selectedSection = item.id
                        }
                    }
                    // 底部弹性占位 + 版本信息
                    View { attr { flex(1f) } }
                    Text {
                        attr {
                            fontSize(AdminType.micro)
                            color(AdminColors.textTertiary)
                            marginLeft(AdminSpace.md)
                            marginBottom(AdminSpace.md)
                            text("小智服务端 · 管理台")
                        }
                    }
                }

                // 侧边栏右侧 1px 分隔
                View {
                    attr {
                        width(1f)
                        backgroundColor(AdminColors.divider)
                    }
                }

                // ===== 内容区 =====
                View {
                    attr {
                        flex(1f)
                        flexDirectionColumn()
                        paddingTop(topInset)
                    }

                    largeTitleBar(
                        title = { AdminNav.label(ctx.selectedSection) },
                        trailing = {
                            vif({ ctx.selectedSection == "config" && ctx.form.conflict }) {
                                // 过期写入（409）：草稿仍在本地，给一个「重新加载」出口
                                secondaryButton("重新加载") { ctx.form.reload(ctx) }
                                View { attr { width(AdminSpace.sm) } }
                            }
                            vif({ ctx.selectedSection == "config" }) {
                                primaryButton(
                                    "保存配置",
                                    loading = { ctx.form.saving },
                                    loadingText = "保存中…",
                                    result = { ctx.form.saveResult },
                                    successText = "✓ 已保存",
                                    errorText = "✕ 保存失败",
                                ) {
                                    // toast 在**接口返回后**由回调触发（不能紧随 save() 取 statusMsg：
                                    // 那时请求还没回来，会弹出上一条旧消息）
                                    ctx.form.save(ctx) { msg, level -> ctx.showToast(msg, level) }
                                }
                            }
                        },
                    )

                    // ===== section 切换：vif 闭包内读取 selectedSection（observable）=====
                    vif({ ctx.selectedSection == "overview" }) {
                        opsScroller { renderOpsOverview(ctx.stats, ctx.plugins, ctx) }
                    }
                    vif({ ctx.selectedSection == "devices" }) {
                        opsScroller { renderOpsDevices(ctx.devices, ctx) }
                    }
                    vif({ ctx.selectedSection == "sessions" }) {
                        opsScroller { renderOpsSessions(ctx.sessions, ctx) }
                    }
                    vif({ ctx.selectedSection == "conversations" }) {
                        opsScroller { renderOpsConversations(ctx.conversations, ctx) }
                    }
                    vif({ ctx.selectedSection == "usage" }) {
                        opsScroller { renderOpsUsage(ctx.usage, ctx) }
                    }
                    vif({ ctx.selectedSection == "metrics" }) {
                        opsScroller { renderOpsMetrics(ctx.metrics, ctx) }
                    }
                    vif({ ctx.selectedSection == "config" }) {
                        // 配置改为「左侧分组导航 + 右侧单 section」两栏布局（替代 14 个平铺 tab）
                        renderConfigLayout(
                            ctx.form,
                            ctx.plugins,
                            ctx.mcpTools,
                            ctx.ota,
                            ctx.toast,
                            ctx,
                        )
                    }
                }
            }

            // 顶部浮动 toast：必须放在**页面根容器**最后（绝对定位，不占高度、盖在内容之上）
            ToastHost(ctx.toast)
        }
    }
}
