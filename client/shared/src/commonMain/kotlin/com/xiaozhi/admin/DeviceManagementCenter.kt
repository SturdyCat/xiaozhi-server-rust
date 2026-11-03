package com.xiaozhi.admin

import androidx.compose.runtime.*
import com.gearui.components.alert.Alert
import com.gearui.components.alert.AlertStatus
import com.gearui.components.button.Button
import com.gearui.components.button.ButtonType
import com.gearui.components.empty.EmptyState
import com.gearui.foundation.primitives.ScrollView
import com.gearui.foundation.primitives.Text
import com.gearui.overlay.*
import com.gearui.primitives.composite.Card
import com.gearui.theme.Theme
import com.tencent.kuikly.compose.foundation.layout.*
import com.tencent.kuikly.compose.ui.Alignment
import com.tencent.kuikly.compose.ui.Modifier
import com.tencent.kuikly.compose.ui.unit.dp
import com.xiaozhi.admin.contracts.IconToken
import com.xiaozhi.admin.kit.GearUiBreakpoints
import com.xiaozhi.admin.kit.LocalAdminPager
import com.xiaozhi.admin.kit.LocalAdminSpacing
import com.xiaozhi.admin.kit.LocalAdminViewport
import com.xiaozhi.admin.kit.LocalFieldHelp
import com.xiaozhi.admin.kit.statusBadge

/**
 * 「设备管理」中心（侧栏唯一入口 `devicehub`，2026-11-03 由原 EchoMuse / 涂鸦设备两页合并）。
 *
 * 形态：顶部 title 栏（摘要 + 扩展工具 + 刷新 + **齿轮一次性配置**）→ 设备卡片列表
 * （EchoMuse 卡 + 涂鸦卡，同一套栅格断点）→ 点卡片弹各自详情（EchoMuse 六标签弹窗 /
 * [TuyaDeviceDialog] DP 弹窗）。配置是**一次性**的：接入开关与凭据在齿轮弹层 /
 * `openConfigSection` 跳转的唯一配置表单里，不在设备卡片里重复。
 *
 * 多入口复用：配置卡（EchoMuse 接入 / Tuya 接入）尾部的「管理设备」按钮跳回本页；
 * 语音流程的 EchoMuse 接入节点也经同一配置卡到达这里——运行面全仓只此一份。
 */
