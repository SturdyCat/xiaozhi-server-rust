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
    // Kotlin 2.1.21 与 gradle.properties 中的 kuiklyVersion=2.29.2-2.1.21 绑定版本一致。
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
        // com.yarnpkg:yarn 备援源（2026-11-08 修复）：Kotlin/JS 的 kotlinYarnSetup 在任务图阶段
        // 解析 detached configuration，从 KGP 内置的 GitHub release 资产端点拉 yarn tarball；
        // 该端点当时对 HEAD/GET 一律 504（GHA 与本地冷缓存同时复现，逐字一致），构建必然失败，
        // 且失败的 RUN 不导出缓存挂载 → 下次依旧冷构建依旧死在同一处。
        // npm registry 上的 yarn@1.22.17 与 GitHub release tarball 逐字节同源
        // （Gradle 构件缓存目录名 = 文件 sha1 = bf910747…，两源完全一致），顶层布局同为
        // yarn-v1.22.17/，KGP 解压即用。两个 ivy 源均 content 过滤【只】服务该模块：
        // 其他依赖解析零额外网络面；KGP 的 GitHub 源仍在链尾兜底（npm 双源全挂时才触达）。
        // ⚠️ 与上文「不追加额外源」的约定不冲突：那条防的是无过滤源扩大 DNS 故障面，
        // 此处为单模块备援，不改变其他依赖的解析链。Kotlin 升级若提高默认 yarn 版本，
        // pattern 的 [revision] 自动跟随，无需改这里。
        ivy("https://registry.npmmirror.com") {
            patternLayout { artifact("yarn/-/yarn-[revision].tgz") }
            metadataSources { artifact() }
            content { includeModule("com.yarnpkg", "yarn") }
        }
        ivy("https://registry.yarnpkg.com") {
            patternLayout { artifact("yarn/-/yarn-[revision].tgz") }
            metadataSources { artifact() }
            content { includeModule("com.yarnpkg", "yarn") }
        }
    }
}

rootProject.name = "client"

// 跨端业务逻辑（Kotlin Multiplatform）：android / ios / js(web) 共用
include(":shared")

// GearUI Kit 组件库（Apache-2.0，源码 vendored）：上游只发布 android/ios/js，缺 macosArm64，
// 本项目 macOS(Mac Catalyst) 是主力端，故源码引入并自补 macosArm64 目标。
// 来源、许可与相对上游的构建适配见 gearuiKit/THIRD_PARTY_NOTICES.md。
include(":gearuiKit")

// Pure BLE protocol tests reuse the production codec source without linking a
// Kuikly renderer, native host exports, or introducing tests into shipped bundles.
include(":bleProtocolTests")

// Production UI contracts and navigation models, without a Kuikly renderer or KSP.
include(":adminCoreTests")

// 各平台宿主 App 统一汇聚在 apps/ 下
// —— 以下两个是 Gradle 模块（带 build.gradle.kts）
include(":apps:androidApp")   // Android 宿主（com.android.application）
include(":apps:h5App")        // Web(H5) 宿主（kotlin js）
// —— 以下两个是原生宿主目录（非 Gradle 模块，由 Xcode / DevEco 打开）：
//   apps/iosApp   : iOS 宿主（Podfile 链接 shared.framework + OpenKuiklyIOSRender；仅 arm64）
//   apps/macosApp : macOS 宿主（Mac Catalyst，复用 iOS 渲染器——本质即 iPad/iOS App 的桌面兼容模式；提供 ASR/TTS 测试页，仅 Mac 可用）
//   apps/ohosApp  : HarmonyOS 宿主（Entry 模块链接 @kuikly 鸿蒙渲染）
// 如需把它们也纳入 Gradle 构建，可在此补充对应 include 子模块。
