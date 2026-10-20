package com.xiaozhi.admin

import com.tencent.kuikly.core.annotations.Page
import com.tencent.kuikly.core.base.CursorType
import com.tencent.kuikly.core.base.ViewBuilder
import com.tencent.kuikly.core.base.ViewContainer
import com.tencent.kuikly.core.directives.vif
import com.tencent.kuikly.core.module.Module
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
 * ## 响应式布局（P1-4）
 *
 * 侧边栏按 `pagerData.pageViewWidth`（响应式）二态：宽屏（≥768）常驻左列；
 * 窄屏隐藏常驻列，顶栏汉堡按钮开**抽屉**（绝对定位浮层：遮罩 + 面板，复用 ToastHost
 * 范式；点遮罩/选中导航后收起）。两态共用 [adminSidebar]，视觉不漂移。
 * 运维页首次加载显示骨架卡（`skeletonCard`，见 OpsPages）。
 *
 * ## 平台差异（expect/actual）
 *
 * web 与 macOS **共用本壳**（原 macOS 专属 `AdminShell` @Page("router") 已废除，见
 * macosArm64Main/AdminPlatform.kt 的 actual）。差异全部收口为 expect/actual：macOS 注入
 * [XiaoZhiModule]、渲染 mac-only 测试台（`renderTestbenchSection`）与「切换服务器」
 * （`macServerSwitchEntry`）、基址经 [pagerBaseUrl] 读 SP 记忆地址；web 全部为空/""（同域
 * 相对路径）。测试台菜单仅 macOS 显示（[AdminNav.visible]），web 端彻底隐藏。
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

    /** 引擎健康自检的数据源（`GET /api/health/engines`，只读，P1-6）。 */
    val engineHealth = EngineHealthState(this)

    /** 「MCP 工具」（`GET /api/mcp/tools`，只读）。 */
    val mcpTools = McpToolsState(this)

    /** 「OTA 固件」（`GET /api/ota/firmware` + 上传/删除）。 */
    val ota = OtaState(this)

    /** 顶部 toast（跨端统一实现，见 [ToastHost]）。 */
    val toast = ToastState(this)

    // ---- 运维面数据源（新增，对应服务端 `app/registry` 的只读投影）----
    val stats = StatsState(this)
    val devices = DevicesState(this)

    /** 「EchoMuse 设备」（`GET /api/echomuse/devices` + 批准 / 重启 / WiFi 下发）。 */
    val echomuse = EchoMuseState(this)
    val sessions = SessionsState(this)
    val conversations = ConversationsState(this)
    val usage = UsageState(this)

    /** 「延迟」大盘（`GET /api/metrics`，P0-5 事件总线订阅者的只读投影）。 */
    val metrics = MetricsState(this)

    /** 当前选中的导航项（[AdminNav] 的 id）。 */
    var selectedSection by observable(AdminNav.DEFAULT)

    /**
     * 窄屏抽屉开关（P1-4）：`pagerData.pageViewWidth < [NARROW_BREAKPOINT]` 时侧边栏
     * 常驻列隐藏，改由顶栏汉堡按钮展开抽屉（绝对定位浮层，点遮罩/选中导航后收起）。
     */
    var drawerOpen by observable(false)

    /**
     * macOS「服务器设置」表单开关（web 恒 false）：入口与表单渲染在 mac actual 的
     * [macServerSwitchEntry]（侧边栏内联展开），保存后经 [refreshAll] 重拉数据。
     */
    var serverSwitchOpen by observable(false)

    /** 当前生效的 API 基址（[refreshAll] 时确定）：mac 后续动作（save/reload/个性化 load）沿用同一基址；web 恒 ""。 */
    var apiBase by observable("")

    /** 顶部弹出一条提示（成功=ok / 失败=error / 提示=info），3 秒后自动消失。 */
    fun showToast(message: String, level: String = "info") = toast.show(message, level)

    override fun pageDidAppear() {
        super.pageDidAppear()
        refreshAll(pagerBaseUrl(this))
    }

    /**
     * 拉取全部 section 数据：页面首现与 macOS「切换服务器」后共用。
     * web 端 base=""（server 同域托管走相对路径，行为不变）；macOS 传记忆地址。
     * 鉴权 token 由 URL 参数带进来（`?token=xxx&page_name=admin`，与 /api/ws 的浏览器兜底口径一致）。
     */
    fun refreshAll(base: String) {
        apiBase = base
        val token = pagerData.params.optString("token", "")
        form.load(this, base)
        form.loadPresets(this, base)
        plugins.load(this, base)
        mcpTools.load(this, base)
        ota.load(this, base, token)
        // 运维面多路数据：并行发出，各自的 loading/loadError 独立（一个端点挂了不影响其他页面）
        stats.load(this, base)
        devices.load(this, base)
        echomuse.configureAccess(token, base)
        echomuse.load(this, base)
        sessions.load(this, base)
        conversations.load(this, base)
        usage.load(this, base)
        metrics.load(this, base)
        engineHealth.load(this, base)
    }

    override fun createExternalModules(): Map<String, Module> = platformExternalModules()

    override fun body(): ViewBuilder {
        val ctx = this
        // 字号设备适配：按本机视口宽度算一次缩放系数（1.0~1.5），AdminType.* 取值时生效
        AdminType.scale = AdminType.scaleFor(pagerData.pageViewWidth)
        // H5 是同域托管的网页（没有 macOS 红黄绿窗口按钮），不需要顶部避让；
        // 若 macOS 直接打开本页（`?page_name=admin`），保留 36pt 避开窗口按钮。
        val isH5 = pagerData.params.optString("is_H5", "").isNotEmpty()
        val topInset = if (isH5) 0f else 36f
        // 窄屏判定（P1-4）：pageViewWidth 是响应式字段，但**直接访问只在 body 顶层合法**
        // （@DslMarker 屏蔽嵌套 builder 闭包对 pagerData 的访问），故收敛成顶层 lambda，
        // 嵌套 vif 里一律经 isNarrow() 求值——窗口 resize 跨过断点时自动 常驻 ⇄ 抽屉 切换。
        val isNarrow = { pagerData.pageViewWidth < NARROW_BREAKPOINT }
        // 测试台布局参数（mac-only section，见下方分支）：与 AdminShell 时代同一口径
        //（宽 = 窗口宽 - 侧边栏(240) - 分隔(1)；高 = 窗口高 - 顶避让(36) - 标题栏区(145-36)）。
        val twoColumn = { pagerData.pageViewWidth >= 900f }
        val contentPageWidth = { pagerData.pageViewWidth - 241f }
        val contentPageHeight = { pagerData.pageViewHeight - 145f }
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

                // ===== 侧边栏：宽屏常驻；窄屏隐藏（改顶栏汉堡 + 抽屉浮层，见文件末尾）=====
                vif({ !isNarrow() }) {
                    // ⚠️ 非限定调用：接收者必须是本容器，侧边栏节点才会挂进主行
                    adminSidebar(ctx, topInset)
                    // 侧边栏右侧 1px 分隔
                    View {
                        attr {
                            width(1f)
                            backgroundColor(AdminColors.divider)
                        }
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
                        title = {
                            // 配置 section：编辑目标为设备时标题带设备号（P1-5「当前编辑对象」）
                            if (ctx.selectedSection == "config") {
                                if (ctx.form.editDeviceId.isEmpty()) "配置" else "配置 · 设备 ${ctx.form.editDeviceId}"
                            } else {
                                AdminNav.label(ctx.selectedSection)
                            }
                        },
                        // 窄屏：汉堡按钮开抽屉（宽屏 vif 不渲染，常驻侧边栏承担导航）
                        leading = {
                            vif({ isNarrow() }) {
                                hamburgerButton { ctx.drawerOpen = true }
                            }
                        },
                        trailing = {
                            vif({ ctx.selectedSection == "config" && ctx.form.conflict }) {
                                // 过期写入（409）：草稿仍在本地，给一个「重新加载」出口
                                secondaryButton("重新加载") { ctx.form.reload(ctx, ctx.apiBase) }
                                View { attr { width(AdminSpace.sm) } }
                            }
                            // 保存按钮：构建期文案参数 → 用 vif 切换两份按钮响应编辑对象
                            //（全局 = POST /api/config；设备 = 白名单差量 PUT overlay）
                            vif({ ctx.selectedSection == "config" && ctx.form.editDeviceId.isEmpty() }) {
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
                                    ctx.form.save(ctx, ctx.apiBase) { msg, level -> ctx.showToast(msg, level) }
                                }
                            }
                            vif({ ctx.selectedSection == "config" && ctx.form.editDeviceId.isNotEmpty() }) {
                                primaryButton(
                                    "保存设备配置",
                                    loading = { ctx.form.saving },
                                    loadingText = "保存中…",
                                    result = { ctx.form.saveResult },
                                    successText = "✓ 已保存",
                                    errorText = "✕ 保存失败",
                                ) {
                                    ctx.form.save(ctx, ctx.apiBase) { msg, level -> ctx.showToast(msg, level) }
                                }
                            }
                        },
                    )

                    // ===== section 切换：vif 闭包内读取 selectedSection（observable）=====
                    vif({ ctx.selectedSection == "overview" }) {
                        opsScroller {
                            renderOpsOverview(ctx.stats, ctx.plugins, ctx.engineHealth, ctx) { section ->
                                // 横幅跳转：切到配置页并定位对应 section（P0 导航重组既有链路）
                                ctx.selectedSection = "config"
                                ctx.form.configSection = section
                            }
                        }
                    }
                    vif({ ctx.selectedSection == "testbench" }) {
                        // 测试台（P1-3）：mac-only（菜单已在 web 隐藏）。mac actual 按 Pager 缓存
                        // TestBenchState，切走再切回来连接/录音状态不丢；⚠️ 非限定调用挂进内容区
                        renderTestbenchSection(ctx, ctx.form, twoColumn, contentPageWidth, contentPageHeight)
                    }
                    vif({ ctx.selectedSection == "devices" }) {
                        opsScroller {
                            renderOpsDevices(
                                ctx.devices,
                                ctx.form,
                                // 「个性化配置」入口（P1-5）：以该设备为编辑目标跳配置 section
                                //（editDeviceId 置位后 load 走「全局基线 + overlay」分支）
                                { id ->
                                    ctx.form.editDeviceId = id
                                    ctx.form.load(ctx, ctx.apiBase)
                                    ctx.selectedSection = "config"
                                },
                                ctx,
                            )
                        }
                    }
                    vif({ ctx.selectedSection == "echomuse" }) {
                        opsScroller {
                            renderEchoMuse(
                                ctx.echomuse,
                                ctx.toast,
                                ctx,
                                isActive = { ctx.selectedSection == "echomuse" },
                            )
                        }
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
                        // 配置采用三级导航：一级侧边栏 → 二级横向 tabs → 三级纵向子菜单 + 内容
                        renderConfigLayout(
                            ctx.form,
                            ctx.plugins,
                            ctx.engineHealth,
                            ctx.mcpTools,
                            ctx.ota,
                            ctx.toast,
                            ctx,
                        )
                    }
                }
            }

            // ===== 窄屏抽屉浮层（P1-4，复用 ToastHost 的绝对定位范式）=====
            // zIndex 8/9 < ToastHost 的 10：toast 永远盖在抽屉之上。v1 直接出现/消失
            // 不做位移动画——跨端动画成本高，"能用、可关"优先。
            vif({ isNarrow() && ctx.drawerOpen }) {
                // 遮罩：点击任意处关闭
                View {
                    attr {
                        absolutePosition(top = 0f, left = 0f, right = 0f, bottom = 0f)
                        backgroundColor(AdminColors.overlay)
                        zIndex(8)
                    }
                    event { click { ctx.drawerOpen = false } }
                }
                // 抽屉面板：左侧滑出位（内部 adminSidebar 自带顶部避让 paddingTop）
                View {
                    attr {
                        absolutePosition(top = 0f, left = 0f, bottom = 0f)
                        width(240f)
                        backgroundColor(AdminColors.sidebarBg)
                        zIndex(9)
                    }
                    adminSidebar(ctx, topInset)
                }
            }

            // 顶部浮动 toast：必须放在**页面根容器**最后（绝对定位，不占高度、盖在内容之上）
            ToastHost(ctx.toast)
        }
    }
}

