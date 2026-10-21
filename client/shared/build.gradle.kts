import com.tencent.kuikly.gradle.config.KuiklyConfig
import groovy.json.JsonSlurper
import java.security.MessageDigest
import org.jetbrains.kotlin.gradle.ExperimentalKotlinGradlePluginApi
import org.jetbrains.kotlin.gradle.dsl.JvmTarget
import javax.inject.Inject

plugins {
    kotlin("multiplatform")
    id("com.android.library")
    // Kuikly KSP：扫描 @Page 注解生成页面注册代码（KuiklyCoreEntry，按编译目标注入）
    id("com.google.devtools.ksp")
    // Kuikly 官方 Gradle 插件：跨端产物打包（web 端 packLocalJSBundle* 产 nativevue2.zip）。
    // 经根 buildscript classpath 引入（版本与 kuiklyVersion 双写同步，见根脚本注释）。
    id("com.tencent.kuikly-open.kuikly")
}

val kuiklyVersion: String = property("kuiklyVersion") as String

// source map 开关（`-PwebSourceMap=false` 关闭，默认开）——与 apps/h5App 同一属性：
// 本地开发留 map，浏览器报错能定位到 Kotlin 源码；生产/Docker 构建要关：
// 生成 .map 只抬高 webpack 峰值内存（xiaoya 实测 2.84GB → 3.22GB），ACR 构建机内存有限。
val webSourceMap = (findProperty("webSourceMap") as String?)?.toBoolean() ?: true

// Apple 平台 framework 的 bundleId（修复链接期 "Cannot infer a bundle ID" 警告；
// Framework DSL 无 bundleId 属性，经 binaryOption 映射为 -Xbinary=bundleId 编译参数）
val frameworkBundleId = "com.xiaozhi.admin.shared"

android {
    namespace = "com.xiaozhi.admin"
    compileSdk = 35
    defaultConfig {
        minSdk = 21
    }
    sourceSets.named("main") { assets.srcDir("src/commonMain/assets") }
}

kotlin {
    @OptIn(ExperimentalKotlinGradlePluginApi::class)
    androidTarget {
        compilerOptions {
            jvmTarget = JvmTarget.JVM_17
        }
    }

    // iOS：仅 arm64（iosArm64 真机 + iosSimulatorArm64 Apple Silicon 模拟器），不编译 x86_64（Intel 模拟器）
    listOf(
        iosArm64(),
        iosSimulatorArm64(),
    ).forEach {
        it.binaries.framework {
            baseName = "shared"
            isStatic = true
            binaryOption("bundleId", frameworkBundleId)
        }
    }

    // macOS（Apple Silicon，仅 macosArm64）：复用 iOS 渲染器 OpenKuiklyIOSRender。
    // 说明：Kuikly 官方 macOS 支持（Alpha）并非原生 AppKit 渲染器，而是把 iOS/UIKit 渲染层
    // 通过 Mac Catalyst（platform :osx）搬到 Mac 上跑——本质上就是「iPad/iOS App 的桌面兼容模式」。
    // 仅 arm64 单目标，故 macOS 专用代码直接放在 macosArm64Main 源集（默认层级模板已将其挂到 commonMain），
    // 由 src/macosArm64Main 承载 TestPage 与 XiaoZhiModule（测试功能，web 不暴露）。
    macosArm64().binaries.framework {
        baseName = "shared"
        isStatic = true
        binaryOption("bundleId", frameworkBundleId)
    }

    // Web/JS target：业务页面编译打包为 nativevue2.js（业务 bundle），由 :apps:h5App:publishWeb
    // 经 packLocalJSBundleRelease 的 zip 汇入 apps/h5App/web/，与壳 h5App.js 分开加载。
    // （历史注释曾称「shared 不声明 js 目标」——不成立：js 消费方无法解析无 js 目标的 KMP 模块，
    //   实测 :apps:h5App 的 jsNpmAggregated 解析 :shared 直接失败；xiaoya-player 同为
    //   shared js + h5App js 双目标共存，无 NodeJsRootPlugin 冲突。）
    js(IR) {
        outputModuleName.set("nativevue2")
        browser {
            webpackTask {
                mainOutputFileName.set("nativevue2.js")
            }
            commonWebpackConfig {
                // 不导出全局对象，只导出必要的入口函数
                output?.library = null
                devtool = if (webSourceMap) "source-map" else null
            }
        }
        binaries.executable()
    }

    sourceSets {
        val commonMain by getting {
            dependencies {
                // Kuikly 跨端核心：UI 框架。各宿主渲染器版本需与之对齐（都读 kuiklyVersion）。
                implementation("com.tencent.kuikly-open:core:$kuiklyVersion")
                // @Page 注解
                implementation("com.tencent.kuikly-open:core-annotations:$kuiklyVersion")
            }
        }
    }
}

