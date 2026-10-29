package com.xiaozhi.admin

import com.xiaozhi.admin.contracts.*
import com.xiaozhi.admin.state.stateVar

import com.gearui.App
import com.gearui.theme.ThemeMode
import com.tencent.kuikly.compose.ComposeContainer
import com.tencent.kuikly.compose.setContent
import com.tencent.kuikly.core.annotations.Page
import com.tencent.kuikly.core.module.Module

/**
 * 管理后台根页面（`page_name=admin`）——**跨端统一的多页管理台**（T06 起是 Compose 壳）。
 *
 * ## 本文件现在只负责什么
 *
 * 1. **页面级状态的所有权**：13 个 `XxxState(this)`（表单、插件、引擎健康、MCP、OTA、文件、
 *    toast、概览/设备两件套、EchoMuse、控制台；2026-10-29 范围削减后不再持有
 *    sessions / conversations / usage / metrics）与原实现逐字一致——它们的方法签名、构造参数、
 *    生命周期都不变，T06 只换了渲染层；
 * 2. **数据拉取入口**：[refreshAll]（首现与 macOS「切换服务器」共用）；
 * 3. **组合根**：`didInit` → `setContent` → [App]（GearUI 主题 + i18n + Overlay 运行时）→
 *    [AdminShell]。
 *
 * 布局、导航、断点、抽屉、标题栏、section 分发全部在 `AdminShell.kt`；本文件不再出现任何
 * 布局代码（旧 DSL 的 `body()` 已整体移除）。
 *
 * ## 为什么是 `ComposeContainer` + 手写 `App(...)`，而不是 `com.gearui.View`
 *
 * `gearui/View.kt` 的 `didInit` 是 `final override`，只把 `themeMode`/`theme`/`runtimeFlags`
 * 转给 `App(...)`——**不传 `languageTag`**，而 `App` 的默认值是 `en-US`。一旦走 `View`，
 * 内置文案（`EmptyState`、`Table.emptyText`、`Select` 占位、`ConfirmDialog` 的确定/取消、
 * `Checkbox` 的无障碍描述）会全部变成英文，界面中英混杂。所以这里手写组合根，
 * 显式把 `zh-Hans` 传给 `App`（见 [App] 的 `languageTag` / `fallbackLanguageTag`）。
 *
 * ## i18n 与主题（D2）
 *
 * - `themeMode = Light`、`theme = null`：完全采用 GearUI 默认主题，不覆写色板（Q1/A1）；
 * - `languageTag = fallbackLanguageTag = "zh-Hans"`：命中 GearUI 内置的 `Strings.zh_Hans`；
 * - `keyboardDismissMode` 用默认 `OnTapOrScroll`：点空白/滚动收起键盘，管理台表单正需要。
 *
 * ## 状态机制（T05）
 *
 * 页面级字段 `selectedSection` / `drawerOpen` / `serverSwitchOpen` / `apiBase` 都是
 * `stateVar`（Compose 快照）：AdminShell 读它们即会随写入重组——旧 `observable` 做不到这点。
 */
@Page("admin")
class AdminPage : ComposeContainer(), FieldHelpHost {

    /** 字段帮助弹层（ⓘ）显隐状态：本页唯一实例，经 `LocalFieldHelp` 下发给组件层。 */
    override val fieldHelp = FieldHelpState()

    /** 配置表单（复用既有实现：同一份 [ConfigFormState] 驱动「语音流程」与「全局配置」）。 */
    val form = ConfigFormState(this)

    /** 能力总览的数据源（`GET /api/plugins`，只读）。 */
    val plugins = PluginMetaState(this)

    /** 引擎健康自检的数据源（`GET /api/health/engines`，只读，P1-6）。 */
    val engineHealth = EngineHealthState(this)

    /** 「MCP 工具」（`GET /api/mcp/tools`，只读）。 */
    val mcpTools = McpToolsState(this)

    /** 「OTA 固件」（`GET /api/ota/firmware` + 上传/删除）。 */
    val ota = OtaState(this)

    /** 「文件」（服务端存储目录 `GET /api/files` + 新建/上传/删除）。 */
    val files = FilesState(this)

    /** 顶部 toast（T06 起转发 GearUI 的 `Toast`，3 秒自动消失，见 [ToastState]）。 */
    val toast = ToastState(this)

