import com.tencent.kuikly.gradle.config.KuiklyConfig
import org.jetbrains.kotlin.gradle.ExperimentalKotlinGradlePluginApi
import org.jetbrains.kotlin.gradle.dsl.JvmTarget
import javax.inject.Inject

plugins {
    kotlin("multiplatform")
    // kotlinx.serialization 编译器插件：为 @Serializable 类生成序列化器（版本随 Kotlin，见根脚本）。
    kotlin("plugin.serialization")
    // Compose 编译器插件：Kuikly Compose 的 @Composable 由它处理（见下方 compose 依赖）。
    id("org.jetbrains.kotlin.plugin.compose")
    id("com.android.library")
    // Kuikly KSP：扫描 @Page 注解生成页面注册代码（KuiklyCoreEntry，按编译目标注入）
    id("com.google.devtools.ksp")
    // Kuikly 官方 Gradle 插件：跨端产物打包（web 端 packLocalJSBundle* 产 nativevue2.zip）。
    // 经根 buildscript classpath 引入（版本与 kuiklyVersion 双写同步，见根脚本注释）。
    id("com.tencent.kuikly-open.kuikly")
}

val kuiklyVersion: String = property("kuiklyVersion") as String

// ===== kotlinx 基础库版本（按 Kotlin 2.1.21 兼容口径锁定，勿随手升级）=====
// - serialization：1.9.0+ 用更新 Kotlin 编译，元数据版本高于 2.1.21 会直接失败，停在 1.8.1
//   （最低要求 Kotlin 2.1.20，与 2.1.21 同代）。
// - datetime：停在 0.6.x —— 0.7.0 起 `Instant` 迁往 `kotlin.time`（实验 API），
//   升级必须连带改业务代码。
// 协程不走 kotlinx：用 Kuikly core 自带的 com.tencent.kuikly.core.coroutines（见下方依赖块注释）。
val serializationVersion = "1.8.1"
val datetimeVersion = "0.6.2"

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
                // Kuikly Compose 运行时（含 foundation / material3 / ui）：Settings 磁贴组件的基础。
                // 与 core 同一版本口径（kuiklyVersion），且有 macosarm64 / js 变体。
                implementation("com.tencent.kuikly-open:compose:$kuiklyVersion")
                // GearUI Kit 组件库（Apache-2.0，源码 vendored，见 gearuiKit/THIRD_PARTY_NOTICES.md）。
                // 用 implementation 而非 api：管理台以外无消费方，不需要向宿主泄漏。
                // ⚠️ 该模块目标集必须与本模块严格对齐（android/iosArm64/iosSimulatorArm64/
                //    macosArm64/js），缺任一目标本模块在该目标上即解析失败。
                implementation(project(":gearuiKit"))

                // ===== kotlinx 基础库（android / iosArm64 / macosArm64 / js 四端齐全）=====
                // JSON：替代手写解析器（FilesModel）与后续强类型回包解析。
                implementation("org.jetbrains.kotlinx:kotlinx-serialization-json:$serializationVersion")
                // 时间：收敛 formatEpochSeconds / localZoneOffsetSeconds 的四端 expect/actual 实现。
                implementation("org.jetbrains.kotlinx:kotlinx-datetime:$datetimeVersion")
                // ⚠️ 刻意**不**引入 kotlinx-coroutines-core：Kuikly core 自带一套协程
                // （com.tencent.kuikly.core.coroutines：Pager.lifecycleScope / launch / delay / Job），
                // 其 delay 到期经框架 timeout 注册投递回 context 线程。两套 Job 体系互不兼容，
                // 混用会让取消语义错乱（cancel 一套、等另一套），因此业务协程一律用 Kuikly 的那套。
                // 见 AdminRealtime 的重连/心跳。
            }
        }
        // T05 垂直切片：状态机制的单测。Compose 快照在**纯 JVM 单测**里可直接读写，
        // 不需要真机、不需要组合环境（Compose Runtime 的快照系统是纯 Kotlin 实现）。
        // 跑法：./gradlew :shared:testDebugUnitTest
        val commonTest by getting {
            dependencies {
                implementation(kotlin("test"))
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

// Admin icons are embedded vector pathData drawn by Canvas (AdminIcons.kt，方法对齐
// xiaoya-player TvIcons)：无资源文件、无端宿主 SVG 渲染器。护栏锁定该契约。
val verifyAdminIcons by tasks.registering {
    group = "verification"
    description = "Verify embedded vector icon coverage: token-to-geometry 1:1, Canvas rendering, no bitmap/resource fallback"
    val adminDir = layout.projectDirectory.dir("src/commonMain/kotlin/com/xiaozhi/admin")
    // T03 搬迁后：纯契约进 contracts/，图标几何与 1:1 映射进 icons/
    //（ui/ 目录本步不删，剩下的组件层要在 T12/T13 才整体替换）
    val contracts = adminDir.file("contracts/UiContracts.kt")
    val mappingFile = adminDir.file("icons/AdminIcons.kt")
    val geometryFile = adminDir.file("icons/IconVectors.kt")
    inputs.files(contracts, geometryFile, mappingFile)
    doLast {
        val enumBody = Regex("enum class IconToken\\s*\\{([^}]+)}")
            .find(contracts.asFile.readText())!!.groupValues[1]
        val tokenList = enumBody.split(',').map(String::trim).filter(String::isNotEmpty)
        val tokens = tokenList.toSet()
        val mappingText = mappingFile.asFile.readText()
        val pairs = Regex("IconToken\\.(\\w+) -> AdminIconVectors\\.(\\w+)").findAll(mappingText).toList()
        val mappings = pairs.associate { it.groupValues[1] to it.groupValues[2] }
        check(tokens.isNotEmpty() && tokenList.size == tokens.size && pairs.size == tokens.size &&
            mappings.values.toSet().size == tokens.size && tokens == mappings.keys
        ) { "IconToken geometry mapping must cover the nonempty canonical catalogue without duplicates" }
        check(mappingText.contains("Canvas(")) { "Icons must render through Kuikly Canvas with embedded vectors" }
        check(!Regex("\\bSvg\\s*\\{").containsMatchIn(mappingText)) { "Icons must not go through the removed Svg view" }
        listOf("h", "m").forEach { ext ->
            check(!rootProject.file("apps/macosApp/XzSvgView.$ext").exists()) { "Obsolete native SVG bridge source remains" }
            check(!rootProject.file("apps/macosApp/AdminAssetAdapter.$ext").exists()) { "Obsolete bitmap adapter source remains" }
        }
        logger.lifecycle("Verified ${tokens.size} embedded vector icons: Canvas rendering, no resource/bitmap fallback")
    }
}
tasks.matching { it.name == "check" || it.name.startsWith("packLocalJSBundle") }.configureEach {
    dependsOn(verifyAdminIcons)
}

// Admin UI state must go through Compose snapshot state (stateVar / stateListOf)，页面才能在
// Kit 组件（GearUI）与自有 Compose 组件之间共享同一套重组触发机制。Kuikly 自研的
// scope.observable()/observableList()/ObservableList 只被 Kuikly DSL 的 ViewContainer 消费，
// GearUI 组件内部读不到，改组件的状态会“写了不刷新”。护栏锁定三点：
//   1) 迁移后的 stateVar( 调用数精确命中（迁移完成度，防止漏改）；
//   2) stateListOf( 调用数精确命中；
//   3) 遗留原语计数为 0（import com.tencent.kuikly.core.reactive / scope.observable( /
//      observableList( / ObservableList 类型引用）。
// 只扫源码、只扫非注释行——StateVar.kt 的文档注释里会提到旧原语名，属正常说明。
// 白名单口径：T05 时唯一保留的旧机制触点是 LegacyVfor.kt（Kuikly DSL 的 vfor 只接受
// ObservableList（core 的 LoopDirectivesViewKt 只有一个 () -> ObservableList<T> 重载），
// 而状态层列表已是 SnapshotStateList，故桥接文件必须构造一次 ObservableList）。
//
// ✅ **T13 末检已执行（2026-10-29）**：`ui/`（14 文件）与两个过渡桥接文件
//   （`LegacyVfor.kt`、`LegacyAppIcon.kt`）已删除，`bridgeWhitelist` 收口为 `emptySet()`；
//   白名单撤掉后重跑护栏，`legacy` 计数仍为 0（旧原语真正归零，不是靠豁免蒙过去）。
//   `LegacyAppIcon.kt` 删前确认无 `stateVar(`/`stateListOf(` 调用，451/48 期望值不动。
//   此后出现任何旧原语即判失败（白名单机制随 T13 一并退役，不再有豁免通道）。
// 计数口径（改动原因逐条写在这里，避免下一个人以为护栏被"调松")：
//   T05 收口时 = 483。T06 换 Compose 壳时 `ToastHost.kt` 从"自持 ToastData 状态 + 自绘浮层"
//   改成"转发 GearUI 的 Toast"（浮层状态归 GearUI 的 OverlayRoot，不再由我们的源码持有），
//   该文件随之少掉 1 处 `var current by stateVar<ToastData?>(null)` → 483 - 1 = 482。
//   同一次改动里 `AdminPage.kt` 的 4 个页面级字段（selectedSection/drawerOpen/
//   serverSwitchOpen/apiBase）**逐一保留**，一个没少（已与 HEAD 版本对比确认）。
//   2026-10-29 范围削减（docs/plans/gearui-kit-scope-reduction.md §二）按 codemod
//   `--check` 实测值回填 482→455 / 51→48：删除 OpsSessionsPage / OpsConversationsPage /
//   OpsUsagePage / OpsMetricsPage 四页、OpsConversationsState / OpsUsageState /
//   OpsMetricsState 三个 state 类、OpsState.kt 内的 SessionRow / SessionsState，
//   以及 AdminPage 的 sessions / conversations / usage / metrics 四个字段与加载请求。
//   ⚠️ 这是"死代码必须删"的回填，不是把护栏调松——被删代码已不在任何渲染路径上。
//   T12 测试台重写回填 455→451：测试台 Audio 下沉删除 4 个播放 stateVar
//   （recAudioSrc/ttsAudioSrc/recPlayCommand/ttsPlayCommand）——DSL 原生 AudioView 挂不进
//   Compose 树，播放控制回归 XiaoZhiModule 的 playRecording/playTts/stopPlayback +
//   100ms 轮询 getAudioState，这 4 个字段及其命令值（0/1/3）不再有任何消费方。
//   T13 清理收尾回填 451→448：删除 ui/ 目录（14 个旧 DSL 组件文件）时，其中**自持状态的
//   组件**（ui/Tabs.kt 的 TabUiState——tab 选中/展开态，随目录一并退役）带走了 3 处
//   `stateVar(` 调用；stateListOf 48 不变（ui/ 无 stateListOf 调用），legacy 0 不变。
//   ⚠️ 逐文件分布未留档（ui/ 文件未入 git 索引即被删除），数值以本护栏实测回填。
val expectedStateVarCalls = 448
val expectedStateListCalls = 48
val bridgeWhitelist = emptySet<String>()
val verifyComposeStateMigration by tasks.registering {
    group = "verification"
    description = "Verify UI state uses Compose snapshot state (stateVar/stateListOf) and no legacy Kuikly reactive primitives"
    val sourceRoots = listOf("commonMain", "androidMain", "iosMain", "jsMain", "macosArm64Main")
        .map { layout.projectDirectory.dir("src/$it/kotlin") }
        .filter { it.asFile.isDirectory }
    inputs.files(sourceRoots.map { it.asFile }).withPathSensitivity(PathSensitivity.RELATIVE)
    doLast {
        // 状态定义文件自身含 fun stateVar( / fun stateListOf( 声明，不计入调用数；
        // 桥接文件允许出现旧机制（见上方注释），但**不允许**出现 stateVar/stateListOf 调用。
        val definitionFiles = setOf("state/StateVar.kt")
        val bridgeFiles = bridgeWhitelist
        var stateVarCalls = 0
        var stateListCalls = 0
        var legacyHits = 0
        val legacyDetail = mutableListOf<String>()
        val legacyPatterns = listOf(
            Regex("import\\s+com\\.tencent\\.kuikly\\.core\\.reactive\\."),
            Regex("\\bscope\\.observable\\s*\\("),
            Regex("\\bobservableList\\s*\\("),
            Regex("\\bObservableList\\s*<"),
        )
        sourceRoots.forEach { root ->
            root.asFile.walkTopDown().filter { it.isFile && it.extension == "kt" }.forEach { file ->
                val relative = file.relativeTo(root.asFile).invariantSeparatorsPath
                val isDefinition = definitionFiles.any { relative.endsWith(it) }
                val isBridge = bridgeFiles.any { relative.endsWith(it) }
                file.readLines().forEachIndexed { index, rawLine ->
                    // 注释行不参与统计：StateVar.kt 的等价说明里会出现旧原语名
                    val line = rawLine.trim()
                    if (line.startsWith("//") || line.startsWith("*") || line.startsWith("/*")) return@forEachIndexed
                    val code = rawLine.substringBefore("//")
                    if (!isDefinition && !isBridge) {
                        stateVarCalls += Regex("\\bstateVar\\s*(<[^<>]*>)?\\s*\\(").findAll(code).count()
                        stateListCalls += Regex("\\bstateListOf\\s*(<[^<>]*>)?\\s*\\(").findAll(code).count()
                    }
                    if (isBridge) return@forEachIndexed
                    legacyPatterns.forEach { pattern ->
                        if (pattern.containsMatchIn(code)) {
                            legacyHits += 1
                            legacyDetail += "$relative:${index + 1}: $line"
                        }
                    }
                }
            }
        }
        logger.lifecycle("Compose state migration: stateVar=$stateVarCalls (expect $expectedStateVarCalls), " +
            "stateListOf=$stateListCalls (expect $expectedStateListCalls), legacy=$legacyHits (expect 0)")
        // 末检提示仅在白名单非空时打印（T13 已收口为空集，此提示随之退役）
        if (bridgeFiles.isNotEmpty()) {
            logger.lifecycle("Legacy bridge whitelist = ${bridgeFiles.sorted()} " +
                "-> must delete ${bridgeFiles.sorted()} and reset " +
                "`bridgeWhitelist` to emptySet(), then re-run this task with legacy still 0")
        }
        check(legacyHits == 0) {
            "Legacy Kuikly reactive primitives remain ($legacyHits hit(s)); " +
                "migrate them to stateVar/stateListOf:\n" + legacyDetail.joinToString("\n")
        }
        check(stateVarCalls == expectedStateVarCalls) {
            "stateVar( call count is $stateVarCalls, expected $expectedStateVarCalls"
        }
        check(stateListCalls == expectedStateListCalls) {
            "stateListOf( call count is $stateListCalls, expected $expectedStateListCalls"
        }
    }
}
// T05 起挂到 check / web 打包：状态机制是"读得到才刷新"的前提，
// 一旦有人回填旧原语，后续 Compose 页面会静默不重组 —— 必须在打包前就拦住。
tasks.matching { it.name == "check" || it.name.startsWith("packLocalJSBundle") }.configureEach {
    dependsOn(verifyComposeStateMigration)
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
