// 根工程：仅做统一仓库 / 通用配置。
// 各模块的具体依赖与产物在其自身 build.gradle.kts 中声明。
// Kuikly 版本集中管理在 gradle.properties 的 kuiklyVersion。

allprojects {
    repositories {
        google()
        mavenCentral()
        maven { setUrl("https://mirrors.tencent.com/repository/maven-tencent/") }
        maven { setUrl("https://maven.tencent.com/repository/maven/") }
    }
}
