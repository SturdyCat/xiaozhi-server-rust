// h5App：Web 壳 h5App.js + shared 业务 nativevue2.js（入口先业务、后壳）。
import java.util.zip.ZipFile

plugins {
    kotlin("multiplatform")
}

val kuiklyVersion: String = property("kuiklyVersion") as String
val webSourceMap = (findProperty("webSourceMap") as String?)?.toBoolean() ?: true

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
            dependencies {
                implementation("com.tencent.kuikly-open.core-render-web:h5:$kuiklyVersion")
            }
        }
    }
}

// Web hosts only the two bundles and index; admin icons are embedded vector
// pathData drawn by shared Canvas (AdminIcons.kt), never shipped resource files.
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
    description = "Publish H5 shell bundle"
    dependsOn("jsBrowserProductionWebpack")
    from(layout.buildDirectory.dir("kotlin-webpack/js/productionExecutable")) { include("h5App.js") }
    into(webDir)
}

tasks.register("publishWeb") {
    group = "kuikly"
    description = "Build and publish the Web application bundles"
    dependsOn(":shared:verifyAdminIcons", syncBusinessBundle, syncShellBundle)
}
