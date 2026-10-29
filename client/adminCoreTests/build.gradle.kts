import org.jetbrains.kotlin.gradle.dsl.JvmTarget

// Compile the production pure models, never a copied implementation or native UI stub.
plugins {
    kotlin("multiplatform")
    // FilesModel.kt 的 @Serializable 线格式解析需要编译器插件（与 :shared 同版本，见根 build）
    kotlin("plugin.serialization")
}

kotlin {
    jvm {
        compilerOptions.jvmTarget.set(JvmTarget.JVM_17)
    }
    sourceSets {
        val commonMain by getting {
            dependencies {
                // TimeFormat.kt：公历换算与系统时区取值
                implementation("org.jetbrains.kotlinx:kotlinx-datetime:0.6.2")
                // FilesModel.kt 清单/错误回包的 JSON 解码（替代此前手写的 140 行解析器）
                implementation("org.jetbrains.kotlinx:kotlinx-serialization-json:1.8.1")
            }
            kotlin.srcDir(rootProject.file("shared/src/commonMain/kotlin"))
            // T03 搬迁后纯契约位于 contracts/（零 import，不依赖 Kuikly）
            kotlin.include("com/xiaozhi/admin/contracts/UiContracts.kt")
            // 导航常量表（零依赖纯数据）：2026-10-29 范围削减后 UiContractsTest 需要断言
            // 「12 项 → 8 项」的导航契约，故纳入编译清单
            kotlin.include("com/xiaozhi/admin/contracts/AdminNav.kt")
            kotlin.include("com/xiaozhi/admin/EchoMuseWakeWord.kt")
            kotlin.include("com/xiaozhi/admin/FilesModel.kt")
            // 时间展示口径（formatEpochSeconds 的格式契约）：依赖 kotlinx-datetime，
            // 但无 Kuikly 依赖，直接由 TimeFormatTest 以固定时区钉住输出。
            kotlin.include("com/xiaozhi/admin/TimeFormat.kt")
            kotlin.include("com/xiaozhi/admin/PipelinePanels.kt")
            kotlin.include("com/xiaozhi/admin/PipelineFlow.kt")
            kotlin.include("com/xiaozhi/admin/PipelineWorkflow.kt")
            kotlin.include("com/xiaozhi/admin/NodeTestModels.kt")
            kotlin.include("com/xiaozhi/admin/ConfigSections.kt")
            // 控制台纯文本管线（ANSI 剥离 / \r 覆盖 / 双上限）：无 Kuikly 依赖，
            // 直接被 EchoMuseConsoleTextTest 直驱（设计 §7-T03）。
            kotlin.include("com/xiaozhi/admin/EchoMuseConsoleText.kt")
            // 控制台协议字面量 + 文案 + `closed` 文案纯映射 `ConsoleHint`：同样无 Kuikly
            // 依赖，被 EchoMuseConsoleClosedTest 直驱。`EchoMuseConsoleState`（要 Pager）**不在**
            // 这里——所以 D1 的修复点被搬到纯函数里测，状态机本身没被拆散。
            kotlin.include("com/xiaozhi/admin/EchoMuseConsoleProtocol.kt")
        }
        val commonTest by getting {
            dependencies { implementation(kotlin("test")) }
        }
    }
}
