package com.xiaozhi.admin

import com.tencent.kuikly.core.render.web.IKuiklyRenderExport
import com.tencent.kuikly.core.render.web.expand.KuiklyRenderViewDelegatorDelegate
import com.tencent.kuikly.core.render.web.ktx.SizeI
import com.tencent.kuikly.core.render.web.runtime.web.expand.KuiklyRenderViewDelegator

/**
 * 实现 Web Render 提供的代理接口：传入 H5 页面初始化参数，注册自定义模块与内嵌官方页。
 */
class KuiklyWebRenderViewDelegator : KuiklyRenderViewDelegatorDelegate {
    private val delegate = KuiklyRenderViewDelegator(this)

    fun init(containerId: String, pageName: String, pageData: Map<String, Any>, size: SizeI) {
        delegate.onAttach(containerId, pageName, pageData, size)
    }

    fun resume() = delegate.onResume()
    fun pause() = delegate.onPause()
    fun detach() = delegate.onDetach()

    override fun registerExternalRenderView(kuiklyRenderExport: IKuiklyRenderExport) {
        super.registerExternalRenderView(kuiklyRenderExport)
        kuiklyRenderExport.renderViewExport("EchoMuseOfficialViewHost", { EchoMuseOfficialViewHost() })
    }

    override fun registerExternalModule(kuiklyRenderExport: IKuiklyRenderExport) {
        super.registerExternalModule(kuiklyRenderExport)
        // web 端补齐 cursor 样式（base-js 不消费 "cursor" prop）：业务侧 attr { cursor(...) }
        // 经此处理器落到 DOM style.cursor，菜单/Tabs/按钮等点击目标悬停显手型——见 WebCursorPropHandler
        kuiklyRenderExport.viewPropExternalHandlerExport(WebCursorPropHandler())
    }
}
