import org.jetbrains.kotlin.gradle.ExperimentalKotlinGradlePluginApi

plugins {
    kotlin("multiplatform")
    id("com.android.library")
}

val kuiklyVersion: String = property("kuiklyVersion") as String

android {
    namespace = "com.xiaozhi.admin"
    compileSdk = 35
    defaultConfig {
        minSdk = 21
    }
}

kotlin {
    @OptIn(ExperimentalKotlinGradlePluginApi::class)
    androidTarget {
        compilations.all {
            kotlinOptions {
                jvmTarget = "17"
            }
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
    }

    // 注意：shared 不声明 js 目标。Web(H5) 构建由 apps/h5App 这个独立的 Kotlin/JS 宿主模块负责，
    // 它通过 implementation(project(":shared")) 复用 commonMain 的 ConfigPage 等代码。
    // 若 shared 也声明 js 目标，会与 h5App 的 Kotlin/JS 插件在根工程重复加载 NodeJsRootPlugin 而报错。

    sourceSets {
        val commonMain by getting {
            dependencies {
                // Kuikly 跨端核心：UI 框架 + 注解（@Page）。各宿主渲染器版本需与之对齐。
                implementation("com.tencent.kuikly-open:core:$kuiklyVersion")
            }
        }
    }
}
