# macOS / Mac Catalyst 指引

继承[客户端指引（从上级入口导航）](<../../AGENTS.md>)。宿主使用 UIKit 渲染器，只选 **My Mac (Mac Catalyst)**，不是原生 My Mac。

- 改 shared 页面后先运行 `:shared:linkReleaseFrameworkMacosArm64`；只跑 Xcode 会链接旧业务框架。Web 构建不覆盖该路径。
- 工程生成顺序：xcodegen generate → pod install → 打开 workspace。重生 xcodeproj 会抹掉 Pods 集成，必须重跑 pod install。
- [project](<project.yml>)的 Catalyst 防误选与 ad-hoc 手动签名保留；不要回退无团队的 Automatic 或未经验证的 Catalyst-only 签名配方。
- 应用图标来自 [AppIcon](<Assets.xcassets/AppIcon.appiconset/Contents.json>)：保留资源输入及 `ASSETCATALOG_COMPILER_APPICON_NAME=AppIcon`；[图标声明](<Info.plist>)同步维护在 [project](<project.yml>) 的 `info.properties`，避免工程再生成时丢失。构建后确认 App 包含 `Contents/Resources/AppIcon.icns`，且成品 plist 的 `CFBundleIconFile=AppIcon`。仅修改源配置不会更新旧 App；先重建并确认启动的是新产物，仍显示缓存图标时再从 Dock 移除旧入口并重新添加，不要优先清理全局缓存。
- [Podfile](<Podfile>)以 iOS 平台配置，给 Pod 目标启用 Catalyst、排除 x86_64 并设置 iOS deployment target；只改 MACOSX_DEPLOYMENT_TARGET 不能修 UIKit Pod 的映射目标。
- shared 框架路径从 apps/macosApp 上两级，用 PROJECT_DIR 绝对化搜索路径；releaseFramework 不是 releaseShared。
- Kotlin/Native macabi 平台补丁已由[shared 构建任务](<../../shared/build.gradle.kts>)自动 finalizedBy；使用[补丁脚本](<../../scripts/patch_framework_macabi.py>)，不要手动 vtool 重写一套。
- WS/采音/播放/Opus 桥在[XiaoZhiModule](<XiaoZhiModule.m>)及其 Audio/Opus 分类；协议改动同时检查原生桥与服务端。
- 宿主启动即打开 shared 的统一管理台 `@Page("admin")`（[SceneDelegate](<SceneDelegate.m>)，pageName 已从旧 `router` 切换）；macOS 与 web 同壳，平台差异（测试台 / 服务器设置 / 服务器基址）由 shared 的 AdminPlatform expect/actual 承载，原 AdminShell 双壳已删除。
- **服务器设置与自动连接**：侧边栏「服务器设置」内联表单（[AdminPlatform mac actual](<../../shared/src/macosArm64Main/kotlin/com/xiaozhi/admin/AdminPlatform.kt>)）展示局域网自动发现列表 + 手填地址；首次运行（从未保存地址）发现到服务器即自动接管 = 开箱自动连接。发现经 [XiaoZhiModule+Discovery.m](<XiaoZhiModule+Discovery.m>) 浏览服务端常驻广播的 `_xiaozhi-admin._tcp`（Info.plist/project.yml 的 `NSBonjourServices` 必须声明该类型，缺失时系统静默拦截浏览）。
- **⚠️ 线程铁律**：Kuikly Kotlin 侧跑在独立 context 线程，observable/setTimeout 等只允许在该线程调用。一切异步事件（WS 帧、发现结果）必须经 XiaoZhiModule 的 `invoke:` 回调上行——框架把回调 block 包裹为 `performOnContextQueue`，任意原生线程调用都会被切回 context 线程。禁止在 Kotlin 侧自建 NSURLSession/GCD 直调共享层（2026-10 管理台 WS 断线重连触发 `assertContextQueue` 卡死事故即此根因，AdminWs 就是那次整改的产物）。

本级说明 · [协议联调（从上级入口导航）](<../../AGENTS.md>)


---

## macOS 语音测试台使用

> 本文为当前功能使用说明；实现细节以邻近源码、配置样例与测试为准。

### 7. ASR/TTS 测试台（macOS App）

`web` 受浏览器麦克风 / WebSocket 二进制等限制，不适合语音链路测试。macOS App（`apps/macosApp`，Mac Catalyst，复用 iOS 渲染器 `OpenKuiklyIOSRender`）承载端到端语音测试：

- **测试代码只在 `client/shared/src/macosArm64Main`**：web/Android/iOS/OHOS 的包不含测试代码；web 管理台不显示「测试台」菜单（`NavDescriptor.macOnly`），仅 macOS 客户端可见。
- **五个 tab**：连接 / ASR 识别（录音试听 → 识别解耦 + 波形回放）/ TTS 合成（音色下拉 + 语速，可临时覆盖当前配置）/ LLM 对话 / NLU 意图（意图 + 置信度 + tokenize/infer 延迟拆解）；每个 tab 右侧内嵌对应配置卡，配合顶栏「保存配置」写回。
- 原生桥接 `apps/macosApp/XiaoZhiModule*.m`：WS 建连（测试台 [XiaoZhiModule.m](<XiaoZhiModule.m>)、管理台实时通道 [XiaoZhiModule+AdminWs.m](<XiaoZhiModule+AdminWs.m>)、服务器发现 [XiaoZhiModule+Discovery.m](<XiaoZhiModule+Discovery.m>)）、麦克风采集、音频播放、**Opus 编解码**（`XiaoZhiModule+Opus.m`，libopus）均已实现。

```bash
## ① 编出 KMP 业务框架（仅 Apple Silicon）
cd client && ./gradlew :shared:linkReleaseFrameworkMacosArm64
##    产物：client/shared/build/bin/macosArm64/releaseFramework/shared.framework
##    （⚠️ 改动任何 @Page 或 shared 业务代码后必须重跑本任务，否则 Xcode 跑旧框架）

## ② 生成 Xcode 工程（需 brew install xcodegen）
cd client/apps/macosApp && xcodegen generate

## ③ 安装渲染器依赖（需 CocoaPods）
pod install                                        # 生成 macosApp.xcworkspace

## ④ Xcode 打开 workspace，运行目标选「My Mac (Mac Catalyst)」
open macosApp.xcworkspace
```

> 启动顺序不可省（① 在 ② 之前，否则链接报 `shared.framework not found`）。Xcode 重跑后若白屏（页面回退占位），先 Clean Build Folder——那是 Kuikly 找不到 page 的 fallback，不是代码 bug。

---

## 上级入口

[上级 AGENTS](<../../AGENTS.md>)