@Composable
fun DeviceManagementCenter(
    echoMuse: EchoMuseState,
    tuya: TuyaState,
    plugins: PluginMetaState,
    toast: ToastState,
    isActive: () -> Boolean,
    apiBase: String = "",
    token: String = "",
    openConfigSection: (sectionId: String) -> Unit = {},
    modelFiles: @Composable () -> Unit = {},
) {
    val pager = LocalAdminPagerOrThrow()
    val active = isActive()
    val viewport = LocalAdminViewport.current
    val spacing = LocalAdminSpacing.current
    val contentWidth = viewport.width.value -
        (if (viewport.isNarrow) 0f else GearUiBreakpoints.SIDEBAR_WIDTH + 1f) - spacing.pagePadding.value * 2
    val columns = ((contentWidth + spacing.gutter.value) / (240f + spacing.gutter.value)).toInt().coerceAtLeast(1)
    val cardWidth = ((contentWidth - spacing.gutter.value * (columns - 1)) / columns).coerceAtLeast(1f)

    var extensions by remember(apiBase, token) { mutableStateOf(false) }
    var selectedEmId by remember(apiBase, token) { mutableStateOf<String?>(null) }
    var selectedTuyaId by remember(apiBase, token) { mutableStateOf<String?>(null) }
    var gearOpen by remember(apiBase, token) { mutableStateOf(false) }
    val selectedEm = echoMuse.devices.firstOrNull { it.deviceId == selectedEmId }
    val selectedTuya = tuya.devices.firstOrNull { it.deviceId == selectedTuyaId }

    // 设备插件注册门控（GET /api/plugins 的 device_plugins 投影）：注册后分区/菜单才出现。
    // 旧服务端未上报该键时按已注册兜底（与侧栏门控同一口径，不闪隐）。
    val emRegistered = devicePluginRegistered(plugins, "echomuse")
    val tuyaRegistered = devicePluginRegistered(plugins, "tuya")
    val noneRegistered =
        plugins.loaded && plugins.devicePluginRows.isNotEmpty() && !emRegistered && !tuyaRegistered

    // EchoMuse 实时更新（管理台 WS）；离页/切服务器释放轮询与 shell，并兜底清两路详情态
    //（TuyaDeviceDialog 自身 dispose 也会 closeDetail，这里兜防止宿主形态变化时残留）
    DisposableEffect(echoMuse, active, apiBase, token) {
        echoMuse.ensureLiveUpdates(pager) { active }
        onDispose {
            echoMuse.ensureLiveUpdates(pager) { false }
            echoMuse.detail.leave()
            tuya.closeDetail()
        }
    }
    // 首次进入补拉涂鸦目录（AdminPage.refreshAll 启动已加载时 loading 闸门直接跳过；切服务器兜底）
    LaunchedEffect(active, apiBase, token) {
        if (active && !tuya.loaded && !tuya.loading) tuya.load(pager, apiBase)
    }
    // 选中设备从实时列表反查：列表里没了（被删除/换令牌）就收起弹窗
    LaunchedEffect(selectedEmId, selectedEm?.deviceId, echoMuse.loaded) {
        if (echoMuse.loaded && selectedEmId != null && selectedEm == null) selectedEmId = null
    }
    LaunchedEffect(selectedTuyaId, selectedTuya?.deviceId, tuya.loaded) {
        if (tuya.loaded && selectedTuyaId != null && selectedTuya == null) selectedTuyaId = null
    }

    Column(modifier = Modifier.fillMaxSize()) {
        // ---- title 栏：摘要 + 扩展工具 + 刷新 + 齿轮一次性配置 ----
        Row(
            modifier = Modifier.fillMaxWidth().padding(spacing.pagePadding),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(spacing.sm),
        ) {
            // title 栏两层：固定标题「设备管理」+ 摘要（扩展模式摘要换成选择提示，标题不消失）
            Column(modifier = Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
                Text(
                    text = "设备管理", style = Theme.typography.titleMedium,
                    color = Theme.colors.foreground,
                )
                Text(
                    text = if (extensions) "自定义扩展 · 选择设备" else deviceHubSummary(echoMuse, tuya),
                    style = Theme.typography.caption, color = Theme.colors.mutedForeground,
                )
            }
            Button(
                text = if (extensions) "返回设备管理" else "扩展工具", type = ButtonType.OUTLINE,
                onClick = {
                    selectedEmId = null; selectedTuyaId = null
                    echoMuse.detail.leave(); extensions = !extensions
                },
            )
            EchoMuseRefreshButton(loading = echoMuse.loading || tuya.loading, onClick = {
                tuya.refresh(pager)
                echoMuse.refresh(pager)
            })
            EchoMuseActionButton(icon = IconToken.Settings, contentDescription = "设备接入配置", onClick = { gearOpen = true })
        }
        ScrollView(modifier = Modifier.weight(1f).fillMaxWidth()) {
            Column(
                modifier = Modifier.fillMaxWidth().padding(horizontal = spacing.pagePadding),
                verticalArrangement = Arrangement.spacedBy(spacing.gutter),
            ) {
                if (echoMuse.loadError.isNotEmpty()) {
                    Alert(title = "EchoMuse 设备列表加载失败", description = echoMuse.loadError,
                        status = AlertStatus.DANGER, modifier = Modifier.fillMaxWidth())
                }
                if (tuya.loadError.isNotEmpty()) {
                    Alert(title = "涂鸦设备加载失败", description = tuya.loadError,
                        status = AlertStatus.DANGER, modifier = Modifier.fillMaxWidth())
                }
                if (echoMuse.loading && !echoMuse.loaded) EmNote("正在读取 EchoMuse 设备列表…")
                if (noneRegistered) {
                    EmptyState(
                        message = "没有已注册的设备插件",
                        description = "EchoMuse 与涂鸦设备都是插件：在「EchoMuse 接入」或「Tuya 接入」配置开启（注册）后，对应设备分区与管理入口会出现在这里。",
                        modifier = Modifier.fillMaxWidth(),
                    )
                }
                // 扩展模式只留 EchoMuse 卡（扩展工作区按设备进入）；涂鸦区隐藏
                if (emRegistered) DeviceHubEchoMuseBlock(
                    echoMuse = echoMuse, cardWidth = cardWidth, columns = columns,
                    registration = plugins.devicePlugin("echomuse"),
                    onOpen = { selectedEmId = it }, onOpenConfig = { openConfigSection("echomuse") },
                )
                if (!extensions && tuyaRegistered) DeviceHubTuyaBlock(
                    tuya = tuya, cardWidth = cardWidth, columns = columns,
                    registration = plugins.devicePlugin("tuya"),
                    onOpen = { selectedTuyaId = it }, onOpenConfig = { openConfigSection("tuya") },
                    onSync = { tuya.syncNow(pager) { ok, msg -> toast.show(msg, if (ok) "ok" else "error") } },
                )
            }
        }
    }
    EchoMuseManagementDialog(
        row = selectedEm.takeIf { active && echoMuse.enabled }, state = echoMuse, toast = toast,
        apiBase = apiBase, token = token, onDismiss = { selectedEmId = null },
        modelFiles = modelFiles, extensions = extensions,
    )
    TuyaDeviceDialog(
        row = selectedTuya.takeIf { active && tuya.enabled && !extensions },
        state = tuya, toast = toast, onDismiss = { selectedTuyaId = null },
    )
    DeviceAccessConfigDialog(
        visible = gearOpen && active,
        echoMuse = echoMuse, tuya = tuya, plugins = plugins, toast = toast,
        onDismiss = { gearOpen = false },
        openConfigSection = openConfigSection,
    )
}

