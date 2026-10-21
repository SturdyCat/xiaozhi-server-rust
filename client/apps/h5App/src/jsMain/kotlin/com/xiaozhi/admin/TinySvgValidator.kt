package com.xiaozhi.admin

import kotlinx.browser.document
import org.w3c.dom.Document
import org.w3c.dom.Element
import org.w3c.dom.Node
import kotlin.math.abs

/** Phase-one Tiny schema shared with XzSvgDocument; browser DOM does the vector rendering. */
internal object TinySvgValidator {
    private const val NS = "http://www.w3.org/2000/svg"
    private val number = Regex("""[+-]?(?:\d+(?:\.\d*)?|\.\d+)(?:[eE][+-]?\d+)?""")
    private val styleKeys = setOf("fill", "stroke", "color", "stroke-width", "stroke-linecap", "stroke-linejoin", "fill-opacity", "stroke-opacity", "fill-rule")
    private val rootKeys = setOf("xmlns", "width", "height", "viewBox", "preserveAspectRatio", "version")
    private val geometry = mapOf(
        "path" to setOf("d"), "rect" to setOf("x", "y", "width", "height", "rx", "ry"),
        "circle" to setOf("cx", "cy", "r"), "ellipse" to setOf("cx", "cy", "rx", "ry"),
        "line" to setOf("x1", "y1", "x2", "y2"), "polyline" to setOf("points"), "polygon" to setOf("points"),
    )

    fun parse(text: String): Element {
        require(text.isNotEmpty() && text.length <= 65536 && text.encodeToByteArray().size <= 65536)
        require(!text.contains("<!DOCTYPE", true) && !text.contains("<!ENTITY", true) && !text.contains('\u0000'))
        val parser: dynamic = js("new DOMParser()")
        val parsed = parser.parseFromString(text, "image/svg+xml").unsafeCast<Document>()
        require(parsed.getElementsByTagName("parsererror").length == 0)
        val source = parsed.documentElement ?: error("Missing SVG root")
        for (i in 0 until parsed.childNodes.length) {
            val node = parsed.childNodes.item(i) ?: continue
            if (node !== source) require(node.nodeType.toInt() == 8 || (node.nodeType.toInt() == 3 && node.textContent.orEmpty().isBlank()))
        }
        val builder = Builder()
        val root = builder.build(source, 1)
        builder.verifyBounds(root)
        return root
    }

    private class Builder {
        private var elements = 0
        private var pathCharacters = 0
        private val shapes = mutableListOf<Pair<Element, SvgBounds.Matrix>>()

        fun verifyBounds(root: Element) = SvgBounds.verify(root, shapes)

        fun build(source: Element, depth: Int, parentMatrix: SvgBounds.Matrix = SvgBounds.Matrix()): Element {
            require(++elements <= 512 && depth <= 32 && source.attributes.length <= 32)
            val name = source.localName
            val root = depth == 1
            require(source.namespaceURI == null || source.namespaceURI == NS)
            require(if (root) name == "svg" else name == "g" || geometry.containsKey(name))
            val attrs = linkedMapOf<String, String>()
            for (i in 0 until source.attributes.length) {
                val attr = source.attributes.item(i) ?: continue
                attrs[attr.name] = attr.value.trim()
            }
            attrs.remove("style")?.split(';')?.forEach { declaration ->
                if (declaration.isNotBlank()) {
                    val parts = declaration.split(':')
                    require(parts.size == 2)
                    val key = parts[0].trim()
                    require(key in styleKeys || key == "opacity")
                    attrs[key] = parts[1].trim()
                }
            }
            for ((key, value) in attrs) {
                when {
                    key in styleKeys -> validatePresentation(key, value)
                    key == "opacity" -> require(scalar(value) in 0.0..1.0)
                    key == "transform" -> validateTransformNumbers(value) // Affine bounds checked below.
                    key in geometry[name].orEmpty() -> Unit
                    root && key in rootKeys -> Unit
                    else -> error("Unsupported SVG attribute")
                }
            }
            val matrix = parentMatrix.then(SvgBounds.transform(attrs["transform"].orEmpty()))
            val clean = document.createElementNS(NS, name)
            for ((key, value) in attrs) clean.setAttribute(key, value)
            if (root) {
                require(attrs["xmlns"] == null || attrs["xmlns"] == NS)
                require(attrs["preserveAspectRatio"] == null || attrs["preserveAspectRatio"] in setOf("xMidYMid", "xMidYMid meet"))
                val width = attrs["width"]?.let(::scalar) ?: 0.0
                val height = attrs["height"]?.let(::scalar) ?: 0.0
                require(width >= 0 && height >= 0)
                val box = attrs["viewBox"]?.let(::numbers) ?: listOf(0.0, 0.0, width, height)
                require(box.size == 4 && box[2] >= 1e-6 && box[3] >= 1e-6)
                // Native fits the fallback width/height box too; materialize it for browser SVG.
                clean.setAttribute("viewBox", box.joinToString(" "))
            } else if (name in geometry) {
                pathCharacters += attrs["d"].orEmpty().length + attrs["points"].orEmpty().length
                require(pathCharacters <= 65536)
                validateGeometry(name, attrs)
                shapes.add(clean to matrix)
            }
            for (i in 0 until source.childNodes.length) {
                val child = source.childNodes.item(i) ?: continue
                when (child.nodeType.toInt()) {
                    1 -> {
                        require(root || name == "g")
                        clean.appendChild(build(child.unsafeCast<Element>(), depth + 1, matrix))
                    }
                    3 -> require(child.textContent.orEmpty().isBlank())
                    8 -> Unit
                    else -> error("Unsupported SVG content")
                }
            }
            return clean
        }
    }

