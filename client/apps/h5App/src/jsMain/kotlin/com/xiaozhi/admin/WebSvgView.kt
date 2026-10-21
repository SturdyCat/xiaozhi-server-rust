package com.xiaozhi.admin

import com.tencent.kuikly.core.render.web.export.IKuiklyRenderViewExport
import com.tencent.kuikly.core.render.web.ktx.KuiklyRenderCallback
import com.tencent.kuikly.core.render.web.ktx.toRgbColor
import kotlinx.browser.document
import org.w3c.dom.Element
import org.w3c.dom.HTMLElement

/** Dedicated inline vector DOM; never an Image, canvas, filter or bitmap fallback. */
class WebSvgView : IKuiklyRenderViewExport {
    private val root = document.createElement("div").unsafeCast<HTMLElement>().apply {
        style.setProperty("display", "block")
        style.setProperty("overflow", "hidden")
        style.setProperty("pointer-events", "none")
        style.setProperty("color", "rgba(0, 0, 0, 1)")
        setAttribute("data-xz-svg-src", "")
        setAttribute("data-xz-svg-state", "loading")
    }
    override val ele: Element get() = root
    private var src = ""
    private var generation = 0L
    private var detached = false
    private var destroyed = false
    private var hasTint = false
    private var template: Element? = null
    private var loadError: KuiklyRenderCallback? = null
    private var pendingError: String? = null
    private var errorDelivered = false

    override fun setProp(propKey: String, propValue: Any): Boolean = when (propKey) {
        "src" -> {
            src = propValue as? String ?: ""
            renderSource()
            true
        }
        "tintColor" -> {
            applyTint(propValue.toString())
            true
        }
        "loadError" -> {
            if (!destroyed) {
                loadError = propValue.unsafeCast<KuiklyRenderCallback>()
                deliverPendingError()
            }
            true
        }
        else -> super.setProp(propKey, propValue)
    }

    private fun applyTint(value: String) {
        val wasTinted = hasTint
        hasTint = value.isNotEmpty()
        // Exact SDK decimal ARGB conversion; empty means no tint, not a black override.
        root.style.setProperty("color", if (hasTint) value.toRgbColor() else "rgba(0, 0, 0, 1)")
        if (wasTinted != hasTint && !detached && !destroyed) template?.let(::installTemplate)
    }

    override fun resetProp(propKey: String): Boolean = when (propKey) {
        "tintColor" -> { applyTint(""); true }
        "loadError" -> { loadError = null; true }
        else -> super.resetProp(propKey)
    }

    private fun renderSource() {
        val ticket = ++generation
        template = null
        clearError()
        clearVector()
        val canonical = TrustedSvgDocuments.canonicalSource(src)
        // Rejected arbitrary URLs/queries are never reflected into DOM or callback diagnostics.
        root.setAttribute("data-xz-svg-src", canonical ?: "")
        if (canonical == null) {
            root.setAttribute("data-xz-svg-state", "error")
            if (!detached && !destroyed) reportError("unsupported_source")
            return
        }
        root.setAttribute("data-xz-svg-state", "loading")
        if (detached || destroyed) return
        TrustedSvgDocuments.load(canonical).then { loaded ->
            if (isCurrent(ticket)) {
                template = loaded
                installTemplate(loaded)
            }
        }.catch {
            if (isCurrent(ticket)) {
                template = null
                clearVector()
                root.setAttribute("data-xz-svg-state", "error")
                reportError("load_failed")
            }
        }
    }

    private fun installTemplate(loaded: Element) {
        clearVector()
        root.appendChild(TrustedSvgDocuments.cloneForView(loaded, hasTint))
        root.setAttribute("data-xz-svg-state", "ready")
    }

    private fun isCurrent(ticket: Long): Boolean = !destroyed && !detached && generation == ticket

    private fun clearVector() {
        while (root.firstChild != null) root.removeChild(root.firstChild!!)
    }

    private fun clearError() {
        pendingError = null
        errorDelivered = false
    }

    private fun reportError(code: String) {
        pendingError = code
        deliverPendingError()
    }

    private fun deliverPendingError() {
        if (destroyed || detached || errorDelivered) return
        val code = pendingError ?: return
        val callback = loadError ?: return
        errorDelivered = true
        callback.invoke(mapOf("errorCode" to code, "message" to "图标资源不可用或不符合受限 SVG 格式"))
    }

    override fun onRemoveFromParent(parent: Element) {
        detached = true
        generation++
        template = null
        clearError()
        clearVector()
    }

    override fun onAddToParent(parent: Element) {
        if (detached && !destroyed) {
            detached = false
            renderSource()
        }
    }

    override fun onDestroy() {
        destroyed = true
        generation++
        template = null
        loadError = null
        clearError()
        clearVector()
    }

    companion object {
        const val VIEW_NAME = "XzSvgView"
    }
}
