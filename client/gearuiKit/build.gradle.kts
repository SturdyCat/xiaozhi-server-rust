// GearUI Kit（Apache-2.0）源码 vendored：上游只发布 android/ios/js，缺 macosArm64，
// 而本项目 macOS(Mac Catalyst) 是主力端，故源码引入并自补 macosArm64 目标。
// 与上游差异（全部为构建适配，无视觉/行为改造）：
//   ① 去 org.jetbrains.compose 插件与 implementation(compose.runtime)
//      —— 实测 com.tencent.kuikly-open:compose 的 macosArm64 变体已 api 依赖
//         org.jetbrains.compose.runtime:runtime:1.7.3（见其 .module），无需再声明；
//      且 gearui 源码零处使用 org.jetbrains.compose.* / compose.resources。
//   ② 去 androidx.annotation 依赖 —— 源码零处使用（grep 0 命中）。
//   ③ 去 com.vanniktech.maven.publish —— 本地模块不发布。
//   ④ 去 ohosArm64Main / commonTest / androidUnitTest 三个源集（本项目不用）。
//   ⑤ 加 macosArm64()；去 iosX64()（本项目不含 Intel 模拟器）。
//   ⑥ compileSdk 36→35、jvmTarget 1.8→17（与 :shared 对齐）。
//   ⑦ 用默认层级模板（不手动创建 iosMain），故不得引入
//      kotlin.mpp.applyDefaultHierarchyTemplate=false（上游为 false，我们保持默认）。
// 许可与来源见同目录 LICENSE / NOTICE / THIRD_PARTY_NOTICES.md。
// ⚠️ 升级上游时按 THIRD_PARTY_NOTICES.md 的路径重新拉取比对，不要手改组件代码。
plugins {
    kotlin("multiplatform")
    // Kuikly Compose 的 @Composable 需要编译器插件（版本由根 build.gradle.kts 统一锁定 2.1.21）。
    id("org.jetbrains.kotlin.plugin.compose")
    id("com.android.library")
}

val kuiklyVersion: String = property("kuiklyVersion") as String

android {
    namespace = "com.gearui.kit"
    compileSdk = 35
    defaultConfig {
        minSdk = 21
    }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
}

kotlin {
    androidTarget {
        compilerOptions {
            jvmTarget = org.jetbrains.kotlin.gradle.dsl.JvmTarget.JVM_17
        }
    }

    // 目标集与 :shared 严格对齐 —— 库缺任一目标，:shared 在该目标上解析失败。
    iosArm64()
    iosSimulatorArm64()
    macosArm64()

    // 库模块只需参与编译与打包，不产出可执行 bundle（executable 由 :shared 的 js 目标负责）。
    js(IR) {
        browser()
    }

    targets.all {
        compilations.all {
            // 上游原样保留的编译参数：组件里有 opt-in 的 Kuikly API。
            // 用 compilerOptions DSL（等价于上游的 kotlinOptions.freeCompilerArgs，
            // 但不再触发 'kotlinOptions is deprecated' 警告；参数值一字不改）。
            compileTaskProvider.configure {
                compilerOptions {
                    freeCompilerArgs.add("-opt-in=kotlin.RequiresOptIn")
                }
            }
        }
    }

    sourceSets {
        val commonMain by getting {
            dependencies {
                // 用 api：组件签名里直接出现 Compose/Dp/Shape 类型，消费方 :shared 必须可见。
                api("com.tencent.kuikly-open:core:$kuiklyVersion")
                api("com.tencent.kuikly-open:compose:$kuiklyVersion")
            }
        }
    }
}
