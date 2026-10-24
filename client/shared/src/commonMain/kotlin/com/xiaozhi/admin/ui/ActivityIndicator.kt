package com.xiaozhi.admin.ui

import com.tencent.kuikly.core.base.Animation
import com.tencent.kuikly.core.base.Color
import com.tencent.kuikly.core.base.ComposeAttr
import com.tencent.kuikly.core.base.ComposeEvent
import com.tencent.kuikly.core.base.ComposeView
import com.tencent.kuikly.core.base.Rotate
import com.tencent.kuikly.core.base.ViewBuilder
import com.tencent.kuikly.core.base.ViewContainer
import com.tencent.kuikly.core.reactive.handler.observable
import com.tencent.kuikly.core.views.Canvas
import kotlin.math.PI
import kotlin.math.min

/** Render-layer loop; no polling timer, network state, or platform-specific animation bridge. */
private class AdminActivityIndicatorView(private val tint: Color) : ComposeView<ComposeAttr, ComposeEvent>() {
    private var spinning by observable(false)

    override fun createAttr() = ComposeAttr()
    override fun createEvent() = ComposeEvent()

    override fun viewDidLayout() {
        super.viewDidLayout()
        spinning = true
    }

    override fun viewWillUnload() {
        spinning = false
        super.viewWillUnload()
    }

    override fun body(): ViewBuilder {
        val ctx = this
        return {
            Canvas({
                attr {
                    absolutePositionAllZero()
                    touchEnable(false)
                    transform(Rotate(if (ctx.spinning) 360f else 0f))
                    animate(Animation.linear(0.85f).repeatForever(ctx.spinning), ctx.spinning)
                }
            }) { canvas, width, height ->
                canvas.beginPath()
                canvas.strokeStyle(ctx.tint)
                canvas.lineWidth(2f)
                canvas.lineCapRound()
                canvas.arc(width / 2f, height / 2f, ((min(width, height) - 3f) / 2f).coerceAtLeast(0f), 0f, 1.5f * PI.toFloat(), false)
                canvas.stroke()
            }
        }
    }
}

/** Mount only while loading (vif); removing it also removes the repeating native animation. */
fun ViewContainer<*, *>.activityIndicator(size: Float = 16f, tint: Color = AdminColors.disabledText) {
    addChild(AdminActivityIndicatorView(tint)) {
        attr { size(size, size); touchEnable(false) }
    }
}
