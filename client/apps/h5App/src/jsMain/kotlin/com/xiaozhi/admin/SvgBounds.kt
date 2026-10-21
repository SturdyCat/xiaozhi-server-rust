package com.xiaozhi.admin

import kotlinx.browser.document
import org.w3c.dom.Element
import kotlin.math.PI
import kotlin.math.abs
import kotlin.math.cos
import kotlin.math.sin
import kotlin.math.tan

/** Numeric safety checks only; all SVG geometry is interpreted by the browser, not a new renderer. */
internal object SvgBounds {
    class Matrix(
        val a: Double = 1.0, val b: Double = 0.0, val c: Double = 0.0,
        val d: Double = 1.0, val e: Double = 0.0, val f: Double = 0.0,
    ) {
        fun then(r: Matrix): Matrix = Matrix(
            a * r.a + c * r.b, b * r.a + d * r.b,
            a * r.c + c * r.d, b * r.c + d * r.d,
            a * r.e + c * r.f + e, b * r.e + d * r.f + f,
        ).also { require(listOf(it.a, it.b, it.c, it.d, it.e, it.f).all(::safe)) }

        fun checkPoint(x: Double, y: Double) {
            require(safe(a * x + c * y + e) && safe(b * x + d * y + f))
        }
    }

    private val numeric = Regex("""[+-]?(?:\d+(?:\.\d*)?|\.\d+)(?:[eE][+-]?\d+)?""")
    private fun safe(value: Double): Boolean = value.isFinite() && abs(value) <= 1e6
    private fun separators(value: String): Boolean = value.all { it.isWhitespace() || it == ',' }

    fun transform(value: String): Matrix {
        require(value.length <= 4096)
        val operation = Regex("""([A-Za-z]+)\s*\(([^()]*)\)""")
        var result = Matrix()
        var cursor = 0
        var count = 0
        for (match in operation.findAll(value)) {
            require(separators(value.substring(cursor, match.range.first)) && ++count <= 32)
            val params = match.groupValues[2]
            val values = mutableListOf<Double>()
            var position = 0
            for (number in numeric.findAll(params)) {
                require(separators(params.substring(position, number.range.first)))
                values.add(number.value.toDouble().also { require(safe(it)) })
                position = number.range.last + 1
            }
            require(separators(params.substring(position)))
            val step = when (match.groupValues[1]) {
                "matrix" -> {
                    require(values.size == 6)
                    Matrix(values[0], values[1], values[2], values[3], values[4], values[5])
                }
                "translate" -> {
                    require(values.size in 1..2)
                    Matrix(e = values[0], f = values.getOrElse(1) { 0.0 })
                }
                "scale" -> {
                    require(values.size in 1..2)
                    Matrix(a = values[0], d = values.getOrElse(1) { values[0] })
                }
                "rotate" -> {
                    require(values.size == 1 || values.size == 3)
                    val angle = values[0] * PI / 180.0
                    val rotate = Matrix(a = cos(angle), b = sin(angle), c = -sin(angle), d = cos(angle))
                    if (values.size == 1) rotate else Matrix(e = values[1], f = values[2])
                        .then(rotate).then(Matrix(e = -values[1], f = -values[2]))
                }
                "skewX", "skewY" -> {
                    require(values.size == 1)
                    val skew = tan(values[0] * PI / 180.0)
                    require(safe(skew))
                    if (match.groupValues[1] == "skewX") Matrix(c = skew) else Matrix(b = skew)
                }
                else -> error("Unsupported SVG transform")
            }
            result = result.then(step)
            cursor = match.range.last + 1
        }
        require(separators(value.substring(cursor)))
        return result
    }

    fun verify(root: Element, shapes: List<Pair<Element, Matrix>>) {
        // Only already-sanitized, script-free vector DOM is measured. Opacity zero keeps
        // native getBBox available; display:none would make some browsers report zero boxes.
        root.setAttribute("style", "position:fixed;left:-100000px;top:-100000px;width:1px;height:1px;opacity:0;pointer-events:none")
        root.setAttribute("aria-hidden", "true")
        val parent = document.body ?: error("SVG measurement unavailable")
        try {
            parent.appendChild(root)
            for ((shape, matrix) in shapes) {
                val box: dynamic = shape.asDynamic().getBBox()
                val x = box.x as Double
                val y = box.y as Double
                val width = box.width as Double
                val height = box.height as Double
                require(listOf(x, y, width, height).all { it.isFinite() } && width >= 0 && height >= 0)
                // Conservative transformed geometry bounds, excluding stroke just like CGPath.
                matrix.checkPoint(x, y)
                matrix.checkPoint(x + width, y)
                matrix.checkPoint(x, y + height)
                matrix.checkPoint(x + width, y + height)
            }
        } finally {
            root.parentNode?.removeChild(root)
            root.removeAttribute("style")
            root.removeAttribute("aria-hidden")
        }
    }
}
