package com.xiaozhi.admin

import androidx.compose.runtime.Composable
import com.xiaozhi.admin.contracts.*
import com.xiaozhi.admin.kit.AdminScopeHost
import com.xiaozhi.admin.kit.GearFieldHelpDialog

import com.gearui.App
import com.gearui.components.button.Button
import com.gearui.components.button.ButtonTheme
import com.gearui.components.button.ButtonType
import com.gearui.components.navbar.NavBar
import com.gearui.foundation.layout.Spacing
import com.gearui.theme.ThemeMode
import com.tencent.kuikly.compose.ComposeContainer
import com.tencent.kuikly.compose.foundation.layout.Arrangement
import com.tencent.kuikly.compose.foundation.layout.Box
import com.tencent.kuikly.compose.foundation.layout.Column
import com.tencent.kuikly.compose.foundation.layout.Row
import com.tencent.kuikly.compose.foundation.layout.fillMaxSize
import com.tencent.kuikly.compose.foundation.layout.padding
import com.tencent.kuikly.compose.setContent
import com.tencent.kuikly.compose.ui.Alignment
import com.tencent.kuikly.compose.ui.Modifier
import com.tencent.kuikly.compose.ui.unit.dp
import com.tencent.kuikly.core.annotations.Page

/**
 * 管理后台 · 配置页（`page_name=config`），位于 shared 公共层，跨端复用（web / Android / iOS / macOS）。
 *
 * 统一管理台（`@Page("admin")`）之外的单页模式兜底入口：其它平台没有侧边栏壳，
 * 仍可用本页单独打开配置。页面 id **不变**（T06 只换渲染层，不改路由契约）。
 *
 * ## T06 起本页的形态
 *
 * 旧实现是纯 DSL（`body(): ViewBuilder`）。Compose 无法嵌进 DSL 布局，所以本页与
 * [AdminPage] 同步换成 `ComposeContainer` + 手写 `App(...)` 组合根：
 *
 * ```
 * didInit → setContent → App(themeMode/languageTag) → ConfigShell
 *                                                    ├─ NavBar（标题 + 保存）
 *                                                    └─ section 主体
 * ```
 *
 * **当前主体是"迁移中"占位**：配置表单（语音流程 / 全局配置两套视图）由 T07 / T08
 * 迁到 GearUI 表单组件，复用同一份 [ConfigFormState]。占位阶段保留完整的数据加载链路
 * （[pageDidAppear] 里的 5 个 loader 一行未删），所以迁移只换渲染、不改接口口径。
 *
 * ## 为什么手写 `App(...)` 而不是继承 `com.gearui.View`
 *
 * 与 [AdminPage] 同一条理由：`gearui/View.didInit()` 是 `final override` 且不传
 * `languageTag`，走它会让 GearUI 内置文案（空态、对话框按钮、占位符）全变英文。
 */
@Page("config")
class ConfigPage : ComposeContainer(), FieldHelpHost {

    /** 字段帮助弹层（ⓘ）显隐状态：本页唯一实例，经 `LocalFieldHelp` 下发给组件层。 */
    override val fieldHelp = FieldHelpState()

    /** 配置表单（与统一管理台**同一份**实现，不复制提交路径）。 */
    val form = ConfigFormState(this)

    /** 能力总览的数据源（`GET /api/plugins`，只读，不随表单回传）。 */
    val plugins = PluginMetaState(this)

    /** 引擎健康自检的数据源（`GET /api/health/engines`，只读，P1-6：总览 section 健康卡）。 */
    val engineHealth = EngineHealthState(this)

    /** 「MCP 工具」的数据源（`GET /api/mcp/tools`，只读，不随表单回传）。 */
    val mcpTools = McpToolsState(this)

    /** 「OTA 固件」的数据源（`GET /api/ota/firmware` + 上传/删除；与 `/api/config` 无关）。 */
    val ota = OtaState(this)

    /** 顶部 toast（转发 GearUI `Toast`，见 [ToastState]；`App` 已在根容器挂好 `ToastHost()`）。 */
    val toast = ToastState(this)

    /** 顶部弹出一条提示（成功=ok / 失败=error / 提示=info），3 秒后自动消失。 */
    fun showToast(message: String, level: String = "info") = toast.show(message, level)

    override fun pageDidAppear() {
        super.pageDidAppear()
        // 编辑目标（P1-5）：`?edit_device=<device_id>` = 编辑该设备的个性化 overlay；
        // 缺省 = 全局配置。load 内部按目标分道（全局 /api/config vs 全局基线+overlay）。
        form.editDeviceId = pagerData.params.optString("edit_device", "")
        // 基址一律留空：web 由 server 同域托管走相对路径；macOS 单页模式同样以页面
        // 自身来源为基址（与旧实现逐字一致，T06 不改网络口径）。
        form.load(this)
        form.loadPresets(this)
        plugins.load(this)
        engineHealth.load(this)
        mcpTools.load(this)
        // 鉴权 token 由 URL 参数带进来（`?token=xxx&page_name=config`，与 /api/ws 的浏览器兜底口径一致）
        ota.load(this, "", pagerData.params.optString("token", ""))
    }

