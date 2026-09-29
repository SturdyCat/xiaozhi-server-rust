package com.xiaozhi.admin

import com.tencent.kuikly.core.render.web.ktx.SizeI
import com.xiaozhi.admin.KuiklyWebRenderViewDelegator
import kotlinx.browser.document
import kotlinx.browser.window
import utils.URL

/**
 * Web(H5) 入口：用渲染代理初始化并创建 RenderView。
 * 访问 http://<host>/?page_name=router 打开对应页面。
 */
fun main() {
    console.log("##### Kuikly Web Render")

    // 根容器 id，需与 index.html 中的容器一致
    val containerId = "root"
    val H5Sign = "is_H5"

    // 解析 URL 参数
    val urlParams = URL.parseParams(window.location.href)
    // 页面名，默认 config（管理后台配置页）
    val pageName = urlParams["page_name"] ?: "config"

    val containerWidth = window.innerWidth
    val containerHeight = window.innerHeight

    val params: MutableMap<String, String> = mutableMapOf()
    if (urlParams.isNotEmpty()) {
        urlParams.forEach { (k, v) -> params[k] = v }
    }
    // 标记来自 H5 平台（业务侧可据此做降级 / 特殊处理）
    params[H5Sign] = "1"

    val paramMap = mapOf(
        "statusBarHeight" to 0f,
        "activityWidth" to containerWidth,
        "activityHeight" to containerHeight,
        "param" to params,
    )

    val delegator = KuiklyWebRenderViewDelegator()
    delegator.init(containerId, pageName, paramMap, SizeI(containerWidth, containerHeight))
    delegator.resume()

    document.addEventListener("visibilitychange", {
        val hidden = document.asDynamic().hidden as Boolean
        if (hidden) delegator.pause() else delegator.resume()
    })
}
