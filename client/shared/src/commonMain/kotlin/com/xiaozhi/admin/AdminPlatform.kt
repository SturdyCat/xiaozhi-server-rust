package com.xiaozhi.admin

import com.tencent.kuikly.core.base.ViewContainer
import com.tencent.kuikly.core.module.Module
import com.tencent.kuikly.core.pager.Pager

/**
 * 管理台**平台差异点**集中声明（expect/actual）。
 *
 * ## 为什么收口到一个文件
 *
 * 废除 macOS 双壳（`AdminShell` @Page("router")）后，web 与 macOS 共用同一个
 * `@Page("admin")` 壳（[AdminPage]）。两者只剩四类差异，全部以 expect/actual 表达，
 * commonMain 的壳代码只面向这些 hook 编程，不出现任何 `if (平台)` 散落分支：
 *
 * - [IS_MACOS]：平台判定（导航过滤、测试台分支）；
 * - [platformExternalModules]：macOS 注入 [XiaoZhiModule]（测试台的 WS/麦克风/Opus 原生桥）；
 * - [renderTestbenchSection]：测试台渲染（mac-only，web 菜单已隐藏，此处是防御分支）；
 * - [macServerSwitchEntry]：macOS 的「切换服务器」入口与表单（web 由 server 同域托管，无此概念）；
 * - [pagerBaseUrl]：API 基址（web 相对路径 = ""，macOS 读 SP 记忆地址）。
 *
 * actual 落点：macosArm64Main（真实现）与 jsMain / androidMain / iosMain（空实现/默认值）。
 */
expect val IS_MACOS: Boolean

/**
 * 文本输入是否需要**额外绑定 `text()`** 才能显示已存值（仅 web 为 true）。
 *
 * web（h5）渲染器的 `setProp` 不处理 `textInputState` attr（只认 `text` prop），
 * 只绑 textInputState 时输入框永远显示 placeholder——用户会误读成「保存的值丢了」；
 * 原生渲染器（Catalyst/Android/iOS）由 textInputState 通道负责显示，
 * 再绑 `text()` 会打断 IME 组词形成受控回写循环（见 [AdminFormControls.labeledField] 注释），
 * 故仅 web 需要。
 */
expect val INPUT_NEEDS_TEXT_PROP: Boolean

/**
 * Pager 额外注册的平台模块：macOS 返回 [XiaoZhiModule]（测试台依赖），
 * 其他平台返回空 map（测试代码只存在于 macosArm64Main）。
 */
expect fun Pager.platformExternalModules(): Map<String, Module>

/**
 * 测试台 section 渲染（mac-only）。菜单已在 web 端隐藏（[AdminNav.visible]），
 * 非 mac 平台的 actual 是空实现，仅作防御。
 *
 * ⚠️ 必须在目标容器闭包内**非限定**调用（接收者 = 内容区容器），否则节点挂到根容器布局逃逸；
 * twoColumn/pageWidth/pageHeight 须为 lambda，在 vif/attr 闭包内读取 pageViewWidth 才响应 resize。
 */
expect fun ViewContainer<*, *>.renderTestbenchSection(
    pager: Pager,
    form: ConfigFormState,
    twoColumn: () -> Boolean,
    pageWidth: () -> Float,
    pageHeight: () -> Float,
)

/**
 * macOS「切换服务器」hook（web 空实现）：
 * - 在调用处（侧边栏内）渲染「切换服务器」入口项；
 * - 展开时在侧边栏内联显示地址输入表单，保存后写回平台存储并回调 [onApplied]
 *   （壳层据此重跑数据加载），[onClosed] 收起表单。
 */
expect fun ViewContainer<*, *>.macServerSwitchEntry(
    pager: Pager,
    open: () -> Boolean,
    onClosed: () -> Unit,
    onApplied: () -> Unit,
)

/**
 * 管理 API 基址：web 返回 ""（server 同域托管，走相对路径）；
 * macOS 返回用户记忆的服务器地址（SP key "xz.server.baseUrl"，空则回退本地默认）。
 */
expect fun pagerBaseUrl(pager: Pager): String

/**
 * unix 秒 → 本地时区 `"yyyy-MM-dd HH:mm:ss"`（EchoMuse 设备的 first_seen/last_seen
 * 等时间戳展示统一走这里；commonMain 无日期库，由平台 API 换算本地时区）。
 */
expect fun formatEpochSeconds(secs: Long): String

/**
 * 复制文本到系统剪贴板，返回是否成功。
 * web 管理台常以 `http://LAN-IP` 访问（非 secure context，无 async Clipboard API），
 * actual 内回落 `execCommand('copy')`；不支持剪贴板的平台返回 false（调用方 toast 提示）。
 */
expect fun copyToClipboard(text: String): Boolean