    /**
     * 组合根：`setContent` 只在这里调用一次（与 [AdminPage] 同一生命周期点 `Pager.didInit`）。
     *
     * `themeMode = Light` + `theme = null`：完全采用 GearUI 默认主题（不覆写色板）；
     * `languageTag / fallbackLanguageTag = "zh-Hans"`：命中内置 `Strings.zh_Hans`。
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
                ConfigShell(page)
            }
        }
    }
}

/**
 * 配置页壳：标题栏 + 主体 + 字段帮助弹层。
 *
 * 与 [AdminShell] 共用 [AdminScopeHost]（视口测量 + 三个 CompositionLocal），
 * 断点口径因此不会在两个页面之间分叉。
 */
@Composable
private fun ConfigShell(page: ConfigPage) {
    AdminScopeHost(pager = page, fieldHelp = page.fieldHelp) { viewport ->
        Column(modifier = Modifier.fillMaxSize()) {
            ConfigTopBar(page)
            // 主体：T06 阶段统一渲染迁移占位（12 个 section 的真实内容由 T07–T11 接管）
            Box(modifier = Modifier.weight(1f).fillMaxSize()) {
                SectionPlaceholder(
                    title = configPageTitle(page.form),
                    detail = "配置表单（语音流程节点 / 全局配置）由 T07 / T08 迁到 GearUI 表单组件；" +
                        "两套视图共用同一份 ConfigFormState，不建第二条提交路径。",
                    viewport = viewport,
                )
            }
        }
        // 字段帮助弹层（ⓘ 的唯一实例）：每页根部挂一次，走 Popup.Host（不占布局）
        GearFieldHelpDialog()
    }
}

/**
 * 标题栏：标题随编辑目标切换（节点管理 / 全局配置），右侧是保存与（冲突时的）重新加载。
 *
 * 窄屏不做抽屉——单页模式本来就没有侧栏，抽屉没有可装的东西。
 */
@Composable
private fun ConfigTopBar(page: ConfigPage) {
    val form = page.form
    NavBar(
        title = configPageTitle(form),
        centerTitle = false,
        rightWidget = { SaveControls(page) },
        // 冲突态两个按钮并排（重新加载 + 保存），否则只占一个文字按钮槽
        rightWidgetWidth = if (form.conflict) 184.dp else 104.dp,
    )
}

/**
 * 保存区：三态按钮（idle / 保存中…/ 完成由 toast 呈现）。
 *
 * ⚠️ GearUI `Button` 没有旧 `primaryButton` 的 `result/successText/errorText` 内联反馈参数，
 * 保存结果改由 toast 呈现（[ConfigFormState.save] 的 `onResult` 本来就带 level）。
 * T07 落地真实表单时若需要按钮内联反馈，再评估是否用 `Button` + 状态文案自行实现。
 *
 * 设备个性化目标下文案切「保存设备配置」（提交白名单差量 overlay）——这是构建期参数，
 * 所以直接读 `editDeviceId` 求值（Compose 里读取快照状态即会重组，不需要旧 DSL 的 vif 双份按钮）。
 */
@Composable
private fun SaveControls(page: ConfigPage) {
    val form = page.form
    Row(
        modifier = Modifier.padding(end = Spacing.md),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(Spacing.sm),
    ) {
        if (form.conflict) {
            // 过期写入（409）后：草稿仍在本地，给一个「重新加载」出口（对齐 DSH 的冲突文案口径）
            Button(
                text = "重新加载",
                onClick = { form.reload(page) },
                type = ButtonType.OUTLINE,
            )
        }
        Button(
            text = when {
                form.saving -> "保存中…"
                form.editDeviceId.isEmpty() -> "保存配置"
                else -> "保存设备配置"
            },
            onClick = {
                // toast 在**接口返回后**由 save 回调触发（不能在 save() 之后立即取 statusMsg：
                // 那时请求还没回来，会弹出上一条旧消息）
                form.save(page) { msg, level -> page.showToast(msg, level) }
            },
            theme = ButtonTheme.PRIMARY,
            type = ButtonType.FILL,
            // loading 期间 Button 自身拦截点击，等价旧实现「保存中…灰底不可点」
            loading = form.saving,
        )
    }
}

/**
 * 单页模式标题：停在流程节点=「节点管理」（设备个性化带设备号），停在全局 section=「全局配置」。
 *
 * 统一管理台（`@Page("admin")`）的标题由侧边栏菜单决定（「语音流程」/「全局配置」），
 * 这里只是让**独立的配置页**在两种语义下都不要自称"节点管理"。
 */
private fun configPageTitle(form: ConfigFormState): String = when {
    form.workflowPosition != null ->
        "节点管理" + if (form.editDeviceId.isEmpty()) "" else " · 设备 ${form.editDeviceId}"
    else -> "全局配置"
}
