package com.xiaozhi.admin

import com.tencent.kuikly.core.render.web.ktx.SizeI
import com.tencent.kuikly.core.render.web.processor.KuiklyProcessor
import com.tencent.kuikly.core.render.web.runtime.web.expand.setAutoUpdateRootViewSizeOnResize
import com.xiaozhi.admin.KuiklyWebRenderViewDelegator
import kotlinx.browser.document
import kotlinx.browser.window
import utils.URL

/**
 * Web(H5) 入口：用渲染代理初始化并创建 RenderView。
 * 访问 `http://<host>/?page_name=admin` 打开管理台（默认页）；
 * `?page_name=config` 仍可单独打开配置页（[com.xiaozhi.admin.ConfigPage]）。
 */
fun main() {
    console.log("##### Kuikly Web Render")

    // 根容器 id，需与 index.html 中的容器一致
    val containerId = "root"
    val H5Sign = "is_H5"

    // 解析 URL 参数
    val urlParams = URL.parseParams(window.location.href)
    // 页面名，默认 admin（管理台：侧边栏 + 概览/设备/会话/对话记录/用量/配置）。
    // 单页模式仍可用 `?page_name=config` 单独打开配置页（ConfigPage）。
    val pageName = urlParams["page_name"] ?: "admin"

    val containerWidth = window.innerWidth
    val containerHeight = window.innerHeight

    val params: MutableMap<String, String> = mutableMapOf()
    // 标记来自 H5 平台（业务侧可据此做降级 / 特殊处理）
    params[H5Sign] = "1"
    // 透传所有 URL 参数（如 page_name），空参数集时等价于不清空
    params.putAll(urlParams)

    val paramMap = mapOf(
        "statusBarHeight" to 0f,
        "activityWidth" to containerWidth,
        "activityHeight" to containerHeight,
        "param" to params,
    )

    // Official throttled window/container forwarding keeps Pager breakpoints reactive
    // on desktop resize without recreating the page or losing drafts/IME state.
    setAutoUpdateRootViewSizeOnResize(true)

    // Web 管理台允许划选/复制文本：渲染器默认 preventDefaultSelect=true，会在桌面浏览器把
    // Scroller 内容的 selectstart 全部 preventDefault——整页只有 Scroller 外的顶部标题可选，
    // 卡片里的设备号/板型/日志等全部无法选中复制。关掉 select 拦截（渲染器 click 处理已自带
    // 「存在选区则忽略 click」防误触）；drag 拦截保持默认 true，防 HTML5 原生拖拽吞掉
    // mouseup 把 PageList 滚动状态机卡死（上游 KuiklyProcessor 文档同口径）。
    KuiklyProcessor.preventDefaultSelect = false

    val delegator = KuiklyWebRenderViewDelegator()
    delegator.init(containerId, pageName, paramMap, SizeI(containerWidth, containerHeight))
    delegator.resume()

    document.addEventListener("visibilitychange", {
        val hidden = document.asDynamic().hidden as Boolean
        if (hidden) delegator.pause() else delegator.resume()
    })
}