dependencies {
    // Kuikly KSP 处理器：按编译目标注入（android / js / macosArm64）。
    // ⚠️ macosArm64 必须接入：KSP 生成的 KuiklyKotlinCoreEntry（页面注册入口）若缺失，
    //    macosApp 启动即崩——渲染器按名字找不到 entry（实测 "找不到对应
    //    KuiklyKotlinCoreEntry" → NSInternalInconsistencyException）。
    // iOS/macOS 目标不注入则页面无法注册；iOS 宿主如需启用再补 kspIosArm64 等。
    compileOnly("com.tencent.kuikly-open:core-ksp:$kuiklyVersion") {
        add("kspAndroid", this)
        add("kspJs", this)
        add("kspMacosArm64", this)
    }
}

ksp {
    arg("pageName", (project.properties["pageName"] as? String) ?: "")
    arg("pageNameList", (project.properties["pageNameList"] as? String) ?: "")
}

// Kuikly 插件配置：web 打包产物名与 KMP 插件 webpackTask#mainOutputFileName 一致
configure<KuiklyConfig> {
    js {
        outputName("nativevue2")
    }
}

// SVG-only resources. Drawing belongs to the dedicated vector view, never UIImage.
val verifyAdminIcons by tasks.registering {
    group = "verification"
    description = "Verify SVG-only mapping, currentColor, hashes, licence and native vector contract"
    val icons = layout.projectDirectory.dir("src/commonMain/assets/common/icons")
    val contracts = layout.projectDirectory.file("src/commonMain/kotlin/com/xiaozhi/admin/UiContracts.kt")
    val mappingFile = layout.projectDirectory.file("src/commonMain/kotlin/com/xiaozhi/admin/AdminIcons.kt")
    val nativeSvg = rootProject.layout.projectDirectory.file("apps/macosApp/XzSvgView.m")
    val appDelegate = rootProject.layout.projectDirectory.file("apps/macosApp/AppDelegate.m")
    inputs.dir(icons)
    inputs.files(contracts, mappingFile, nativeSvg, appDelegate)
    doLast {
        val enumBody = Regex("enum class IconToken\\s*\\{([^}]+)}")
            .find(contracts.asFile.readText())!!.groupValues[1]
        val tokenList = enumBody.split(',').map(String::trim).filter(String::isNotEmpty)
        val tokens = tokenList.toSet()
        val mappingText = mappingFile.asFile.readText()
        val pairs = Regex("IconToken\\.(\\w+) -> \"([a-z-]+)\"").findAll(mappingText).toList()
        val mappings = pairs.associate { it.groupValues[1] to it.groupValues[2] }
        check(tokens.isNotEmpty() && tokenList.size == tokens.size && pairs.size == tokens.size && mappings.values.toSet().size == tokens.size && tokens == mappings.keys) {
            "IconToken mapping must cover the nonempty canonical catalogue without duplicates"
        }
        check(mappingText.contains("\"assets://common/icons/source/\${assetName()}.svg\"")) { "Public icon URI must remain canonical SVG" }
        val nativeText = nativeSvg.asFile.readText()
        check(nativeText.contains("@implementation XzSvgView") && nativeText.contains("XzSvgDocument")) { "Dedicated native SVG vector view missing" }
        check(nativeText.contains("assets://common/") && nativeText.contains("URLByResolvingSymlinksInPath")) { "Native bundled-common SVG root policy missing" }
        check(!nativeText.contains("UIImage") && !nativeText.contains(".png", ignoreCase = true)) { "Native SVG must not use bitmap fallback" }
        check(!appDelegate.asFile.readText().contains("AdminAssetAdapter")) { "Obsolete bitmap adapter still registered" }
        listOf("h", "m").forEach { ext ->
            check(!rootProject.file("apps/macosApp/AdminAssetAdapter.$ext").exists()) { "Obsolete bitmap adapter source remains" }
        }
        val root = icons.asFile
        check(root.resolve("source").listFiles()!!.filter { it.extension == "svg" }.map { it.nameWithoutExtension }.toSet() == mappings.values.toSet())
        check(root.walkTopDown().none { it.isFile && it.extension.equals("png", ignoreCase = true) }) { "PNG assets forbidden in SVG-only icon catalogue" }
        check(!root.resolve("source/render.py").exists()) { "Obsolete bitmap generator remains" }
        val manifest = JsonSlurper().parse(root.resolve("manifest.json")) as Map<*, *>
        val entries = manifest["icons"] as List<*>
        check(entries.size == tokens.size && entries.map { (it as Map<*, *>)["token"] }.toSet() == tokens)
        check(manifest["license"] == "CC0-1.0" && manifest["format"] == "svg" && manifest["color"] == "currentColor")
        val renderer = manifest["renderer"] as Map<*, *>
        check(renderer["common"] == "XzSvgView" && renderer["web"] == "inline_svg" && renderer["native"] == "CAShapeLayer" && renderer["bitmapFallback"] == false)
        val license = root.resolve("LICENSE.txt")
        check(license.isFile && license.readText().contains("SPDX-License-Identifier: CC0-1.0"))
        fun hash(file: java.io.File): String = MessageDigest.getInstance("SHA-256")
            .digest(file.readBytes()).joinToString("") { "%02x".format(it) }
        check(hash(license) == manifest["licenseSha256"]) { "Icon licence hash mismatch" }
        entries.forEach { value ->
            val entry = value as Map<*, *>
            val name = entry["name"] as String
            check(mappings[entry["token"]] == name && entry.keys == setOf("token", "name", "svgSha256"))
            val svg = root.resolve("source/$name.svg")
            val text = svg.readText()
            check(text.contains("viewBox=\"0 0 24 24\"") && text.contains("currentColor"))
            val svgRoot = Regex("<svg\\b[^>]*>").find(text)?.value ?: error("SVG root missing: $name")
            check(!Regex("\\s(?:width|height)\\s*=").containsMatchIn(svgRoot)) { "SVG root dimensions must be container-driven: $name" }
            check(Regex("(?:fill|stroke)=\"([^\"]*)\"").findAll(text).all { it.groupValues[1] in setOf("none", "currentColor") }) { "Non-theme-aware SVG paint: $name" }
            check(svg.length() <= 65536 && hash(svg) == entry["svgSha256"]) { "SVG bounds/hash mismatch: $name" }
        }
        logger.lifecycle("Verified ${tokens.size} SVG-only icons: currentColor, SHA-256, licence and vector renderer")
    }
}
tasks.matching { it.name == "check" || it.name.startsWith("packLocalJSBundle") }.configureEach {
    dependsOn(verifyAdminIcons)
    if (name.startsWith("packLocalJSBundle")) {
        // Removed bitmap sources must invalidate an otherwise up-to-date archive.
        inputs.dir(layout.projectDirectory.dir("src/commonMain/assets/common/icons"))
    }
}

