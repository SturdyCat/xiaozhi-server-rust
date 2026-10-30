// h5App：Web 壳 h5App.js + shared 业务 nativevue2.js（入口先业务、后壳）。
import java.security.MessageDigest
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

// ===== 官方 EchoMuse 设备管理页（原样 vendor，见 em/README.md）=====
// 只有 web 用：作为独立文档挂到 /em/，由 Rust 的 ServeDir 同源托管；与 Kuikly 页零冲突。
private val emDir = layout.projectDirectory.dir("em")

/**
 * 生成物与源码的一致性闸门：`dashboard.js` 首行的 `src-sha256` 必须等于当前
 * `dashboard.jsx` 的摘要，否则说明"改了 jsx 却忘了重跑 tools/build_em_dashboard.sh"，
 * 此时让 publishWeb 直接失败，而不是把上一版 UI 悄悄发出去。
 */
val verifyEmBundle by tasks.registering {
    group = "verification"
    description = "Verify the vendored EchoMuse UI source matches its .jsx and host assets exist"
    val jsx = emDir.file("static/dashboard.jsx")
    val js = emDir.file("static/dashboard.js")
    inputs.files(jsx, js)
    doLast {
        val digest = MessageDigest.getInstance("SHA-256")
            .digest(jsx.asFile.readBytes())
            .joinToString("") { String.format("%02x", it) }
        val head = js.asFile.readText().take(512)
        check(head.contains("em-bundle src-sha256=$digest")) {
            "dashboard.js 不是由当前 dashboard.jsx 编译出来的（期望横幅 em-bundle src-sha256=$digest）；" +
                "在仓库根执行 tools/build_em_dashboard.sh 重新生成"
        }
        check(head.contains("em-host: App auto-mount removed") || js.asFile.readText().contains("em-host: App auto-mount removed")) {
            "dashboard.js 缺少 App 自动挂载摘除标记：host.js 会与官方 App 双挂载"
        }
        listOf("adapter.js", "host.js", "dashboard.jsx").forEach {
            check(emDir.file("static/$it").asFile.isFile) { "缺少 static/$it" }
        }
        // 第三方库与字体走 CDN（index.html 里带 SRI），仓库不再 vendor 它们。
        check(emDir.file("LICENSE").asFile.isFile) { "缺少上游 LICENSE（MIT 要求随分发保留）" }
    }
}

/** 拷进 web/em/：不含 jsx 源码与仓库文档（下发只需要可执行的三份 js，第三方库走 CDN）。 */
val syncEmAssets by tasks.registering(Copy::class) {
    group = "kuikly"
    description = "Publish the EchoMuse device-management page (upstream UI source + our adapter/host) to web/em"
    from(emDir) { exclude("README.md", "README.sha256", "LICENSE", "static/dashboard.jsx") }
    into(webDir.dir("em"))
}

tasks.register("publishWeb") {
    group = "kuikly"
    description = "Build and publish the Web application bundles"
    dependsOn(":shared:verifyAdminIcons", verifyEmBundle, syncBusinessBundle, syncShellBundle, syncEmAssets)
}
