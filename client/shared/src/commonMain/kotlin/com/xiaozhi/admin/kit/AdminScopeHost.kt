package com.xiaozhi.admin.kit

import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import com.tencent.kuikly.compose.foundation.background
import com.tencent.kuikly.compose.foundation.layout.Box
import com.tencent.kuikly.compose.foundation.layout.BoxScope
import com.tencent.kuikly.compose.foundation.layout.fillMaxSize
import com.tencent.kuikly.compose.ui.Modifier
import com.tencent.kuikly.compose.ui.layout.onSizeChanged
import com.tencent.kuikly.compose.ui.platform.LocalDensity
import com.tencent.kuikly.compose.ui.unit.dp
import com.gearui.theme.Theme
import com.tencent.kuikly.core.pager.Pager
import com.xiaozhi.admin.contracts.AdminFieldHelpState

/**
 * 页面根容器：量视口 + 下发 [LocalAdminPager] / [LocalAdminViewport] / [LocalFieldHelp]。
 *
 * ## 为什么抽成一个组件
 *
 * `@Page("admin")` 与 `@Page("config")` 是两个 Compose 页面，浅层组件（卡片、字段、弹层）
 * 都靠这三个 CompositionLocal 取"当前页面/当前视口/帮助状态"。若各页各写一遍
 * `remember + onSizeChanged + CompositionLocalProvider`，断点口径迟早分叉。
 *
 * ## 视口为什么用 `onSizeChanged` 而不是 `pagerData.pageViewWidth`
 *
 * 后者是一次性取值，窗口 resize 不触发重排。这里的 [AdminViewport] 是 Compose 状态：
 * 拖动窗口跨过断点会实时重排（方案 D4 的要求，比旧实现强）。
 *
 * @param pager 当前页面（Kuikly `Pager`；`ComposeContainer` 本身就是 Pager）。
 * @param fieldHelp 字段帮助状态；null = 本页不渲染 ⓘ（与旧 `FieldHelpHost` 未实现的语义一致）。
 * @param content 收到**真实视口**的内容；首帧为 0×0（`isNarrow` 为 true）。
 */
@Composable
fun AdminScopeHost(
    pager: Pager,
    fieldHelp: AdminFieldHelpState?,
    modifier: Modifier = Modifier,
    content: @Composable BoxScope.(AdminViewport) -> Unit,
) {
    // 初值 0×0：首帧按窄屏渲染（抽屉形态），onSizeChanged 立刻用真实尺寸覆盖。
    // 不默认成宽屏，是为了避免窄屏设备首帧闪一下 240dp 侧栏再抽走。
    var viewport by remember { mutableStateOf(AdminViewport(0.dp, 0.dp)) }
    val density = LocalDensity.current

    CompositionLocalProvider(
        LocalAdminPager provides pager,
        LocalAdminViewport provides viewport,
        LocalFieldHelp provides fieldHelp,
    ) {
        Box(
            modifier = modifier
                .fillMaxSize()
                .background(Theme.colors.background)
                .onSizeChanged { size ->
                    viewport = with(density) {
                        AdminViewport(width = size.width.toDp(), height = size.height.toDp())
                    }
                },
        ) {
            content(viewport)
        }
    }
}
