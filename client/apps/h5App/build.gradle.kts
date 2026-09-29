// h5App：Web(H5) 壳工程（对齐 xiaoya-player 同名工程）。
// 把「Web 渲染器 + 壳入口 Main.kt」编译为 h5App.js；业务页面产物 nativevue2.js
// 由 :shared:packLocalJSBundleRelease 产出（shared 持有 js 目标），两者汇入 web/ 目录。

plugins {
    kotlin("multiplatform")
}

// Kuikly 版本：唯一来源是 gradle.properties（与 shared 共用，保证 core 与 web 渲染器同版本）
val kuiklyVersion: String = property("kuiklyVersion") as String

// source map 开关（`-PwebSourceMap=false` 关闭，默认开）——与 shared 同一属性。
// 生产/Docker 构建关掉：镜像托管的是压缩后的 js、不做浏览器调试，关掉最省内存且无副作用。
val webSourceMap = (findProperty("webSourceMap") as String?)?.toBoolean() ?: true

kotlin {
    js(IR) {
        moduleName = "h5App"
        browser {
            webpackTask {
                outputFileName = "h5App.js"
            }
            commonWebpackConfig {
                // 保持 IIFE：避免 h5App.js 覆盖 window.com（nativevue2.js 注入的 Kuikly 桥接）
                output?.library = null
                devtool = if (webSourceMap) "source-map" else null
            }
        }
        binaries.executable()
    }
    sourceSets {
        val jsMain by getting {
            dependencies {
                // Kuikly Web 渲染器。⚠️ 真实坐标是 `com.tencent.kuikly-open.core-render-web:h5`
                // （artifactId 是 `h5`，非官方文档口径的 core-render-web-h5，也非裸 core-render-web
                // —— 镜像上 404，实测 Could not resolve）。版本与 shared 的 core 保持一致：
                // 渲染器 klib 经 window.com.tencent.kuikly.core 与业务 bundle 通信，错配属未定义组合。
                implementation("com.tencent.kuikly-open.core-render-web:h5:$kuiklyVersion")
            }
        }
    }
}

// ===== 产物汇聚：把业务 bundle 与壳产物收拢到 web/ =====
// 产物契约：web/ 下需同时存在 index.html（入库）、nativevue2.js（先载，业务包）与
// h5App.js（后载，壳）。server 端 [server].admin_dir 默认即指本目录（config.rs default_admin_dir）。
// ⚠️ 产物断言不可省：缺任一文件时页面白屏或 404，必须在构建期暴露而不是部署后。

private val webDir = layout.projectDirectory.dir("web")

/** shared 业务产物 zip → web/nativevue2.js */
val syncBusinessBundle by tasks.registering(Copy::class) {
    group = "kuikly"
    description = "把 shared 产出的 nativevue2.js 拷贝到 web/（供 index.html 先加载）"
    // 显式声明生产者依赖：产物 zip 由 Kuikly 插件的打包任务生成，
    // 仅按文件路径访问会触发 Gradle 的 implicit-dependency 校验失败。
    dependsOn(":shared:packLocalJSBundleRelease")
    val zipFile = rootProject.layout.projectDirectory
        .file("shared/build/outputs/kuikly/js/release/local/nativevue2.zip")
    from(providers.provider { zipFile.asFile }.map { zipTree(it) }) {
        include("nativevue2.js")
    }
    into(webDir)
    // shared 未构建时给出可操作提示，而不是静默产出空目录
    doFirst {
        check(zipFile.asFile.exists()) {
            "未找到 ${zipFile.asFile}，请先执行 ./gradlew :shared:packLocalJSBundleRelease"
        }
    }
}

/** 壳 webpack 产物 → web/h5App.js */
val syncShellBundle by tasks.registering(Copy::class) {
    group = "kuikly"
    description = "把 :apps:h5App 的 webpack 产物 h5App.js 拷贝到 web/（供 index.html 后加载）"
    dependsOn("jsBrowserProductionWebpack")
    // Kotlin 2.1 起 webpack 产物落在 kotlin-webpack/js/<target>（旧版本是 dist/js/<target>）
    from(layout.buildDirectory.dir("kotlin-webpack/js/productionExecutable")) {
        include("h5App.js")
    }
    into(webDir)
}

/** 一键产出可直接托管的 web/ 目录 */
tasks.register("publishWeb") {
    group = "kuikly"
    description = "构建 web 端全部产物并汇聚到 apps/h5App/web"
    dependsOn(syncBusinessBundle, syncShellBundle)
}