/** 注册门控兜底：旧服务端未上报该插件行时按已注册处理（维持合并前行为，不闪隐）。 */
private fun devicePluginRegistered(plugins: PluginMetaState, id: String): Boolean =
    plugins.devicePlugin(id)?.registered ?: true

/** title 栏摘要：两路设备合计 + 在线数（涂鸦未配置/未启用时如实说，不冒充 0 台）。 */
private fun deviceHubSummary(em: EchoMuseState, tuya: TuyaState): String {
    if (em.loading && !em.loaded && tuya.loading && !tuya.loaded) return "加载中…"
    val online = em.devices.count { it.online } + tuya.devices.count { it.online }
    return "EchoMuse ${em.devices.size} 台 · 涂鸦 ${tuya.devices.size} 台 · 在线 $online"
}

@Composable
private fun DeviceHubSectionLabel(title: String, count: Int, phase: String = "active") {
    Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(LocalAdminSpacing.current.xs)) {
        Text(text = title, style = Theme.typography.titleSmall, color = Theme.colors.foreground)
        Text(text = "$count 台", style = Theme.typography.caption, color = Theme.colors.mutedForeground)
        // 只在偏离时打标（正常/在线不渲染徽标）；注册门控相位用于调试插件运行状态
        if (phase == "degraded") statusBadge(state = "busy", text = "降级")
        if (phase == "failed") statusBadge(state = "error", text = "异常")
    }
}

@Composable
private fun DeviceHubEchoMuseBlock(
    echoMuse: EchoMuseState,
    cardWidth: Float,
    columns: Int,
    registration: DevicePluginRow?,
    onOpen: (String) -> Unit,
    onOpenConfig: () -> Unit,
) {
    DeviceHubSectionLabel("EchoMuse", echoMuse.devices.size, registration?.phase ?: "active")
    registration?.takeIf { it.reason.isNotEmpty() }?.let { EmNote(it.reason) }
    if (echoMuse.loaded && !echoMuse.enabled) {
        EmNote("EchoMuse 未启用。到「EchoMuse 接入」配置开启设备入口并重启服务端。")
        Button(text = "打开接入配置", type = ButtonType.OUTLINE, onClick = onOpenConfig)
    } else if (echoMuse.enabled) {
        if (echoMuse.loaded && echoMuse.devices.isEmpty()) {
            EmptyState(
                message = "还没有 EchoMuse 设备登记",
                description = "设备首次连上服务端后会出现在这里，经批准后即可管理。",
                modifier = Modifier.fillMaxWidth(),
            )
        }
        DeviceHubCardGrid(columns = columns, count = echoMuse.devices.size) { index ->
            val row = echoMuse.devices[index]
            EchoMuseDeviceCard(row = row, onOpen = { onOpen(row.deviceId) }, modifier = Modifier.width(cardWidth.dp))
        }
    }
}