// ===== macOS Catalyst 补丁 =====
// Kotlin/Native 没有 macabi（Mac Catalyst）目标，macosArm64 静态框架的平台标记是 macOS；
// macosApp（Mac Catalyst）链接时 ld 直接拒绝：
//   ld: building for 'macCatalyst', but linking in object file ... built for 'macOS'
// 链接完成后用 scripts/patch_framework_macabi.py 把归档内各目标文件的
// LC_BUILD_VERSION 平台改写为 MacCatalyst(6)（幂等，详见脚本头注释）。
// 本机 vtool 不认 macabi 平台名（实测），故走脚本而非 vtool。
val patchFrameworkForCatalyst by tasks.registering(PatchFrameworkForCatalystTask::class) {
    group = "build"
    description = "把 macosArm64 静态框架平台标记改写为 Mac Catalyst（macabi），供 macosApp 链接"
    patchScript.set(rootProject.file("scripts/patch_framework_macabi.py"))
    frameworkBinaries.set(providers.provider {
        listOf("releaseFramework", "debugFramework").map { dir ->
            layout.buildDirectory.dir("bin/macosArm64/$dir").get()
                .file("shared.framework/Versions/A/shared")
        }
    })
}

// project.exec 已在 Gradle 9 移除：任务内执行外部命令改走注入的 ExecOperations
abstract class PatchFrameworkForCatalystTask : DefaultTask() {
    @get:Inject
    abstract val execOps: ExecOperations

    @get:InputFile
    @get:PathSensitive(PathSensitivity.NONE)
    abstract val patchScript: RegularFileProperty

    @get:Internal
    abstract val frameworkBinaries: ListProperty<RegularFile>

    @TaskAction
    fun patch() {
        val script = patchScript.get().asFile
        frameworkBinaries.get().forEach { bin ->
            if (bin.asFile.exists()) {
                execOps.exec { commandLine("python3", script.absolutePath, bin.asFile.absolutePath) }
            }
        }
    }
}
tasks.matching { it.name.startsWith("link") && it.name.contains("FrameworkMacosArm64") }.configureEach {
    finalizedBy(patchFrameworkForCatalyst)
}