/** 窄屏断点（P1-4）：低于此宽度侧边栏改抽屉。与 twoPane 的 900 双列断点各自独立。 */
private const val NARROW_BREAKPOINT = 768f

/**
 * 侧边栏（宽屏常驻列与窄屏抽屉**共用**）：分组标题 + 导航项 + 底部版本行。
 * 从 body 内联代码抽出（P1-4）：抽屉与常驻列必须是同一个组件，视觉才不会漂移。
 *
 * ⚠️ 非限定调用（接收者 = 目标容器）；导航表是编译期常量用普通 for；
 * ⚠️ 选中态传 lambda（在闭包内读取 observable），点击后高亮才响应式。
 */
private fun ViewContainer<*, *>.adminSidebar(ctx: AdminPage, topInset: Float) {
    View {
        attr {
            width(240f)
            flexDirectionColumn()
            backgroundColor(AdminColors.sidebarBg)
            paddingTop(topInset)
        }
        // 分组标题在 group 变化处插入（AdminNav.init 已护栏：同组必须相邻声明）
        var lastGroup = ""
        for (item in AdminNav.visible(IS_MACOS)) {
            if (item.group != lastGroup) {
                lastGroup = item.group
                navGroupHeader(item.group)
            }
            sidebarItem(
                item.label,
                { ctx.selectedSection == item.id },
                showDot = { item.id == "config" && ctx.form.dirty },
            ) {
                ctx.selectedSection = item.id
                // 窄屏抽屉：选中导航后收起（宽屏 drawerOpen 恒为 false，无副作用）
                ctx.drawerOpen = false
            }
        }
        // macOS 专属：「服务器设置」入口 + 内联地址表单（js/其他平台 actual 空实现）
        macServerSwitchEntry(
            ctx,
            { ctx.serverSwitchOpen },
            { ctx.serverSwitchOpen = true },
            { ctx.serverSwitchOpen = false },
        ) {
            ctx.refreshAll(pagerBaseUrl(ctx))
            ctx.showToast("已切换服务器，数据已重新加载", "ok")
        }
        // 底部弹性占位 + 版本信息
        View { attr { flex(1f) } }
        Text {
            attr {
                fontSize(AdminType.micro)
                color(AdminColors.textTertiary)
                marginLeft(AdminSpace.md + AdminSpace.sm)
                marginBottom(AdminSpace.md)
                text("小智服务端 · 管理台")
            }
        }
    }
}

