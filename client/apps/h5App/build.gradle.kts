// h5App：Web 壳 h5App.js + shared 业务 nativevue2.js（入口先业务、后壳）。
import java.util.zip.ZipFile

plugins {
    kotlin("multiplatform")
}

val kuiklyVersion: String = property("kuiklyVersion") as String
val webSourceMap = (findProperty("webSourceMap") as String?)?.toBoolean() ?: true
val commonSvgRoot = rootProject.layout.projectDirectory.dir("shared/src/commonMain/assets/common")
val generatedSvgSources = layout.buildDirectory.dir("generated/svg/kotlin")

/** Canonical common resources are the only SVG source; Web embeds strings, never copies files. */
val generateBundledSvgSources by tasks.registering {
    group = "kuikly"
    description = "Embed bounded canonical common SVG sources in the H5 Kotlin registry"
    dependsOn(":shared:verifyAdminIcons")
    val sources = commonSvgRoot.asFileTree.matching { include("**/*.svg") }
    val output = generatedSvgSources.map { it.file("com/xiaozhi/admin/BundledSvgSources.kt") }
    inputs.files(sources).withPathSensitivity(PathSensitivity.RELATIVE)
    outputs.file(output)
    doLast {
        val root = commonSvgRoot.asFile.absoluteFile.normalize()
        check(root.canonicalFile == root) { "SVG source root must not be aliased" }
        val files = sources.files.sortedBy { it.relativeTo(root).invariantSeparatorsPath }
        check(files.isNotEmpty()) { "No common SVG sources found" }
        var total = 0L
        fun quoted(text: String): String = buildString {
            append('"')
            text.forEach { ch ->
                when (ch) {
                    '\\', '"', '$' -> { append('\\'); append(ch) }
                    '\n' -> append("\\n")
                    '\r' -> append("\\r")
                    '\t' -> append("\\t")
                    else -> if (ch.code !in 32..126) {
                        append("\\u"); append(ch.code.toString(16).padStart(4, '0'))
                    } else append(ch)
                }
            }
            append('"')
        }
        val content = buildString {
            appendLine("// Generated from canonical shared common SVGs. DO NOT EDIT.")
            appendLine("package com.xiaozhi.admin")
            appendLine()
            appendLine("internal object BundledSvgSources {")
            appendLine("    val bySource: Map<String, String> = mapOf(")
            files.forEach { file ->
                val relative = file.relativeTo(root).invariantSeparatorsPath
                check(Regex("(?:[A-Za-z0-9_-]+/)*[A-Za-z0-9_-]+\\.svg").matches(relative)) { "Unsafe bundled SVG path: $relative" }
                check(file.canonicalFile == file.absoluteFile.normalize()) { "Aliased SVG source: $relative" }
                check(file.length() <= 65536) { "SVG exceeds 64KiB: $relative" }
                val data = file.readBytes()
                total += data.size
                check(data.size <= 65536 && total <= 2L * 1024 * 1024) { "Common SVG source budget exceeded" }
                val xml = data.toString(Charsets.UTF_8)
                check(xml.toByteArray(Charsets.UTF_8).contentEquals(data)) { "SVG is not valid UTF-8: $relative" }
                append("        "); append(quoted("assets://common/$relative")); append(" to "); append(quoted(xml)); appendLine(",")
            }
            appendLine("    )")
            appendLine("}")
        }
        output.get().asFile.apply { parentFile.mkdirs(); writeText(content, Charsets.UTF_8) }
        logger.lifecycle("Embedded ${files.size} canonical SVGs ($total bytes)")
    }
}

kotlin {
    js(IR) {
        outputModuleName.set("h5App")
        browser {
            webpackTask { mainOutputFileName.set("h5App.js") }
            commonWebpackConfig {
                // 保持 IIFE：不覆盖 nativevue2.js 的 window.com 桥接。
                output?.library = null
                devtool = if (webSourceMap) "source-map" else null
            }
        }
        binaries.executable()
    }
    sourceSets {
        val jsMain by getting {
            kotlin.srcDir(generatedSvgSources)
            dependencies {
                implementation("com.tencent.kuikly-open.core-render-web:h5:$kuiklyVersion")
            }
        }
    }
}
tasks.matching { it.name == "compileKotlinJs" }.configureEach { dependsOn(generateBundledSvgSources) }

// Web hosts only the two bundles and index; canonical SVG XML is embedded in h5App.js.
private val webDir = layout.projectDirectory.dir("web")
val syncBusinessBundle by tasks.registering(Copy::class) {
    group = "kuikly"
    description = "Publish shared nativevue2.js without extracting its native asset files"
    dependsOn(":shared:packLocalJSBundleRelease")
    val zipFile = rootProject.layout.projectDirectory.file("shared/build/outputs/kuikly/js/release/local/nativevue2.zip")
    from(providers.provider { zipFile.asFile }.map { zipTree(it) }) { include("nativevue2.js") }
    into(webDir)
    doFirst {
        check(zipFile.asFile.exists()) { "Missing shared nativevue2.zip; run :shared:packLocalJSBundleRelease" }
        ZipFile(zipFile.asFile).use { archive ->
            check(archive.entries().asSequence().none { entry ->
                entry.name.contains("common/icons/") &&
                    (entry.name.endsWith(".png", ignoreCase = true) || entry.name.endsWith("/source/render.py"))
            }) { "Business bundle contains obsolete bitmap icon resources; rebuild its asset archive" }
        }
    }
}
val syncShellBundle by tasks.registering(Copy::class) {
    group = "kuikly"
    description = "Publish H5 shell and its generated embedded SVG registry"
    dependsOn("jsBrowserProductionWebpack")
    from(layout.buildDirectory.dir("kotlin-webpack/js/productionExecutable")) { include("h5App.js") }
    into(webDir)
}

/** Backwards-compatible task name; cleanup only, deliberately NOT Copy or broad Sync. */
val syncCommonAssets by tasks.registering {
    group = "kuikly"
    description = "Remove only known obsolete copied Web admin icons; never publish duplicate SVGs"
    val obsolete = listOf("save", "delete", "menu", "chevron-down", "chevron-up", "chevron-right", "play", "stop", "refresh", "check", "close", "settings", "microphone", "waveform", "text", "intent", "model", "speaker", "info")
    doLast {
        val root = webDir.asFile.canonicalFile
        val iconDir = root.resolve("assets/common/icons").absoluteFile.normalize()
        check(iconDir.canonicalFile == iconDir) { "Refusing legacy icon cleanup through an aliased directory" }
        val relativePaths = obsolete.flatMap { listOf("$it.png", "source/$it.svg") } +
            listOf("manifest.json", "LICENSE.txt", "source/render.py")
        relativePaths.forEach { relative ->
            val target = iconDir.resolve(relative).absoluteFile.normalize()
            check(target.parentFile == iconDir || target.parentFile == iconDir.resolve("source")) { "Unexpected managed legacy path: $target" }
            check(target.canonicalFile == target && target.parentFile.canonicalFile == target.parentFile) { "Refusing aliased legacy icon: $target" }
            if (target.exists()) check(target.isFile && target.delete()) { "Could not remove obsolete copied icon: $target" }
        }
    }
}

tasks.register("publishWeb") {
    group = "kuikly"
    description = "Build and publish the single-source embedded-SVG Web application"
    dependsOn(syncBusinessBundle, syncShellBundle, syncCommonAssets)
}
