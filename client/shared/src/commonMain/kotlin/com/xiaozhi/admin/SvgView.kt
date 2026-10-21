package com.xiaozhi.admin

import com.tencent.kuikly.core.base.Attr
import com.tencent.kuikly.core.base.Color
import com.tencent.kuikly.core.base.DeclarativeBaseView
import com.tencent.kuikly.core.base.ViewContainer
import com.tencent.kuikly.core.base.event.Event

/** Vector document, never an Image/UIImage. Hosts implement the bounded SVG Tiny contract. */
class SvgView : DeclarativeBaseView<SvgAttr, SvgEvent>() {
    override fun viewName(): String = "XzSvgView"
    override fun createAttr(): SvgAttr = SvgAttr()
    override fun createEvent(): SvgEvent = SvgEvent()
}

class SvgAttr : Attr() {
    /** Only trusted bundled SVG resources are supported initially; no remote/executable SVG. */
    fun src(source: String): SvgAttr {
        "src" with if (':' !in source && !source.startsWith('/')) "assets://common/$source" else source
        return this
    }

    /** Maps SVG currentColor to the same unsigned ARGB string as official Kuikly colors. */
    fun tintColor(color: Color?): SvgAttr {
        "tintColor" with (color?.toString() ?: "")
        return this
    }
}

class SvgEvent : Event() {
    /** A rejected/unsupported document stays empty; hosts never substitute a bitmap. */
    fun loadError(handler: () -> Unit) {
        register("loadError") { handler() }
    }
}

@Suppress("FunctionName")
fun ViewContainer<*, *>.Svg(init: SvgView.() -> Unit) {
    addChild(SvgView(), init)
}
