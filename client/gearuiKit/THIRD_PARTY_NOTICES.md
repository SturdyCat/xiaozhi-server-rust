# 第三方代码来源与许可（gearuiKit）

本模块全部源码为**上游 vendored**（上游未发布 `macosArm64` 制品，而本项目 macOS/Mac Catalyst 是主力端），
改动仅限「补齐构建目标」与「编译适配」，**不含视觉/行为改造，`commonMain` 源码零改动**。
上游升级时按下面的「升级流程」重新拉取比对。

## 1. GearUI Kit

- 上游仓库：https://github.com/gearui/gearui-kit
- 拉取方式：`git clone --depth 1 https://github.com/gearui/gearui-kit`
- 拉取时的 commit：`e5e32e930b06792b3f4c633ea82624b2ef6dbdad`（2026-10-29）
- 采用的版本：`1.0.0`（上游 `gradle.properties` 的 `POM_VERSION`）
- 许可：**Apache License 2.0**
  - 版权：`Copyright 2026 Shanghai Boyu Information Technology Co., Ltd.`（见本目录 `NOTICE`）
  - 许可原文：本目录 `LICENSE`（整份拷入，未改一行）
  - 项目主页：https://gearui.com ／ https://netonstream.com
- 落位包（保持 `com.gearui.*` 不变，不做包名改写）：`com.gearui.components.*` / `com.gearui.foundation.*` /
  `com.gearui.theme.*` / `com.gearui.i18n.*` / `com.gearui.system.*` 等

### vendored 范围与规模

| 上游目录 | 处置 | 规模 |
|---|---|---|
| `gearui-kit/src/commonMain` | 逐字节拷入 | 256 文件 / 43,512 行 |
| `gearui-kit/src/androidMain` | 逐字节拷入 | 7 文件 / 66 行 |
| `gearui-kit/src/iosMain` | 逐字节拷入 | 7 文件 / 124 行 |
| `gearui-kit/src/jsMain` | 逐字节拷入 | 7 文件 / 49 行 |
| `gearui-kit/src/macosArm64Main` | **新增（上游无此目录）** | 7 文件 / 见下节 |
| `gearui-kit/src/ohosArm64Main` | **不拷**（本项目无 ohos 目标） | 7 文件 / 76 行 |
| `gearui-kit/src/commonTest` | **不拷**（上游自测不是本项目质量门） | 64 文件 / 4,010 行 |
| `gearui-kit/src/androidUnitTest` | **不拷** | 1 文件 / 36 行 |
| `api/`、`sample/`、`scripts/`、`docs/`、`build.ohos.gradle.kts` | **不拷**（与运行无关） | — |

> 拷入时**保留全部原始内容**：上游源码无 per-file 版权头（版权在仓库级 `LICENSE`/`NOTICE`），
> 故我们**不注入**自己的注释头——否则破坏与上游的 `diff -r` 可比性，反而弱化合规可追溯性。

### 相对上游的构建适配（7 项，均为 `build.gradle.kts` 侧，源码零改动）

| # | 适配项 | 依据 |
|---|---|---|
| ① | 去 `id("org.jetbrains.compose")` 插件与 `implementation(compose.runtime)` | `grep -rn "org.jetbrains.compose" gearui-kit/src` → 0 命中；`com.tencent.kuikly-open:compose` 的 macosArm64 变体已 `api` 依赖 `org.jetbrains.compose.runtime:runtime:1.7.3`（见其 `.module`） |
| ② | 去 `androidx.annotation:annotation:1.9.1` 依赖 | `grep -rn "androidx.annotation" gearui-kit/src` → 0 命中 |
| ③ | 去 `com.vanniktech.maven.publish` 插件与整套 POM 配置 | 本地模块不发布 |
| ④ | 去 `ohosArm64Main` / `commonTest` / `androidUnitTest` 源集 | 本项目无对应目标与测试口径 |
| ⑤ | 加 `macosArm64()`；去 `iosX64()` | 本项目 `:shared` 只有 `iosArm64`/`iosSimulatorArm64`/`macosArm64`；Intel 模拟器已在 Podfile 里 `EXCLUDED_ARCHS=x86_64` 排除 |
| ⑥ | `compileSdk` 36→35、`jvmTarget` 1.8→17 | 与 `:shared` 对齐 |
| ⑦ | 用默认层级模板（不手动 `val iosMain by creating`） | 本项目未设 `kotlin.mpp.applyDefaultHierarchyTemplate=false`，默认模板会自动建 `iosMain`/`macosMain`/`appleMain`/`nativeMain`；重复创建直接构建失败 |

