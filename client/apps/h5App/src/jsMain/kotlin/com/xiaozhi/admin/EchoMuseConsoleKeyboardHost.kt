package com.xiaozhi.admin

import com.tencent.kuikly.core.render.web.export.IKuiklyRenderViewExport
import com.tencent.kuikly.core.render.web.ktx.KuiklyRenderCallback
import kotlinx.browser.document
import org.w3c.dom.Element
import org.w3c.dom.HTMLDivElement
import org.w3c.dom.HTMLTextAreaElement

/** A one-pixel native textarea captures IME/paste; no global key handlers and no iframe. */
class EchoMuseConsoleKeyboardHost : IKuiklyRenderViewExport {
    override val ele = document.createElement("div") as HTMLDivElement
    private val input = (document.createElement("textarea") as HTMLTextAreaElement).apply {
        setAttribute("aria-label", "EchoMuse 控制台键盘输入")
        setAttribute("autocomplete", "off"); setAttribute("autocapitalize", "off")
        setAttribute("spellcheck", "false"); setAttribute("wrap", "off")
        style.cssText = "position:absolute;width:1px;height:1px;padding:0;border:0;opacity:0;resize:none;overflow:hidden;"
    }
    private var callback: KuiklyRenderCallback? = null
    private var enabled = false
    private var composing = false
    private var disposed = false
    // A view-scoped document listener is necessary: non-focusable Kuikly text/canvas
    // does not naturally blur a textarea. It never cancels or redirects the outside click.
    private val outsidePointer: (org.w3c.dom.events.Event) -> Unit = { event ->
        val target = event.target as? org.w3c.dom.Node
        val viewport = ele.parentElement
        if (target != null && viewport != null && !viewport.contains(target)) input.blur()
    }
    init {
        ele.appendChild(input)
        document.addEventListener("pointerdown", outsidePointer, true)
        input.addEventListener("focus", { if (!disposed) callback?.invoke(mapOf("kind" to "focus", "focused" to true)) })
        input.addEventListener("blur", {
            composing = false; input.value = ""
            if (!disposed) callback?.invoke(mapOf("kind" to "focus", "focused" to false))
        })
        input.addEventListener("keydown", { event ->
            val e = event.asDynamic()
            val key = e.key as? String ?: ""
            val ctrl = e.ctrlKey == true; val alt = e.altKey == true
            val shift = e.shiftKey == true; val meta = e.metaKey == true
            if (enabled && !composing && e.isComposing != true && !meta && !(ctrl && shift && key.lowercase() in setOf("c", "v"))) {
                if (key in special || ((ctrl || alt) && key.length == 1 && !(ctrl && alt))) {
                    event.preventDefault(); event.stopPropagation()
                    emit(mapOf("kind" to "key", "key" to key, "ctrl" to ctrl, "alt" to alt, "shift" to shift, "meta" to meta))
                }
            }
        })
        input.addEventListener("compositionstart", { composing = true })
        input.addEventListener("compositionend", { event ->
            composing = false
            val data = event.asDynamic().data as? String ?: input.value
            emit(mapOf("kind" to "text", "data" to data)); input.value = ""
        })
        input.addEventListener("input", { event ->
            if (!composing && event.asDynamic().isComposing != true) {
                val text = input.value; input.value = ""
                if (text.isNotEmpty()) emit(mapOf("kind" to "text", "data" to text))
            }
        })
        input.addEventListener("paste", { event ->
            event.preventDefault(); event.stopPropagation()
            val text = event.asDynamic().clipboardData?.getData("text/plain") as? String ?: ""
            emit(mapOf("kind" to "paste", "data" to text))
        })
    }
    override fun setProp(propKey: String, propValue: Any): Boolean = when (propKey) {
        "onInput" -> { callback = propValue as? KuiklyRenderCallback; true }
        "enabled" -> {
            val active = propValue == true || propValue == 1
            if (active != enabled) {
                enabled = active; input.disabled = !active
                if (!active) { composing = false; input.value = ""; input.blur() }
                // Availability is not a focus request. Only an explicit terminal click focuses it.
            }
            true
        }
        else -> super.setProp(propKey, propValue)
    }
    override fun call(method: String, params: String?, callback: KuiklyRenderCallback?): Any? {
        when (method) { "focus" -> focus(); "dispose" -> dispose(); else -> return super.call(method, params, callback) }
        return null
    }
    private fun focus() { if (!disposed && enabled) input.focus() }
    private fun emit(value: Map<String, Any>) { if (enabled && !disposed) callback?.invoke(value) }
    private fun dispose() {
        disposed = true; enabled = false; composing = false
        document.removeEventListener("pointerdown", outsidePointer, true)
        input.value = ""; input.blur(); callback = null
    }
    override fun onRemoveFromParent(parent: Element) = dispose()
    override fun onDestroy() = dispose()
    companion object {
        private val special = setOf("Enter", "Backspace", "Tab", "Escape", "ArrowUp", "ArrowDown", "ArrowLeft", "ArrowRight",
            "Home", "End", "Insert", "Delete", "PageUp", "PageDown", "F1", "F2", "F3", "F4", "F5", "F6", "F7", "F8", "F9", "F10", "F11", "F12")
    }
}
