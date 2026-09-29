package com.xiaozhi.admin

import com.tencent.kuikly.core.render.web.IKuiklyRenderExport
import com.tencent.kuikly.core.render.web.expand.KuiklyRenderViewDelegatorDelegate
import com.tencent.kuikly.core.render.web.ktx.SizeI
import com.tencent.kuikly.core.render.web.runtime.web.expand.KuiklyRenderViewDelegator

/**
 * 实现 Web Render 提供的代理接口：传入 H5 页面初始化参数，并注册自定义模块 / 自定义 View。
 */
class KuiklyWebRenderViewDelegator : KuiklyRenderViewDelegatorDelegate {
    private val delegate = KuiklyRenderViewDelegator(this)

    fun init(containerId: String, pageName: String, pageData: Map<String, Any>, size: SizeI) {
        delegate.onAttach(containerId, pageName, pageData, size)
    }

    fun resume() = delegate.onResume()
    fun pause() = delegate.onPause()
    fun detach() = delegate.onDetach()

    override fun registerExternalModule(kuiklyRenderExport: IKuiklyRenderExport) {
        super.registerExternalModule(kuiklyRenderExport)
        // 注册自定义模块（示例）
        // kuiklyRenderExport.moduleExport(KRBridgeModule.MODULE_NAME) { KRBridgeModule() }
    }

    override fun registerExternalRenderView(kuiklyRenderExport: IKuiklyRenderExport) {
        super.registerExternalRenderView(kuiklyRenderExport)
        // 注册自定义 View
        // kuiklyRenderExport.renderViewExport(KRMyView.VIEW_NAME) { KRMyView() }
    }
}
