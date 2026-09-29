// 根构建脚本：声明各子模块共享的插件版本（apply false），并引入 Kuikly 官方 Gradle 插件。
// Kuikly 插件通过 buildscript classpath 引入（与官方独立业务工程一致），避免依赖 plugin marker。
// 构建环境对齐 xiaoya-player：同版本 Kuikly 2.28.0-2.1.21 + Kotlin 2.1.21 + Gradle 8.14.5。
buildscript {
    // Kuikly 插件版本：与 gradle.properties 的 `kuiklyVersion` 保持同值
    // （buildscript 块在 project 属性求值之前执行，无法读该 property，只能同字面量双写）。
    // ⛔ 改版本时两处必须同步：此处 / gradle.properties 的 kuiklyVersion。
    //    （shared / apps:h5App 都从该 property 读版本，core 与 web 渲染器错配属未定义组合。）
    repositories {
        google()
        mavenCentral()
        maven { setUrl("https://mirrors.tencent.com/repository/maven-tencent/") }
    }
    dependencies {
        classpath("com.tencent.kuikly-open:core-gradle-plugin:2.28.0-2.1.21")
    }
}

plugins {
    // 统一锁定版本：各子模块使用简写 id（如 kotlin("multiplatform")）时在此解析。
    // apply false：仅集中锁定版本，由各子模块自行 apply，避免 Kotlin 插件被重复加载报错。
    kotlin("multiplatform") version "2.1.21" apply false
    kotlin("android") version "2.1.21" apply false
    id("com.android.application") version "8.9.0" apply false
    id("com.android.library") version "8.9.0" apply false
    // Kuikly KSP 宿主插件：扫描 @Page 注解生成页面注册代码（版本须与 Kotlin 2.1.21 配套）
    id("com.google.devtools.ksp") version "2.1.21-2.0.1" apply false
}

// ⚠️ 不要在这里（或任何子模块）声明项目级 repositories：
//   ① settings.gradle.kts 已统一管理仓库，重复声明只会漂移；
//   ② 根工程一旦带上项目级仓库，Kotlin/JS 的 Node 发行版（org.nodejs:node，nodejs.org dist 仓库）
//      会被挤到链条末尾——中间某个仓库抛 DNS 异常时整个解析直接中止（实测 maven.tencent.com
//      解析失败导致 org.nodejs:node 必然失败）。xiaoya-player 的根工程同样没有 allprojects 仓库块。