### 新增的 `macosArm64Main`（7 文件 = 8 个 `actual`）

上游 `commonMain` 共 8 个 `expect`，macosArm64 目标需逐一补齐：

| 文件 | 对应 `expect` | 来源 |
|---|---|---|
| `foundation/typography/LineHeight.kt` | `rendererLineHeight` | **照抄 iOS 版**（零平台符号，只用 `TextUnit`） |
| `foundation/typography/NativeGenericFont.kt` | `nativeGenericFont` | **照抄 iOS 版**（纯常量映射） |
| `components/input/NumericKeyboard.macos.kt` | `numericKeyboardType` | **照抄 iOS 版**（只用 `KeyboardType`） |
| `components/calendar/CalendarPlatform.macos.kt` | `currentCalendarDate` | **照抄 iOS 版**（只用 `platform.Foundation`） |
| `i18n/Clock.macos.kt` | `currentEpochMillis` + `localMomentOf` | **照抄 iOS 版**（只用 `platform.Foundation`） |
| `foundation/keyboard/PlatformKeyboard.macos.kt` | `observePlatformKeyboard` | **新写空实现**（见下） |
| `system/InterfaceStyle.macos.kt` | `applyPlatformInterfaceStyle` | **新写空实现**（见下） |

两个空实现的理由：

1. `observePlatformKeyboard`：macosArm64 的 Kotlin/Native **平台库不含 UIKit**（UIKit 只在 ios/tvos/watchos
   平台库中），macOS 也没有 `UIKeyboardWillChangeFrameNotification`。Mac Catalyst 运行时虽带 UIKit，
   但本产物链接的是 macOS 平台库，强行引 UIKit 会在**编译期**找不到符号。macOS 无软键盘 → 不回调。
2. `applyPlatformInterfaceStyle`：该函数唯一作用是让**系统绘制的部件**（iOS 软键盘、系统弹窗）跟随 App 外观，
   macOS 无此概念；且本项目主题固定 Light（`themeMode` 恒为 `Light`），实现也不会有可见差异。
   不选 AppKit 路线（`NSApp.appearance`）：本产物是 Catalyst（UIKit 进程），AppKit 符号能否安全调用未经证实，收益为零。

> 上述 2 个文件在文件头写明「新增：macOS actual，非上游代码」（Apache-2.0 §4(b) 改动标注）。

## 升级流程

1. 重新 `git clone --depth 1 https://github.com/gearui/gearui-kit` 到临时目录；
2. `diff -r <tmp>/gearui-kit/src/commonMain client/gearuiKit/src/commonMain` —— 除下述例外应**完全为空**；
   `androidMain` / `iosMain` / `jsMain` 同理；
3. 同步时**保留**本模块 `build.gradle.kts` 的目标集（含 `macosArm64`）与 `src/macosArm64Main/` 全部新文件
   （`diff -r` 会报「只在本地存在」，属预期差异）；
4. 若上游某个 `expect` 新增/改签名，须同步到 `src/macosArm64Main/`（5 个照抄 iOS 版、2 个空实现）；
5. 重新跑 `./gradlew :gearuiKit:compileKotlinMacosArm64 :gearuiKit:compileKotlinJs :gearuiKit:compileDebugKotlinAndroid`。
