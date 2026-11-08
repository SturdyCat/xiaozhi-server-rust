package com.xiaozhi.admin

//! 插件「详情」弹窗：功能介绍与使用方法（manifest description / version / api_version /
//! permissions / implements / requires 满足状态 / dependents；Active 另附 registration
//! 摘要与动态页面清单）。内容全部来自 [PluginDetailModel] 纯组装——**零请求、零业务**：
//! 插件没声明的信息这里不出现，也不为未激活插件伪造 registration 摘要。

import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.rememberUpdatedState
import com.gearui.foundation.primitives.ScrollView
import com.gearui.foundation.primitives.Text
import com.gearui.overlay.LocalOverlayController
import com.gearui.overlay.OverlayDefaults
import com.gearui.overlay.OverlayDismissPolicy
import com.gearui.overlay.OverlayOptions
import com.gearui.overlay.OverlayPlacement
import com.gearui.primitives.composite.Card
import com.gearui.theme.Theme
import com.tencent.kuikly.compose.foundation.layout.Arrangement
import com.tencent.kuikly.compose.foundation.layout.Column
import com.tencent.kuikly.compose.foundation.layout.Row
import com.tencent.kuikly.compose.foundation.layout.fillMaxSize
import com.tencent.kuikly.compose.foundation.layout.fillMaxWidth
import com.tencent.kuikly.compose.foundation.layout.height
import com.tencent.kuikly.compose.foundation.layout.padding
import com.tencent.kuikly.compose.foundation.layout.width
import com.tencent.kuikly.compose.ui.Alignment
import com.tencent.kuikly.compose.ui.Modifier
import com.tencent.kuikly.compose.ui.unit.dp
import com.xiaozhi.admin.contracts.IconToken
import com.xiaozhi.admin.contracts.PluginDetailModel
import com.xiaozhi.admin.contracts.PluginSlotPlugin
import com.xiaozhi.admin.kit.HintLine
import com.xiaozhi.admin.kit.LabeledCard
import com.xiaozhi.admin.kit.LocalAdminPager
import com.xiaozhi.admin.kit.LocalAdminSpacing
import com.xiaozhi.admin.kit.LocalAdminViewport
import com.xiaozhi.admin.kit.LocalFieldHelp

/**
 * 插件详情弹窗：GearUI 居中 modal overlay（与设备详情弹窗同一套宿主模式——遮罩外点击
 * 不关闭；Pager/viewport/spacing/fieldHelp 显式穿过 overlay 门户）。内容是快照静态渲染：
 * 打开时的清单快照是什么就展示什么，不做二次请求；生命周期变化由清单刷新驱动。
 */
@Composable
internal fun PluginManageDetailDialog(
    plugin: PluginSlotPlugin,
    uiPages: List<String>,
    onDismiss: () -> Unit,
) {
    val overlay = LocalOverlayController.current
    val currentDismiss = rememberUpdatedState(onDismiss)
    val pager = LocalAdminPagerOrThrow()
    val viewport = rememberUpdatedState(LocalAdminViewport.current)
    val spacing = rememberUpdatedState(LocalAdminSpacing.current)
    val fieldHelp = LocalFieldHelp.current
    DisposableEffect(plugin.id) {
        // 纯快照组装（零 IO）：弹窗生命周期内不重算；刷新清单后重开即见新内容
        val model = PluginDetailModel.from(plugin, uiPages)
        val overlayId = overlay.show(
            options = OverlayOptions(
                placement = OverlayPlacement.Center,
                modal = true,
                maskColor = OverlayDefaults.scrimColor,
                dismissPolicy = OverlayDismissPolicy.Modal.copy(outsideClick = false),
            ),
            onDismiss = { currentDismiss.value() },
        ) {
            CompositionLocalProvider(
                LocalAdminPager provides pager,
                LocalAdminViewport provides viewport.value,
                LocalAdminSpacing provides spacing.value,
                LocalFieldHelp provides fieldHelp,
            ) {
                PluginDetailDialogContent(model, onDismiss = { currentDismiss.value() })
            }
        }
        onDispose { overlayId?.let { overlay.close(it) } }
    }
}

@Composable
private fun PluginDetailDialogContent(model: PluginDetailModel, onDismiss: () -> Unit) {
    val viewport = LocalAdminViewport.current
    val spacing = LocalAdminSpacing.current
    Card(
        modifier = Modifier
            .width((viewport.width.value * 0.85f).coerceIn(1f, 640f).dp)
            // 弹窗固定尺寸口径与设备详情一致（min(95%, 640) × min(90%, 620)），长内容内部滚动
            .height((viewport.height.value * 0.90f).coerceIn(1f, 620f).dp),
    ) {
        Column(
            modifier = Modifier.fillMaxSize().padding(spacing.md),
            verticalArrangement = Arrangement.spacedBy(spacing.sm),
        ) {
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(spacing.sm)) {
                Column(modifier = Modifier.weight(1f)) {
                    Text(
                        text = model.title,
                        style = Theme.typography.titleMedium,
                        color = Theme.colors.foreground,
                    )
                    Text(
                        text = model.lifecycleLine,
                        style = Theme.typography.caption,
                        color = Theme.colors.mutedForeground,
                    )
                }
                EchoMuseActionButton(icon = IconToken.Close, contentDescription = "关闭插件详情", onClick = onDismiss)
            }
            ScrollView(modifier = Modifier.weight(1f).fillMaxWidth()) {
                Column(modifier = Modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(spacing.sm)) {
                    for (section in model.sections) {
                        LabeledCard(title = section.title) {
                            for (line in section.lines) {
                                HintLine(line)
                            }
                        }
                    }
                }
            }
        }
    }
}
