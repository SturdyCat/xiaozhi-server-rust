# 第三方代码来源与许可（sharedCompose）

本模块全部源码为**上游 vendored**（未发布制品可用），改动仅限「补齐构建目标」与「编译适配」，
不含视觉/行为改造。上游升级时按下面路径重新拉取比对。

## 1. Compose-Settings（Kuikly 适配版）

- 上游仓库：https://github.com/Kuikly-contrib/Compose-Settings（分支 `main-kuikly`，目录 `settings-kuikly/`）
- 原始项目：https://github.com/alorma/Compose-Settings（MIT，Copyright (c) 2021 Bernat Borrás Paronella）
- 采用的版本：`3.2.0-kuikly`（对应上游 3.2.0）
- 落位包：
  - `com.alorma.compose.settings.ui.*`（ui-tiles：SettingsSwitch / SettingsCheckbox /
    SettingsRadioButton / SettingsTriStateCheckbox / SettingsMenuLink / SettingsGroup）
  - `com.alorma.compose.settings.ui.*`（ui-tiles-extended：SettingsSlider / SettingsSegmented）
  - `com.alorma.compose.settings.ui.expressive.*`（ui-tiles-expressive）
  - `com.alorma.compose.settings.ui.core.*`（CompositionLocals / 默认值）
  - `com.tencent.kuikly.compose.material3.*`（List / Segmented / Toggle / RadioButton 补充实现）
- 许可：MIT（见上游 LICENSE）

### 为什么 vendored 而不是依赖坐标

1. 上游 Kuikly 版**未发布到任何公共仓库**：`com.github.alorma.compose-settings:ui-tiles:3.2.0-kuikly`
   在 Maven Central 与 Tencent 镜像均 404（Maven Central 上只有原版 JetBrains Compose 的 3.x）。
2. 上游 `settings-kuikly/*/build.gradle.kts` 只声明 `android / iosX64 / iosArm64 / iosSimulatorArm64 / js`，
   **没有 `macosArm64`**；本项目 macOS（Mac Catalyst）是主力端，必须补该目标。
3. 上游硬编码 `kuiklyVersion = "2.26.0-2.1.21"`，本项目为 `2.29.2-2.1.21`，需要对齐 core 版本。

### 为什么带着 material3 补充实现

上游 Kuikly Compose `2.29.2-2.1.21` 的 material3 制品实测**不含**
`ListItem` / `SegmentedButton` / `SegmentedListItem` / `ToggleButton` / `RadioButton`
（对 `compose.jar` 的 linkdata 符号逐个核对：这几个符号计数为 0，`Switch` / `Slider` 为 1）。
因此 `com/tencent/kuikly/compose/material3/*.kt` 是**补真实缺位**，不是重复定义。

## 2. kuiklyx-viewmodel

- 上游仓库：https://github.com/Kuikly-contrib/kuiklyx-viewmodel
- 版本：`1.0.6`（上游 `gradle.properties` 的 `kuiklyCoreVersion=2.11.0`）
- 落位包：`com.tencent.kuiklyx.viewmodel.*`（3 个文件，171 行）
- 许可：Copyright (c) 2025 Tencent Music Entertainment Group（文件头保留原始版权声明）

### vendored 原因

上游 `viewmodel/build.gradle.kts` 只声明 `androidTarget + iosX64/iosArm64/iosSimulatorArm64`，
**缺 `js` 与 `macosArm64`**——这两端恰好是本项目的主力端（web + macOS）。
源码为纯 commonMain，只依赖 Kuikly core 的 `Pager` / `PagerScope` / `IPagerEventObserver` /
`Event` / `KLog` / `JSONObject`，没有平台特定实现，因此补目标即可编译运行。

## 升级流程

1. 重新 clone 上游对应分支，比对 `settings-kuikly/{ui-core,ui-tiles,ui-tiles-extended,ui-tiles-expressive}/src`
   与本模块 `src/commonMain/kotlin` 的差异；
2. 同步时**保留**本模块 `build.gradle.kts` 的 targets（含 `macosArm64` / `js`）；
3. 若上游 Kuikly Compose 版本升级后 material3 已自带上述组件，删除
   `com/tencent/kuikly/compose/material3/*.kt` 以避免符号重复。
