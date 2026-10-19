package com.xiaozhi.admin

import com.tencent.kuikly.core.base.ViewContainer
import com.tencent.kuikly.core.module.Module
import com.tencent.kuikly.core.pager.Pager

/** Web：无 macOS 专属能力（测试台菜单已隐藏，服务器地址走同域相对路径）。 */
actual val IS_MACOS: Boolean = false

/** web 渲染器 setProp 不处理 textInputState attr，必须绑 text() 才显示已存值。 */
actual val INPUT_NEEDS_TEXT_PROP: Boolean = true

actual fun Pager.platformExternalModules(): Map<String, Module> = emptyMap()

actual fun ViewContainer<*, *>.renderTestbenchSection(
    pager: Pager,
    form: ConfigFormState,
    twoColumn: () -> Boolean,
    pageWidth: () -> Float,
    pageHeight: () -> Float,
) {
    // 防御分支：web 菜单已隐藏（AdminNav.visible），正常不会走到
}

actual fun ViewContainer<*, *>.macServerSwitchEntry(
    pager: Pager,
    open: () -> Boolean,
    onClosed: () -> Unit,
    onApplied: () -> Unit,
) {
    // web 由 server 同域托管，无「切换服务器」概念
}

actual fun pagerBaseUrl(pager: Pager): String = ""