    private fun scalar(value: String): Double {
        require(value.length <= 64 && number.matches(value))
        return value.toDouble().also { require(it.isFinite() && abs(it) <= 1e6) }
    }

    private fun numbers(value: String): List<Double> {
        require(value.length <= 32768)
        val cursor = NumericCursor(value)
        val result = mutableListOf<Double>()
        cursor.space()
        while (cursor.peek() != null) {
            require(result.size < 8192)
            result.add(cursor.readNumber())
            cursor.separator()
        }
        return result
    }

    /** A lexer only: one optional comma, anchored numbers, and single-character arc flags. */
    private class NumericCursor(private val value: String) {
        private var index = 0
        fun peek(): Char? = value.getOrNull(index)
        fun take(): Char = value[index++]
        fun space() { while (peek()?.let { it in " \t\r\n" } == true) index++ }
        fun separator(): Boolean {
            space()
            if (peek() != ',') return false
            take()
            space()
            require(peek() != null && peek() != ',')
            return true
        }
        fun readNumber(): Double {
            space()
            val match = TinySvgValidator.number.find(value, index) ?: error("Expected SVG number")
            require(match.range.first == index)
            index = match.range.last + 1
            return TinySvgValidator.scalar(match.value)
        }
        fun readFlag(): Double {
            space()
            require(peek() == '0' || peek() == '1')
            return if (take() == '0') 0.0 else 1.0
        }
    }

    private fun isCommand(value: Char?): Boolean = value != null && (value in 'A'..'Z' || value in 'a'..'z')

    private fun validateTransformNumbers(value: String) {
        require(value.length <= 4096)
        val operation = Regex("""([A-Za-z]+)[ \t\r\n]*\(([^()]*)\)""")
        var position = 0
        var first = true
        for (match in operation.findAll(value)) {
            val gap = value.substring(position, match.range.first).filterNot { it in " \t\r\n" }
            require(gap.isEmpty() || (!first && gap == ","))
            numbers(match.groupValues[2])
            position = match.range.last + 1
            first = false
        }
        require(value.substring(position).all { it in " \t\r\n" })
    }

    private fun validatePresentation(key: String, value: String) {
        if (value == "inherit") return
        when (key) {
            "fill", "stroke", "color" -> {
                val named = value in setOf("none", "currentColor", "black", "white", "red", "green", "blue", "transparent")
                val hex = value.startsWith('#') && value.length in setOf(4, 5, 7, 9) && value.drop(1).all { it in "0123456789abcdefABCDEF" }
                require(named || hex)
                require(key != "color" || value != "none")
            }
            "stroke-width" -> require(scalar(value) >= 0)
            "fill-opacity", "stroke-opacity" -> require(scalar(value) in 0.0..1.0)
            "stroke-linecap" -> require(value in setOf("butt", "round", "square"))
            "stroke-linejoin" -> require(value in setOf("miter", "round", "bevel"))
            "fill-rule" -> require(value in setOf("nonzero", "evenodd"))
        }
    }

    private fun validateGeometry(name: String, attrs: Map<String, String>) {
        when (name) {
            "path" -> validatePath(attrs["d"].orEmpty())
            "polyline", "polygon" -> {
                val points = numbers(attrs["points"].orEmpty())
                require(points.size >= 4 && points.size % 2 == 0)
            }
            else -> for (key in geometry.getValue(name)) {
                val value = attrs[key]?.let(::scalar) ?: 0.0
                if (key in setOf("width", "height", "r", "rx", "ry")) require(value >= 0)
            }
        }
    }

    /** Strict command/argument syntax only; painting and geometry remain browser-native. */
    private fun validatePath(value: String) {
        require(value.isNotEmpty() && value.length <= 32768)
        val cursor = NumericCursor(value)
        val arities = mapOf('M' to 2, 'L' to 2, 'H' to 1, 'V' to 1, 'C' to 6, 'S' to 4, 'Q' to 4, 'T' to 2, 'A' to 7, 'Z' to 0)
        var command: Char? = null
        var moved = false
        var operations = 0
        cursor.space()
        while (cursor.peek() != null) {
            require(++operations <= 8192)
            if (isCommand(cursor.peek())) command = cursor.take()
            val current = command ?: error("Expected SVG command")
            val kind = current.uppercaseChar()
            require(kind in arities && (moved || kind == 'M'))
            if (kind == 'Z') {
                command = null // Z cannot be followed by an implicit numeric group.
                cursor.space()
                continue
            }
            for (i in 0 until arities.getValue(kind)) {
                if (i > 0) cursor.separator()
                val n = if (kind == 'A' && (i == 3 || i == 4)) cursor.readFlag() else cursor.readNumber()
                if (kind == 'A' && (i == 0 || i == 1)) require(n >= 0)
            }
            if (kind == 'M') {
                moved = true
                command = if (current == 'm') 'l' else 'L'
            }
            // A comma after a complete group requires another numeric group, never EOF/command.
            if (cursor.separator()) require(!isCommand(cursor.peek()))
        }
        require(moved)
    }
}
