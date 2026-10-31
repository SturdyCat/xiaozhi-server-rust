package com.xiaozhi.admin

import com.tencent.kuikly.core.render.web.export.IKuiklyRenderViewExport
import com.tencent.kuikly.core.render.web.ktx.KuiklyRenderCallback
import kotlinx.browser.document
import org.w3c.dom.Element
import org.w3c.dom.HTMLDivElement
import org.w3c.dom.HTMLIFrameElement

/** A real iframe browsing context, sized by Kuikly's normal frame prop. Never window.open. */
class EchoMuseOfficialViewHost : IKuiklyRenderViewExport {
    override val ele = (document.createElement("div") as HTMLDivElement).apply {
        style.setProperty("overflow", "hidden")
    }
    private var frame: HTMLIFrameElement? = null
    private var currentUrl: String? = null

    override fun setProp(propKey: String, propValue: Any): Boolean {
        if (propKey != "url") return super.setProp(propKey, propValue)
        val url = propValue as? String ?: ""
        if (url == currentUrl) return true
        disposeDocument()
        if (url.isBlank()) return true
        currentUrl = url
        frame = (document.createElement("iframe") as HTMLIFrameElement).apply {
            title = "EchoMuse 官方设备管理"
            style.width = "100%"
            style.height = "100%"
            style.border = "0"
            style.display = "block"
            // The query includes credentials; don't send it as a referrer to linked resources.
            setAttribute("referrerpolicy", "no-referrer")
            // No popups or top-level navigation. Scripts/forms are needed by the official UI.
            setAttribute("sandbox", "allow-scripts allow-same-origin allow-forms allow-downloads")
            src = url
            ele.appendChild(this)
        }
        return true
    }

    override fun call(method: String, params: String?, callback: KuiklyRenderCallback?): Any? {
        if (method == "dispose") {
            disposeDocument()
            return null
        }
        return super.call(method, params, callback)
    }

    override fun onRemoveFromParent(parent: Element) = disposeDocument()
    override fun onDestroy() = disposeDocument()

    private fun disposeDocument() {
        frame?.let {
            // Destroy the document even if its ancestor is being detached as one DOM subtree.
            it.src = "about:blank"
            it.parentNode?.removeChild(it)
            it.removeAttribute("src")
        }
        frame = null
        currentUrl = null
    }
}
