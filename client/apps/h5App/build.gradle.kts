import java.nio.charset.StandardCharsets
import java.nio.file.Files
import java.nio.file.Paths

plugins {
    // Kotlin Multiplatform（JS 目标）
    kotlin("multiplatform")
}

val kuiklyVersion: String = property("kuiklyVersion") as String

repositories {
    // 本地 maven 优先（开发期用本地 SNAPSHOT 时）
    mavenLocal()
}

kotlin {
    js(IR) {
        browser {
            webpackTask {
                outputFileName = "nativevue2.js"
            }
            commonWebpackConfig {
                // 不导出全局对象，仅暴露必要入口方法
                output?.library = null
            }
        }
        binaries.executable()
    }

    sourceSets {
        val jsMain by getting {
            dependencies {
                // 跨端业务（shared 模块，产出 nativevue2.js）
                implementation(project(":shared"))
                // Kuikly Web 渲染器（版本与 shared 的 core 保持一致）
                implementation("com.tencent.kuikly-open:core-render-web:$kuiklyVersion")
            }
        }
    }
}

// 将构建产物拷贝到可直接静态托管的目录（server 的 web 根 / 部署用）
val pageOutDir = Paths.get(project.rootDir.absolutePath, "apps", "h5App", "build", "dist", "js", "productionExecutable")
project.afterEvaluate {
    tasks.register("publishWeb") {
        group = "kuikly"
        dependsOn("jsBrowserDistribution")
        doLast {
            val src = pageOutDir.resolve("nativevue2.js").toFile()
            val dest = pageOutDir.resolve("page").toFile().apply { mkdirs() }
            src.copyTo(dest.resolve("nativevue2.js"), overwrite = true)
            println("publishWeb -> ${dest.resolve("nativevue2.js")}")
        }
    }
}