@Composable
private fun DeviceHubTuyaBlock(
    tuya: TuyaState,
    cardWidth: Float,
    columns: Int,
    registration: DevicePluginRow?,
    onOpen: (String) -> Unit,
    onOpenConfig: () -> Unit,
    onSync: () -> Unit,
) {
    DeviceHubSectionLabel("涂鸦设备", tuya.devices.size, registration?.phase ?: "active")
    registration?.takeIf { it.reason.isNotEmpty() }?.let { TuyaNote(it.reason) }
    if (!tuya.enabled) {
        TuyaNote("涂鸦设备未启用。到「Tuya 接入」配置开启（保存即生效，无需重启）。")
        Button(text = "打开接入配置", type = ButtonType.OUTLINE, onClick = onOpenConfig)
    } else {
        if (tuya.loaded && !tuya.configured) {
            TuyaNote("凭证未配置：点右上角齿轮录入涂鸦云凭证，或到「Tuya 接入」配置。")
        }
        if (tuya.loaded && tuya.devices.isEmpty()) {
            EmptyState(
                message = "还没有涂鸦设备",
                description = "录入凭证后点「立即同步」拉取账号下的设备目录；控制全部走服务端局域网直连。",
                actionText = "立即同步", onAction = onSync, modifier = Modifier.fillMaxWidth(),
            )
        }
        DeviceHubCardGrid(columns = columns, count = tuya.devices.size) { index ->
            val row = tuya.devices[index]
            TuyaDeviceCard(row = row, modifier = Modifier.width(cardWidth.dp), onOpen = { onOpen(row.deviceId) })
        }
        if (tuya.storePath.isNotEmpty()) {
            Text(
                text = "设备目录存储：${tuya.storePath}",
                style = Theme.typography.caption, color = Theme.colors.mutedForeground,
            )
        }
    }
}

/** 统一栅格：按可用宽度换列（chunked 行 + gutter），两种来源卡同一口径。 */
@Composable
private fun DeviceHubCardGrid(columns: Int, count: Int, item: @Composable (Int) -> Unit) {
    val spacing = LocalAdminSpacing.current
    for (group in (0 until count).chunked(columns)) {
        Row(
            modifier = Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.spacedBy(spacing.gutter),
        ) {
            for (index in group) item(index)
        }
    }
}

/**
 * 齿轮「设备接入配置」弹层：**一次性配置**的集中入口。
 *
 * - EchoMuse：只放状态与「打开接入配置」跳转（开关/令牌/欢迎语在语音流程的接入节点，
 *   唯一配置表单，这里不复制第二份）；
 * - Tuya：凭证表单就在这里（`POST /api/tuya/credentials` 是唯一录入路径）+ 立即同步 +
 *   「Tuya 接入」配置跳转（开关/同步周期在全局配置）。
 */
@Composable
private fun DeviceAccessConfigDialog(
    visible: Boolean,
    echoMuse: EchoMuseState,
    tuya: TuyaState,
    plugins: PluginMetaState,
    toast: ToastState,
    onDismiss: () -> Unit,
    openConfigSection: (sectionId: String) -> Unit,
) {
    val overlay = LocalOverlayController.current
    val currentDismiss = rememberUpdatedState(onDismiss)
    val currentOpenConfig = rememberUpdatedState(openConfigSection)
    val currentToast = rememberUpdatedState(toast)
    val currentPlugins = rememberUpdatedState(plugins)
    val pager = LocalAdminPager.current
    val viewport = rememberUpdatedState(LocalAdminViewport.current)
    val spacing = rememberUpdatedState(LocalAdminSpacing.current)
    val fieldHelp = LocalFieldHelp.current
    DisposableEffect(visible) {
        val overlayId = if (visible) overlay.show(
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
                DeviceAccessConfigContent(
                    echoMuse = echoMuse, tuya = tuya, plugins = currentPlugins.value,
                    toast = currentToast.value,
                    onDismiss = { currentDismiss.value() },
                    openConfigSection = { currentOpenConfig.value(it) },
                )
            }
        } else null
        onDispose { overlayId?.let { overlay.close(it) } }
    }
}

