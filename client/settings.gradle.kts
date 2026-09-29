pluginManagement {
    repositories {
        google()
        mavenCentral()
        gradlePluginPortal()
        // Kuikly 官方制品（core / core-render-web:h5 / core-gradle-plugin / core-ksp）都在此镜像。
        // ⚠️ 不要追加 maven.tencent.com 等额外源：多一个仓库就多一分解析链被 DNS/网络异常
        // 中断的概率（org.nodejs:node 就这么挂过，见根 build.gradle.kts 的注释）。
        maven { setUrl("https://mirrors.tencent.com/repository/maven-tencent/") }
    }

    // 统一插件版本：各子模块使用简写 id（如 kotlin("multiplatform")）时必须在此声明版本才能解析。
    // 注意：插件版本声明必须放在 pluginManagement { plugins { } } 内（不能放在 dependencyResolutionManagement 里）。
    // 加 apply false：仅集中锁定版本，由各子模块自行 apply，避免 Kotlin 插件在多个子项目被重复加载报错。
    // Kotlin 2.1.21 与 gradle.properties 中的 kuiklyVersion=2.28.0-2.1.21 绑定版本一致。
    plugins {
        kotlin("multiplatform") version "2.1.21" apply false
        kotlin("android") version "2.1.21" apply false
        id("com.android.application") version "8.9.0" apply false
        id("com.android.library") version "8.9.0" apply false
        // Kuikly KSP 宿主插件（shared 扫描 @Page 生成页面注册代码），与 Kotlin 2.1.21 配套
        id("com.google.devtools.ksp") version "2.1.21-2.0.1" apply false
    }
}

dependencyResolutionManagement {
    // 不设 repositoriesMode（默认 PREFER_PROJECT，与 xiaoya-player 一致）。
    // ⚠️ 不要改成 PREFER_SETTINGS：那会把 Kotlin/JS 插件自动加到根工程的 nodejs.org dist
    // 仓库丢弃掉，org.nodejs:node（Node 发行版）将落到 maven 仓库里找 → 必然 Could not resolve。
    repositories {
        google()
        mavenCentral()
        maven { setUrl("https://mirrors.tencent.com/repository/maven-tencent/") }
    }
}

rootProject.name = "client"

// 跨端业务逻辑（Kotlin Multiplatform）：android / ios / js(web) 共用
include(":shared")

// 各平台宿主 App 统一汇聚在 apps/ 下
// —— 以下两个是 Gradle 模块（带 build.gradle.kts）
include(":apps:androidApp")   // Android 宿主（com.android.application）
include(":apps:h5App")        // Web(H5) 宿主（kotlin js）
// —— 以下两个是原生宿主目录（非 Gradle 模块，由 Xcode / DevEco 打开）：
//   apps/iosApp   : iOS 宿主（Podfile 链接 shared.framework + OpenKuiklyIOSRender；仅 arm64）
//   apps/macosApp : macOS 宿主（Mac Catalyst，复用 iOS 渲染器——本质即 iPad/iOS App 的桌面兼容模式；提供 ASR/TTS 测试页，仅 Mac 可用）
//   apps/ohosApp  : HarmonyOS 宿主（Entry 模块链接 @kuikly 鸿蒙渲染）
// 如需把它们也纳入 Gradle 构建，可在此补充对应 include 子模块。
