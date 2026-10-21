package com.xiaozhi.admin

import kotlinx.browser.document
import org.w3c.dom.Element
import kotlin.js.Promise

/** Common SVG files are the sole source; the build generates the embedded registry. */
internal object TrustedSvgDocuments {
    private const val PREFIX = "assets://common/"
    private const val MAX_CACHE = 32
    private val pathPattern = Regex("(?:[A-Za-z0-9_-]+/)*[A-Za-z0-9_-]+\\.svg")
    private val cache = linkedMapOf<String, Element>()

    fun canonicalSource(src: String): String? {
        if (src.length > 240 || !src.startsWith(PREFIX)) return null
        return src.takeIf { pathPattern.matches(it.removePrefix(PREFIX)) }
    }

    /** Promise delivery preserves generation/unmount checks even for immediately available sources. */
    fun load(canonical: String): Promise<Element> = Promise { resolve, _ ->
        require(canonicalSource(canonical) == canonical)
        val hit = cache.remove(canonical)
        val template = hit ?: TinySvgValidator.parse(
            BundledSvgSources.bySource[canonical] ?: error("Missing bundled SVG"),
        )
        cache[canonical] = template
        while (cache.size > MAX_CACHE) cache.remove(cache.keys.first())
        resolve(template)
    }

    fun cloneForView(template: Element, tintIsSet: Boolean): Element {
        val svg = document.importNode(template, true).unsafeCast<Element>()
        if (tintIsSet) {
            // Tint only currentColor: literal fill/stroke stay untouched. Source color is
            // used only without explicit tint, matching the native layer renderer.
            svg.removeAttribute("color")
            val descendants = svg.querySelectorAll("*")
            for (i in 0 until descendants.length) descendants.item(i)?.unsafeCast<Element>()?.removeAttribute("color")
        }
        svg.setAttribute("width", "100%")
        svg.setAttribute("height", "100%")
        svg.setAttribute("preserveAspectRatio", "xMidYMid meet")
        svg.setAttribute("aria-hidden", "true")
        svg.setAttribute("focusable", "false")
        svg.setAttribute("style", "display:block;width:100%;height:100%;pointer-events:none")
        return svg
    }
}