@Composable
private fun DeviceAccessConfigContent(
    echoMuse: EchoMuseState,
    tuya: TuyaState,
    plugins: PluginMetaState,
    toast: ToastState,
    onDismiss: () -> Unit,
    openConfigSection: (sectionId: String) -> Unit,
) {
    val pager = LocalAdminPagerOrThrow()
    val viewport = LocalAdminViewport.current
    val spacing = LocalAdminSpacing.current
    var confirmClear by remember { mutableStateOf(false) }
    Card(
        modifier = Modifier
            .width((viewport.width.value * 0.95f).coerceIn(1f, 620f).dp)
            .height((viewport.height.value * 0.90f).coerceIn(1f, 640f).dp),
    ) {
        Column(modifier = Modifier.fillMaxSize().padding(spacing.md), verticalArrangement = Arrangement.spacedBy(spacing.sm)) {
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(spacing.sm)) {
                Text(
                    text = "设备接入配置",
                    style = Theme.typography.titleMedium, color = Theme.colors.foreground,
                    modifier = Modifier.weight(1f),
                )
                EchoMuseActionButton(icon = IconToken.Close, contentDescription = "关闭设备接入配置", onClick = onDismiss)
            }
            TuyaNote("一次性配置：接入开关与凭据只在这里和配置页维护；设备列表用顶部刷新按钮更新。")
            ScrollView(modifier = Modifier.weight(1f).fillMaxWidth()) {
                Column(
                    modifier = Modifier.fillMaxWidth(),
                    verticalArrangement = Arrangement.spacedBy(spacing.md),
                ) {
                    // ---- EchoMuse：状态 + 跳转（配置在语音流程接入节点，唯一表单不复制）----
                    // 注册门控：插件未注册（配置段关闭）时不出现该块（注册后菜单才出现）
                    if (devicePluginRegistered(plugins, "echomuse")) {
                    Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(spacing.sm)) {
                        Text(
                            text = "EchoMuse 接入", style = Theme.typography.titleSmall,
                            color = Theme.colors.foreground, modifier = Modifier.weight(1f),
                        )
                        Text(
                            text = if (echoMuse.enabled) "已启用" else "未启用",
                            style = Theme.typography.caption,
                            color = if (echoMuse.enabled) Theme.colors.primary else Theme.colors.warning,
                        )
                    }
                    EmNote("开关 / 接入令牌 / 欢迎语在「语音流程 → 终端与音频入口 → EchoMuse 接入」节点编辑（重启生效）。")
                    Button(
                        text = "打开接入配置", type = ButtonType.OUTLINE,
                        onClick = { onDismiss(); openConfigSection("echomuse") },
                    )
                    }
                    // ---- Tuya：凭证表单（唯一录入路径）+ 同步 + 配置跳转 ----
                    if (devicePluginRegistered(plugins, "tuya")) {
                    Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(spacing.sm)) {
                        Text(
                            text = "涂鸦设备", style = Theme.typography.titleSmall,
                            color = Theme.colors.foreground, modifier = Modifier.weight(1f),
                        )
                        Text(
                            text = if (!tuya.enabled) "未启用"
                            else if (tuya.configured) "已配置 · ${tuya.accessIdMasked} · ${tuya.region}"
                            else "凭证未配置",
                            style = Theme.typography.caption,
                            color = if (tuya.enabled && tuya.configured) Theme.colors.primary else Theme.colors.warning,
                        )
                    }
                    if (tuya.enabled) {
                        TuyaCredentialsCard(
                            state = tuya,
                            confirmClear = confirmClear,
                            onConfirmClearChange = { confirmClear = it },
                            onDone = { ok, msg -> toast.show(msg, if (ok) "ok" else "error") },
                        )
                        Row(horizontalArrangement = Arrangement.spacedBy(spacing.sm)) {
                            Button(
                                text = if (tuya.syncing) "同步中…" else "立即同步",
                                type = ButtonType.FILL, loading = tuya.syncing, disabled = tuya.syncing,
                                onClick = {
                                    tuya.syncNow(pager) { ok, msg -> toast.show(msg, if (ok) "ok" else "error") }
                                },
                            )
                            Button(
                                text = "打开接入配置", type = ButtonType.OUTLINE,
                                onClick = { onDismiss(); openConfigSection("tuya") },
                            )
                        }
                        Text(
                            text = tuya.lastSyncText,
                            style = Theme.typography.caption, color = Theme.colors.mutedForeground,
                        )
                    } else {
                        TuyaNote("涂鸦设备未启用：/api/tuya/* 全部 404（凭证录入同样不可用）。先到「Tuya 接入」开启（保存即生效），再回来录入凭证。")
                        Button(
                            text = "打开接入配置", type = ButtonType.OUTLINE,
                            onClick = { onDismiss(); openConfigSection("tuya") },
                        )
                    }
                    }
                    // 两个插件都未注册：说明注册入口在哪（旧服务端无投影时不会走到这里）
                    if (!devicePluginRegistered(plugins, "echomuse") && !devicePluginRegistered(plugins, "tuya")) {
                        TuyaNote("没有已注册的设备插件：在「EchoMuse 接入」或「Tuya 接入」配置开启后，这里会出现对应的一次性配置。")
                    }
                }
            }
        }
    }
}
