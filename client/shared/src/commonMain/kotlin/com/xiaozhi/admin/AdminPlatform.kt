package com.xiaozhi.admin


import androidx.compose.runtime.Composable
import com.tencent.kuikly.core.module.Module
import com.tencent.kuikly.core.nvi.serialization.json.JSONObject
import com.tencent.kuikly.core.pager.Pager

/**
 * 管理台**平台差异点**集中声明（expect/actual）。
 *
 * ## 为什么收口到一个文件
 *
 * 废除 macOS 双壳（`AdminShell` @Page("router")）后，web 与 macOS 共用同一个
 * `@Page("admin")` 壳（[AdminPage]）。平台差异全部以 expect/actual 表达，
 * commonMain 的壳代码只面向这些 hook 编程，不出现任何 `if (平台)` 散落分支：
 *
 * - [IS_MACOS]：平台判定（导航过滤、侧边栏「服务器设置」入口）；
 * - [platformExternalModules]：macOS 注入 [XiaoZhiModule]（测试台的 WS/麦克风/Opus 原生桥）；
 * - [TestbenchSection]：测试台 section 的 Compose 挂载点（mac-only 真实现，
 *   非 mac 平台是防御空实现——菜单已在 web/iOS/Android 端隐藏）。T12 起
 *   取代旧 DSL 的 `renderTestbenchSection`（`ViewContainer` 扩展无法进 Compose 树）；
 * - [startAdminServerDiscovery] / [stopAdminServerDiscovery]：局域网 mDNS 服务器发现
 *   （macOS 转发 [XiaoZhiModule]；其他平台 no-op）。T12 起取代旧 DSL 的
 *   `macServerSwitchEntry`——发现逻辑本身是平台无关的（见 `MacServerSwitch.kt`），
 *   只有「谁来做 mDNS」是平台差异，故收窄成这两个 Pager 级 hook；
 * - [pagerBaseUrl]：API 基址（web 相对路径 = ""，macOS 读 SP 记忆地址）。
 *
 * actual 落点：macosArm64Main（真实现）与 jsMain / androidMain / iosMain（空实现/默认值）。
 */
expect val IS_MACOS: Boolean

/**
 * Pager 额外注册的平台模块：macOS 返回 [XiaoZhiModule]（测试台依赖），
 * 其他平台返回空 map（测试代码只存在于 macosArm64Main）。
 */
expect fun Pager.platformExternalModules(): Map<String, Module>

/**
 * 测试台 section（mac-only）的 Compose 挂载点。
 *
 * macOS actual：按 Pager 缓存 [TestBenchState]（切走再切回来连接状态/录音不丢）
 * 并渲染五 tab 工作区（`TestBenchPage.kt`）；其他平台 actual 为空实现
 * （防御分支：导航表 [AdminNav.visible] 已按平台隐藏该项，正常不会走到）。
 */
@Composable
expect fun TestbenchSection(page: AdminPage)

/**
 * 启动局域网服务器发现（macOS = [XiaoZhiModule] 的 mDNS 浏览 `_xiaozhi-admin._tcp`；
 * 结果经 [onResult] 回调 {servers:[{name, base}]}，已在 Kuikly 线程）。
 * 其他平台 no-op：侧边栏入口只在 [IS_MACOS] 时渲染。
 */
expect fun Pager.startAdminServerDiscovery(onResult: (JSONObject?) -> Unit)

/** 停止局域网服务器发现（幂等；macOS 之外 no-op）。 */
expect fun Pager.stopAdminServerDiscovery()

/**
 * 管理 API 基址：web 返回 ""（server 同域托管，走相对路径）；
 * macOS 返回用户记忆的服务器地址（SP key "xz.server.baseUrl"，空则回退本地默认）。
 */
expect fun pagerBaseUrl(pager: Pager): String

/**
 * 复制文本到系统剪贴板，返回是否成功。
 * web 管理台常以 `http://LAN-IP` 访问（非 secure context，无 async Clipboard API），
 * actual 内回落 `execCommand('copy')`；不支持剪贴板的平台返回 false（调用方 toast 提示）。
 */
expect fun copyToClipboard(text: String): Boolean
