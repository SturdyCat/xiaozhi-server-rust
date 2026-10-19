package com.xiaozhi.admin

import com.tencent.kuikly.core.render.web.export.IKuiklyRenderViewExport
import com.tencent.kuikly.core.render.web.export.IKuiklyRenderViewPropExternalHandler
import org.w3c.dom.HTMLElement

/**
 * web 渲染器 **cursor 样式补齐**：core 侧 `attr { cursor(CursorType.POINTER) }` 下发的
 * "cursor" prop，base-js 渲染器（2.29.1）的 setCommonProp CSS 映射表不认识、会直接丢弃
 * （该 prop 目前只有 macOS 原生渲染器实现）。本处理器按 [IKuiklyRenderViewPropExternalHandler]
 * 兜底接住（渲染层 setProp 未消费的 prop 会转交外部处理器链），把值写进 DOM 元素的
 * style.cursor——侧边栏/标签页/按钮/下拉等点击目标即可悬停显手型。
 *
 * cursor 是 CSS 可继承属性：容器设置后内部 Text 等子元素一并生效，文本 I-beam 不再出现。
 * 注册点：[KuiklyWebRenderViewDelegator.registerExternalModule]。
 */
class WebCursorPropHandler : IKuiklyRenderViewPropExternalHandler {

    override fun setViewExternalProp(
        renderViewExport: IKuiklyRenderViewExport,
        propKey: String,
        propValue: Any,
    ): Boolean {
        if (propKey != PROP_CURSOR) return false
        renderViewExport.ele.unsafeCast<HTMLElement>()
            .style.setProperty(PROP_CURSOR, propValue.unsafeCast<String>())
        return true
    }

    override fun resetViewExternalProp(
        renderViewExport: IKuiklyRenderViewExport,
        propKey: String,
    ): Boolean {
        if (propKey != PROP_CURSOR) return false
        renderViewExport.ele.unsafeCast<HTMLElement>().style.removeProperty(PROP_CURSOR)
        return true
    }

    companion object {
        /** 与 core `Attr.cursor(type)` 下发的 prop key 一致；value 即 CSS cursor 关键字（如 "pointer"）。 */
        const val PROP_CURSOR = "cursor"
    }
}