/** 侧边栏分组小标题（P1-4）：micro 灰字，左缘对齐导航项文字（sm 外距 + md 内距 = 28）。 */
private fun ViewContainer<*, *>.navGroupHeader(title: String) {
    Text {
        attr {
            fontSize(AdminType.micro)
            color(AdminColors.textTertiary)
            marginLeft(AdminSpace.md + AdminSpace.sm)
            marginTop(AdminSpace.md)
            marginBottom(AdminSpace.xs)
            text(title)
        }
    }
}

/** 汉堡按钮（窄屏开抽屉）：44x44 触控目标内画三条 2px 横线，右留 sm 与标题隔开。 */
private fun ViewContainer<*, *>.hamburgerButton(onClick: () -> Unit) {
    View {
        attr {
            width(44f)
            height(44f)
            alignItemsCenter()
            justifyContentCenter()
            marginRight(AdminSpace.sm)
            // 点击目标：悬停指针
            cursor(CursorType.POINTER)
        }
        event { click { onClick() } }
        for (i in 0 until 3) {
            View {
                attr {
                    width(18f)
                    height(2f)
                    borderRadius(AdminShape.radiusPill)
                    backgroundColor(AdminColors.textPrimary)
                    marginTop(if (i == 0) 0f else 4f)
                }
            }
        }
    }
}
