package com.xiaozhi.admin

import com.tencent.kuikly.core.annotations.Page
import com.tencent.kuikly.core.base.ViewBuilder
import com.tencent.kuikly.core.directives.vif
import com.tencent.kuikly.core.pager.Pager
import com.tencent.kuikly.core.views.View

/**
 * 管理后台 · 配置页（config），位于 shared 公共层，跨端复用（web / Android / iOS / OHOS）。
 *
 * 统一管理台（@Page("admin")）之外的单页模式兜底入口：
 * 其它平台没有侧边栏壳，仍可用本页单独打开配置。
 * 逻辑全部下沉到 ConfigFormState（load/save/renderForm），本页只负责壳 + 生命周期。
 */
@Page("config")
class ConfigPage : Pager() {

    val form = ConfigFormState(this)

    /** 能力总览的数据源（`GET /api/plugins`，只读，不随表单回传）。 */
    val plugins = PluginMetaState(this)

    /** 引擎健康自检的数据源（`GET /api/health/engines`，只读，P1-6：总览 section 健康卡）。 */
    val engineHealth = EngineHealthState(this)

    /** 「MCP 工具」的数据源（`GET /api/mcp/tools`，只读，不随表单回传）。 */
    val mcpTools = McpToolsState(this)

    /** 「OTA 固件」的数据源（`GET /api/ota/firmware` + 上传/删除；与 `/api/config` 无关）。 */
    val ota = OtaState(this)

    /** 顶部 toast（跨端统一实现：状态 + 渲染见 commonMain 的 ToastState / ToastHost）。 */
    val toast = ToastState(this)

    fun showToast(message: String, level: String = "info") = toast.show(message, level)

    override fun pageDidAppear() {
        super.pageDidAppear()
        // 编辑目标（P1-5）：`?edit_device=<device_id>` = 编辑该设备的个性化 overlay；
        // 缺省 = 全局配置。load 内部按目标分道（全局 /api/config vs 全局基线+overlay）。
        form.editDeviceId = pagerData.params.optString("edit_device", "")
        form.load(this)
        form.loadPresets(this)
        plugins.load(this)
        engineHealth.load(this)
        mcpTools.load(this)
        // web 端由 server 同域托管，基址留空（相对路径）；鉴权 token 由 URL 参数带进来
        //（`?token=xxx&page_name=config`，与 /api/ws 的浏览器兜底口径一致）
        ota.load(this, "", pagerData.params.optString("token", ""))
    }

    override fun body(): ViewBuilder {
        // Kuikly DSL 约定：嵌套容器闭包内无法隐式访问 Pager 成员（@DslMarker），
        // 用局部 val ctx 捕获 Pager 取状态；组件扩展则在目标容器闭包内非限定调用。
        val ctx = this
        // 字号设备适配：按本机视口宽度算一次缩放系数（1.0~1.5），AdminType.* 取值时生效
        AdminType.scale = AdminType.scaleFor(pagerData.pageViewWidth)
        // ⚠️ 配置布局内部自管尺寸（DslMarker 仅屏蔽嵌套闭包对 Pager 成员的隐式访问，不影响此处）。
        return {
            attr {
                flex(1f)
                flexDirectionColumn()
                backgroundColor(AdminColors.windowBg)
            }

            // ⚠️ 非限定调用（接收者=当前容器），组件才能挂进正确的父容器
            largeTitleBar({ "节点管理" + if (ctx.form.editDeviceId.isEmpty()) "" else " · 设备 ${ctx.form.editDeviceId}" },
                stackActions = { workspaceIsCompact(ctx) && (ctx.form.conflict || ctx.form.editDeviceId.isNotEmpty()) },
                trailing = {
                // 过期写入（409）后：草稿仍在本地，给一个「重新加载」出口（对齐 DSH 的冲突文案口径）
                vif({ ctx.form.conflict }) {
                    secondaryButton("重新加载") {
                        ctx.form.reload(ctx)
                    }
                    View { attr { width(AdminSpace.sm) } }
                }
                // 三态按钮：saving → 「保存中…」灰底拦截点击；接口返回后短暂显示
                // 「✓ 已保存 / ✕ 保存失败」（form.saveResult，约 2s 后自动回落）。
                // 设备个性化目标下文案切「保存设备配置」（提交白名单差量 overlay）。
                // ⚠️ primaryButton 文案是构建期参数，必须用 vif 切换两份按钮才能响应式
                //（editDeviceId 是 observable，vif 闭包内读取才会触发重建）。
                vif({ ctx.form.editDeviceId.isEmpty() }) {
                    primaryButton(
                        "保存配置",
                        loading = { ctx.form.saving },
                        loadingText = "保存中…",
                        result = { ctx.form.saveResult },
                        successText = "✓ 已保存",
                        errorText = "✕ 保存失败",
                    ) {
                        // toast 在**接口返回后**由 save 回调触发（不能在 save() 之后立即取 statusMsg：
                        // 那时请求还没回来，会弹出上一条旧消息）
                        ctx.form.save(ctx) { msg, level -> ctx.showToast(msg, level) }
                    }
                }
                vif({ ctx.form.editDeviceId.isNotEmpty() }) {
                    primaryButton(
                        "保存设备配置",
                        loading = { ctx.form.saving },
                        loadingText = "保存中…",
                        result = { ctx.form.saveResult },
                        successText = "✓ 已保存",
                        errorText = "✕ 保存失败",
                    ) {
                        ctx.form.save(ctx) { msg, level -> ctx.showToast(msg, level) }
                    }
                }
            })

            // Workflow at the top of this config content area, then the shared two-tab workspace.
            pipelineWorkflowHeader(ctx.form, ctx)
            renderConfigLayout(
                ctx.form,
                ctx.plugins,
                ctx.engineHealth,
                ctx.mcpTools,
                ctx.ota,
                ctx.toast,
                ctx,
            )

            // 顶部浮动 toast（跨端统一，3 秒自动消失）：必须放在**页面根容器**最后——
            // 绝对定位脱离布局（不占高度、不推挤内容），相对整页水平居中、盖在标题栏与内容之上。
            ToastHost(ctx.toast)
        }
    }
}
