package com.xiaozhi.admin

/**
 * 等宽字体族名（**唯一用途**：EchoMuse 控制台的输出区，设计 §6.4「等宽字体」行）。
 *
 * 为什么是 `expect val` 而不是打包字体：
 * - 官方 `Text` 的 `fontFamily(String)` 收的是**真实字体族名**。Android/H5 的 `monospace`
 *   是系统通用族可直接用；iOS/macOS（Catalyst）必须写真实族名，系统自带的是 `Menlo`。
 * - 引入 `.ttf` 会把几百 KB 二进制塞进 shared 资源，还要按端配置加载与注册，换来的只有
 *   「字宽一致」这一项；而本项目的 UI **刻意不依赖**字符宽度对齐——日志的「时间/级别/来源」
 *   列用固定像素宽的 `View` 拼，不用空格（无 `ch` 单位、无字符宽度测量 API，见 §6.4）。
 * - 族名未命中时平台会自动回退到默认字体：此时控制台只是「看起来不像终端」，不会出现
 *   排版错乱或内容丢失，所以这个 `expect` 失败是**降级而非故障**。
 *
 * ⚠️ 新增本 `expect` 时 4 个 `actual` 必须同时存在（`jsMain`/`androidMain`/`iosMain`/
 * `macosArm64Main`）：`sourceSets` 只有这 4 个平台源集，少一个那一端就编译失败。
 */
expect val ADMIN_MONO_FONT: String
