# Kuikly 客户端指引

继承[根指引](<../AGENTS.md>)。本文件只放平台/构建边界；页面和表单维护见[公共管理台维护](<shared/src/commonMain/kotlin/com/xiaozhi/admin/AGENTS.md>)。

- UI 修改前查仓库 Kuikly 技能与官方组件；优先复用既有组件层，不手搓官方已有的 Switch、AlertDialog、Tabs/PageList 或 Button。
- commonMain 管理台跨端复用，web 与 macOS 共用单一 `@Page("admin")` 壳（原 macOS 双壳 AdminShell 已废除，平台差异走 AdminPlatform expect/actual）；麦克风/Opus 语音测试只在 macosArm64Main 与 macOS 原生桥，不把测试依赖带进 Web/Android/iOS/OHOS。
- Web 双 bundle：shared 持有 JS 目标产业务包 nativevue2.js，h5App 壳产 h5App.js，入口先业务后壳。不要移走 shared 的 JS 目标。
- 渲染器坐标/版本以[Gradle 配置](<apps/h5App/build.gradle.kts>)与[版本配置](<gradle.properties>)为准；升级同步根 buildscript，保留[JS 依赖锁](<kotlin-js-store/yarn.lock>)。
- 不设 PREFER_SETTINGS，不加入多余项目级 repositories，避免丢 Kotlin/JS 自动注入的 Node 下载源。

在 client 目录验证：

```bash
./gradlew :bleProtocolTests:jvmTest
./gradlew :apps:h5App:publishWeb -PwebSourceMap=false
./gradlew :shared:compileKotlinMacosArm64
```

`bleProtocolTests` 是无页面、无 Kuikly/KSP 的普通 KMP 测试模块，只引用生产 `EchoMuseBleCodec.kt` 源文件并运行纯协议/校验回归，不复制实现，也不参与生产 bundle。共享模块的 native 测试依赖宿主导出 `com_tencent_kuikly_IsCurrentOnContextThread`，不能将只编译成功称为测试通过，或为测试伪造渲染宿主符号。

publishWeb 不覆盖 macOS 源集；原生运行另重建框架。[macOS 构建指引](<apps/macosApp/AGENTS.md>) · [部署内存限制](<../docs/deployment.md>)

## 上级入口

[上级 AGENTS](<../AGENTS.md>)

## 下级入口

- [macOS 宿主](<apps/macosApp/AGENTS.md>)：Mac Catalyst 原生桥与语音测试台。
- [公共管理台](<shared/src/commonMain/kotlin/com/xiaozhi/admin/AGENTS.md>)：跨端页面、表单与配置契约。