    // ---- 运维面数据源（对应服务端 `app/registry` 的只读投影）----
    // （2026-10-29 范围削减：sessions / conversations / usage / metrics 四页已移除，
    //  对应 state 字段与加载请求一并删除，见 docs/plans/gearui-kit-scope-reduction.md §二）
    val stats = StatsState(this)
    val devices = DevicesState(this)

    /** 「EchoMuse 设备」（`GET /api/echomuse/devices` + 批准 / 重启 / WiFi 下发）。 */
    val echomuse = EchoMuseState(this)

    /** 「控制台」（官方 dashboard 的 CONSOLE / 命令行）：实时服务器日志流 `GET /api/logs`。 */
    val console = ConsoleState(this)

    /** 当前选中的导航项（[AdminNav] 的 id）。 */
    var selectedSection by stateVar(AdminNav.DEFAULT)

    /**
     * 窄屏抽屉开关（P1-4）：视口宽度 < 768dp 时侧栏常驻列隐藏，改由标题栏汉堡按钮展开
     * 抽屉（GearUI `Drawer`，点遮罩/选中导航后收起）。
     */
    var drawerOpen by stateVar(false)

    /**
     * macOS「服务器设置」表单开关（web 恒 false）。
     *
     * ⚠️ T06 的 Compose 壳尚未接管这个入口：旧实现是 DSL 扩展 `macServerSwitchEntry`，
     * 它必须改签名为 `@Composable`（方案 D3）才能进壳，属 macOS 面板迁移任务。
     * 字段先保留，避免 T07–T11 里恢复入口时再改一遍页面状态。
     */
    var serverSwitchOpen by stateVar(false)

    /** 当前生效的 API 基址（[refreshAll] 时确定）：mac 后续动作沿用同一基址；web 恒 ""。 */
    var apiBase by stateVar("")

    /** 顶部弹出一条提示（成功=ok / 失败=error / 提示=info），3 秒后自动消失。 */
    fun showToast(message: String, level: String = "info") = toast.show(message, level)

    /**
     * 切换导航项（侧栏与抽屉共用）。
     *
     * 两个配置菜单共用一条表单路由：进入时各自回到上次停留的位置，否则「全局配置」
     * 会顶着上一轮流程节点的 section 打开，反之亦然（旧壳 `adminSidebar` 的同一口径）。
     * 窄屏下顺带收起抽屉（宽屏 `drawerOpen` 恒为 false，无副作用）。
     */
    fun selectSection(id: String) {
        selectedSection = id
        when (id) {
            "config" -> form.openWorkflowPage()
            "settings" -> form.openGlobalPage()
        }
        drawerOpen = false
    }

    override fun pageDidAppear() {
        super.pageDidAppear()
        // 跨菜单跳转：卡片里的跳转（能力健康修复 / 全局设置按钮）只给 section，
        // 由这里决定侧边栏落到哪个菜单——有流程节点回「语音流程」，否则去「全局配置」。
        form.onSectionRouted = { section ->
            selectedSection = if (ConfigSections.isGlobal(section)) "settings" else "config"
        }
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
        files.configureAccess(token, base)
        files.load(this)
        // 运维面多路数据：并行发出，各自的 loading/loadError 独立（一个端点挂了不影响其他页面）
        stats.load(this, base)
        devices.load(this, base)
        echomuse.configureAccess(token, base)
        echomuse.load(this, base)
        engineHealth.load(this, base)
        // 控制台：连日志流（服务端 /api/logs 已有实现）
        console.connect(base, token)
    }

    override fun createExternalModules(): Map<String, Module> = platformExternalModules()

    /**
     * 组合根：`setContent` 只在这里调用一次。
     *
     * `didInit` 是上游 `gearui/View` 选用的生命周期点（`Pager.didInit`），照搬即可；
     * 不用试点页的 `willInit` 是为了与上游示例一致，减少将来对照成本。
     */
    override fun didInit() {
        super.didInit()
        val page = this
        setContent {
            App(
                themeMode = ThemeMode.Light,
                theme = null,
                languageTag = "zh-Hans",
                fallbackLanguageTag = "zh-Hans",
            ) {
                AdminShell(page)
            }
        }
    }
}
