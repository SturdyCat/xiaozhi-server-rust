plugins {
    kotlin("multiplatform")
    // Kuikly Compose 的 @Composable 需要编译器插件（与 shared 同一套 Kotlin 2.1.21）。
    id("org.jetbrains.kotlin.plugin.compose")
    id("com.android.library")
}

// ===== 本模块是什么 =====
// Kuikly Compose 之上的**第三方组件承载层**，全部为上游源码 vendored 进本仓库：
//   1. Compose-Settings（Kuikly 适配版）：com.alorma.compose.settings.*（设置页磁贴）
//      - 上游 settings-kuikly 只声明 android / ios / js 三个目标，**没有 macosArm64**，
//        而本项目 macOS（Mac Catalyst）是主力端，故在此补齐目标。
//      - 上游未发布到任何公共仓库（3.2.0-kuikly 在 Maven Central / Tencent 镜像均 404），
//        只能源码引入。
//   2. kuiklyx-viewmodel：com.tencent.kuiklyx.viewmodel.*（Pager 级 ViewModel 生命周期）
//      - 上游同样只有 android / ios 目标，缺 js 与 macosArm64；源码为纯 commonMain（171 行，
//        只依赖 Kuikly core 的 Pager/PagerScope/IPagerEventObserver），补齐目标即可用。
//   3. com.tencent.kuikly.compose.material3 缺失组件的补丁（ListItem / SegmentedButton /
//      SegmentedListItem / ToggleButton / RadioButton）——上游 Kuikly Compose 2.29.2 的
//      material3 制品实测**不含**这些符号，属于真实缺位，不是重复定义。
//
// ⚠️ 本模块只做「承载与编译适配」，不做视觉改造：组件样式由上游默认值决定。
// 项目自己的设计 token（AdminColors/AdminType）在 shared 侧的 Compose 页面里通过参数注入。

val kuiklyVersion: String = property("kuiklyVersion") as String

android {
    namespace = "com.xiaozhi.admin.compose"
    compileSdk = 35
    defaultConfig {
        minSdk = 21
    }
}

kotlin {
    androidTarget {
        compilerOptions {
            jvmTarget = org.jetbrains.kotlin.gradle.dsl.JvmTarget.JVM_17
        }
    }

    // 目标集与 :shared 保持一致：库若缺某一目标，:shared 在该目标上会解析失败。
    iosArm64()
    iosSimulatorArm64()
    macosArm64()

    // 库模块只需参与编译与打包，不产出可执行 bundle（executable 由 :shared 的 js 目标负责）。
    js(IR) {
        browser()
    }

    sourceSets {
        val commonMain by getting {
            dependencies {
                // 用 api：组件签名里直接出现 Compose 类型（Modifier / Shape / Dp / ListItemColors），
                // 消费方 :shared 必须能在自己的编译类路径上看到它们。
                api("com.tencent.kuikly-open:core:$kuiklyVersion")
                api("com.tencent.kuikly-open:compose:$kuiklyVersion")
            }
        }
    }
}
