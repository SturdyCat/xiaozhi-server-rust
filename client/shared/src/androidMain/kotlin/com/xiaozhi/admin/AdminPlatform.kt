package com.xiaozhi.admin

import androidx.compose.runtime.Composable
import com.tencent.kuikly.core.module.Module
import com.tencent.kuikly.core.nvi.serialization.json.JSONObject
import com.tencent.kuikly.core.pager.Pager

/** Android：无 macOS 专属能力（测试台与「切换服务器」为 mac-only）。 */
actual val IS_MACOS: Boolean = false

actual fun Pager.platformExternalModules(): Map<String, Module> = emptyMap()

@Composable
actual fun TestbenchSection(page: AdminPage) {
    // 防御分支：Android 侧边栏不渲染测试台入口
}

actual fun Pager.startAdminServerDiscovery(onResult: (JSONObject?) -> Unit) {
    // Android 无 mDNS 能力（测试台与「切换服务器」为 mac-only）
}

actual fun Pager.stopAdminServerDiscovery() {
}

actual fun pagerBaseUrl(pager: Pager): String = ""

/** Android 宿主未接 Context 桥，暂不支持一键复制（调用方 toast 提示）。 */
actual fun copyToClipboard(text: String): Boolean = false
