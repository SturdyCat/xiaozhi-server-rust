package com.xiaozhi.admin

import com.tencent.kuikly.core.base.Border
import com.tencent.kuikly.core.base.BorderStyle
import com.tencent.kuikly.core.base.CursorType
import com.tencent.kuikly.core.base.ViewContainer
import com.tencent.kuikly.core.directives.vif
import com.tencent.kuikly.core.pager.Pager
import com.tencent.kuikly.core.views.Slider
import com.tencent.kuikly.core.views.Text
import com.tencent.kuikly.core.views.View

/**
 * 一台 EchoMuse 设备卡片的渲染（页面壳体见 `EchoMusePage.kt`，菜单状态见 [EchoMuseState]）。
 *
 * 布局（自绘容器，视觉与 [groupedCard] 一致——标题要自定义 header，不走 groupedCard(title)）：
 *
 * ```
 * ┌────────────────────────────────────────────────┐
 * │ 标题  ● 在线 [待批准]                 （[del] 删除） │ ← header：删除 icon 按钮进行内二次确认
 * ├────────────────────────────────────────────────┤
 * │ 概览        │ 设备号  xx:xx…        [复制信息]    │
 * │ WiFi 配置   │ 版本    1.2.3                      │ ← 卡内左菜单 + 右内容面板
 * │ 唤醒词设置  │ …                                  │   （menuPaneRow 原语，与
 * │ 设备管理    │                                    │     ConfigTabs 工作区同一出处）
 * │ 蓝牙代理    │                                    │
 * └────────────────────────────────────────────────┘
 * ```
 *
 * 菜单为**全局单选**（[EchoMuseState.menuDevice]/[menuItem]）：同一时刻只有一张卡展开
 * 非概览面板，切卡/切菜单项都更新同一组 observable；未批准设备只有 概览/批准 两项。
 * 「设备管理」「蓝牙代理」是把原独立卡片（[echoMuseManagementPanel]/[echoMuseBlePanel]）
 * 折进卡内菜单——面板内容与原卡完全一致，只是入口从展开按钮变成菜单项。
 *
 * ⚠️ Kuikly 响应式铁律：菜单选中判断必须是 lambda 且只在 attr/vif 闭包内读 observable；
 * 渲染函数须在目标容器闭包内**非限定**调用（经 `ctx.` 调用会布局逃逸）。
 */

/** 卡片内左列菜单宽度（紧凑菜单；右栏面板 flex(1) 占满剩余宽度）。 */
private const val ECHOMUSE_MENU_WIDTH = 150f

/**
 * 唤醒词面板里滑杆之外的全部固定占位（滚动页左右留白 + 卡内边距 + 左菜单 + 竖分隔线 +
 * 面板左内距 + 右侧阈值读数）。滑杆宽度 = 内容区宽 − 本占位，见 [echoMuseSliderWidth]。
 */
private val ECHOMUSE_SLIDER_CHROME: Float =
    AdminSpace.xl * 2 + AdminSpace.cardPadding * 2 + ECHOMUSE_MENU_WIDTH + 1f + AdminSpace.md + 48f + AdminSpace.sm

/** 滑杆最小宽度（窄窗兜底；低于此值宁可窄也不为负）。 */
private const val ECHOMUSE_SLIDER_MIN_WIDTH = 120f

/**
 * 唤醒灵敏度滑杆的显式宽度。
 *
 * ⚠️ 官方 `Slider` 是**组合组件**（track/progress/thumb 三层自绘），渲染时读自身 attr 的
 * 宽高——`flex(1f)` 不参与该判定，读不到就 `throwRuntimeError`（日志「Slider组件需要设置
 * 宽度和高度」）。异常会中断所在面板的构建事务：实测唤醒词面板（连同前面已构建的开关、
 * 下拉、文案）整片空白，只有卡片 header 和左菜单还在。因此这里按内容区宽度算出**确定值**，
 * 不能用 flex；读取放在 attr 闭包内，窗口宽度变化时随 `pageViewWidth` 响应式更新。
 */
private fun echoMuseSliderWidth(ctx: Pager): Float =
    (adminContentWidth(ctx) - ECHOMUSE_SLIDER_CHROME).coerceAtLeast(ECHOMUSE_SLIDER_MIN_WIDTH)

/** 一台 EchoMuse 设备卡片：header（标题+状态+删除）+ 卡内左菜单/右内容面板（EchoMusePage 调用）。 */
internal fun ViewContainer<*, *>.echoMuseDeviceCard(
    row: EchoMuseDeviceRow,
    state: EchoMuseState,
    toast: ToastState,
    ctx: Pager,
    onOpenModelFiles: () -> Unit = {},
) {
    // 自绘等价 groupedCard 的容器（cardBg/radiusLg/1px divider/cardPadding/marginBottom gutter）
    View {
        attr {
            flexDirectionColumn()
            backgroundColor(AdminColors.cardBg)
            borderRadius(AdminShape.radiusLg)
            border(Border(1f, BorderStyle.SOLID, AdminColors.divider))
            padding(AdminSpace.cardPadding)
            marginBottom(AdminSpace.gutter)
        }
        echoMuseCardHeader(row, state)
        vif({ state.confirmDeleteId == row.deviceId }) { echoMuseDeleteConfirm(row, state, toast, ctx) }
        dividerH()
        // 卡内左右分栏：与配置工作区同一 menuPaneRow 原语（左菜单 │ 1px 纵向分隔线 │ 右面板）
        menuPaneRow(
            menuWidth = ECHOMUSE_MENU_WIDTH,
            panelPaddingLeft = AdminSpace.md,
            menu = {
                for ((itemId, itemLabel, itemIcon) in echoMuseMenuItems(row)) {
                    echoMuseMenuItem(
                        itemIcon,
                        itemLabel,
                        isSelected = { state.activeMenuItem(row) == itemId },
                    ) { state.selectMenu(row, itemId) }
                }
            },
            panel = { echoMusePanel(row, state, toast, ctx, onOpenModelFiles) },
        )
    }
}

// ============================================================
// header：标题 + 在线状态 + 删除（icon 按钮 + 行内二次确认）
// ============================================================

/** 卡片 header：左 = 标题 + 在线状态点（+ 待批准徽标）；右 = 删除记录 icon 按钮。 */
private fun ViewContainer<*, *>.echoMuseCardHeader(row: EchoMuseDeviceRow, state: EchoMuseState) {
    View {
        attr {
            flexDirectionRow()
            alignItemsCenter()
            paddingBottom(AdminSpace.sm)
        }
        Text {
            attr {
                fontSize(AdminType.section)
                fontWeightMedium()
                color(AdminColors.textPrimary)
                text(row.title)
            }
        }
        // 在线状态：绿点「在线」/ 灰点「离线」，文字色随状态
        View {
            attr {
                flexDirectionRow()
                alignItemsCenter()
                marginLeft(AdminSpace.sm)
            }
            View {
                attr {
                    width(8f)
                    height(8f)
                    borderRadius(AdminShape.radiusPill)
                    marginRight(AdminSpace.xs)
                    backgroundColor(if (row.online) AdminColors.accent else AdminColors.textPlaceholder)
                }
            }
            Text {
                attr {
                    fontSize(AdminType.caption)
                    color(if (row.online) AdminColors.accentTintText else AdminColors.textTertiary)
                    text(if (row.online) "在线" else "离线")
                }
            }
        }
        // 未批准：状态旁保留「待批准」徽标
        if (!row.approved) {
            View { attr { width(AdminSpace.sm) } }
            statusBadge(state = { "busy" }, text = { "待批准" })
        }
        // 中间弹性撑开：删除按钮与标题左右两端对齐
        View { attr { flex(1f) } }
        echoMuseDeleteButton(state, row.deviceId)
    }
}

/**
 * 删除记录的 icon-only 按钮（封装 [appButton]：OUTLINE + danger + [IconToken.Delete]
 * 内嵌矢量图标，替代早期手绘 🗑 emoji 圆钮）。点击只进入行内二次确认
 * （[echoMuseDeleteConfirm]），不直接删除。
 */
private fun ViewContainer<*, *>.echoMuseDeleteButton(state: EchoMuseState, deviceId: String) {
    appButton(
        "删除记录",
        ButtonVariant.OUTLINE,
        danger = { true },
        icon = IconToken.Delete,
        iconOnly = true,
        accessibilityLabel = "删除该设备记录",
    ) { state.confirmDeleteId = deviceId }
}

/** 删除记录的行内二次确认（header 下方）：确认/取消 + 口径说明（同原删除按钮文案）。 */
private fun ViewContainer<*, *>.echoMuseDeleteConfirm(
    row: EchoMuseDeviceRow,
    state: EchoMuseState,
    toast: ToastState,
    ctx: Pager,
) {
    View {
        attr {
            flexDirectionColumn()
            paddingBottom(AdminSpace.sm)
        }
        actionRow {
            primaryButton(
                "确认删除",
                danger = { true },
                loading = { state.actionBusy },
                loadingText = "删除中…",
                icon = IconToken.Delete,
            ) {
                state.delete(ctx, row.deviceId) { ok, msg ->
                    toast.show(msg, if (ok) "ok" else "error")
                    if (ok) state.refresh(ctx)
                }
            }
            View { attr { width(AdminSpace.sm) } }
            secondaryButton("取消", icon = IconToken.Close) { state.confirmDeleteId = "" }
        }
        textNote(
            "删除只清服务端登记（含批准/唤醒词设置），不影响设备本身；设备下次接入会重新登记。",
            micro = true,
        )
    }
}

// ============================================================
// 卡内菜单（左列）
// ============================================================

/** 卡片内菜单项集合：未批准 = 概览/批准；已批准 = 概览/WiFi 配置/唤醒词设置/设备管理/蓝牙代理。 */
private fun echoMuseMenuItems(row: EchoMuseDeviceRow): List<Triple<String, String, IconToken>> =
    if (row.approved) {
        listOf(
            Triple(ECHOMUSE_MENU_OVERVIEW, "概览", IconToken.Info),
            Triple(ECHOMUSE_MENU_WIFI, "WiFi 配置", IconToken.Wifi),
            Triple(ECHOMUSE_MENU_WAKE, "唤醒词设置", IconToken.Waveform),
            Triple(ECHOMUSE_MENU_MANAGE, "设备管理", IconToken.Settings),
            Triple(ECHOMUSE_MENU_BLE, "蓝牙代理", IconToken.Bluetooth),
        )
    } else {
        listOf(
            Triple(ECHOMUSE_MENU_OVERVIEW, "概览", IconToken.Info),
            Triple(ECHOMUSE_MENU_APPROVE, "批准", IconToken.Check),
        )
    }

/**
 * 卡片内紧凑菜单项（左图标 + 文字，图标随选中态着色）：高 36、radiusSm，
 * 选中 = sidebarSelectedBg 底 + accentTintText 文字，未选中 textSecondary。
 * ⚠️ isSelected 必须是 lambda 且只在 attr 闭包内求值
 * （Kuikly 仅跟踪闭包内的 observable 读取，同 AdminNav.sidebarItem 的教训）。
 */
private fun ViewContainer<*, *>.echoMuseMenuItem(
    icon: IconToken,
    label: String,
    isSelected: () -> Boolean,
    onClick: () -> Unit,
) {
    View {
        attr {
            height(36f)
            flexDirectionRow()
            alignItemsCenter()
            paddingLeft(AdminSpace.sm)
            paddingRight(AdminSpace.sm)
            borderRadius(AdminShape.radiusSm)
            backgroundColor(if (isSelected()) AdminColors.sidebarSelectedBg else AdminColors.transparent)
            // 菜单是点击目标：悬停指针
            cursor(CursorType.POINTER)
        }
        event { click { onClick() } }
        appIcon(
            { icon },
            size = 16f,
            tint = { if (isSelected()) AdminColors.accentTintText else AdminColors.textTertiary },
        )
        Text {
            attr {
                marginLeft(AdminSpace.xs)
                fontSize(AdminType.body)
                color(if (isSelected()) AdminColors.accentTintText else AdminColors.textSecondary)
                text(label)
            }
        }
    }
}

// ============================================================
// 右栏内容面板
// ============================================================

/**
 * 右栏面板分发：按当前生效菜单项渲染（[EchoMuseState.activeMenuItem]）。
 * ⚠️ 必须用 vif 逐项判断而不是 when——observable 只在 vif 闭包内读取才被跟踪，
 * 构建期 when 求值会让菜单切换后右栏不刷新。
 */
private fun ViewContainer<*, *>.echoMusePanel(
    row: EchoMuseDeviceRow,
    state: EchoMuseState,
    toast: ToastState,
    ctx: Pager,
    onOpenModelFiles: () -> Unit = {},
) {
    vif({ state.activeMenuItem(row) == ECHOMUSE_MENU_OVERVIEW }) { echoMuseOverviewPanel(row, state, toast) }
    vif({ state.activeMenuItem(row) == ECHOMUSE_MENU_APPROVE }) { echoMuseApprovePanel(row, state, toast, ctx) }
    vif({ state.activeMenuItem(row) == ECHOMUSE_MENU_WIFI }) { echoMuseWifiPanel(row, state, toast, ctx) }
    vif({ state.activeMenuItem(row) == ECHOMUSE_MENU_WAKE }) {
        echoMuseWakeSettings(row.deviceId, state, toast, ctx, onOpenModelFiles)
    }
    vif({ state.activeMenuItem(row) == ECHOMUSE_MENU_MANAGE }) { echoMuseManagementPanel(row, state, toast, ctx) }
    vif({ state.activeMenuItem(row) == ECHOMUSE_MENU_BLE }) { echoMuseBlePanel(row, state, ctx) }
}

/** 概览面板：设备信息键值行（设备号/设备名/状态/版本板型系统/时间）+「复制信息」。 */
private fun ViewContainer<*, *>.echoMuseOverviewPanel(
    row: EchoMuseDeviceRow,
    state: EchoMuseState,
    toast: ToastState,
) {
    echoMuseInfoRow("设备号", row.deviceId)
    // 未改名时标题就是设备号，「设备名」行不重复渲染
    if (row.renamed) {
        echoMuseInfoRow("设备名", row.label)
    }
    echoMuseInfoRow("状态", if (row.online) "在线" else "离线")
    if (row.version.isNotEmpty()) {
        echoMuseInfoRow("版本", row.version)
    }
    if (row.board.isNotEmpty()) {
        echoMuseInfoRow("板型", row.board)
    }
    if (row.baseOs.isNotEmpty()) {
        echoMuseInfoRow("系统", row.baseOs)
    }
    if (row.metaText.isNotEmpty()) {
        textNote(row.metaText, micro = true)
    }
    // web 端文本已可划选复制（h5App 入口关掉了渲染器 selectstart 拦截）；此按钮仍是
    // macOS/触屏等无划选平台的一键兜底（粘到聊天/工单可直接读）
    actionRow {
        secondaryButton("复制信息", icon = IconToken.Copy) {
            if (copyToClipboard(row.copyText)) {
                toast.show("设备信息已复制到剪贴板", "ok")
            } else {
                toast.show("复制失败：当前平台剪贴板不可用", "error")
            }
        }
    }
}

/** 概览面板的键值行：左键 caption textSecondary（定宽对齐），右值 caption textPrimary。 */
private fun ViewContainer<*, *>.echoMuseInfoRow(label: String, value: String) {
    View {
        attr {
            flexDirectionRow()
            alignItemsCenter()
            paddingTop(AdminSpace.xs)
        }
        Text {
            attr {
                width(72f)
                fontSize(AdminType.caption)
                color(AdminColors.textSecondary)
                text(label)
            }
        }
        Text {
            attr {
                flex(1f)
                fontSize(AdminType.caption)
                color(AdminColors.textPrimary)
                text(value)
            }
        }
    }
}

/** 批准面板：可填 label 后确认批准（POST approve，成功后刷新列表）。 */
private fun ViewContainer<*, *>.echoMuseApprovePanel(
    row: EchoMuseDeviceRow,
    state: EchoMuseState,
    toast: ToastState,
    ctx: Pager,
) {
    labeledField(
        "设备名称（label，可留空）",
        { state.approveDraft },
        { state.approveDraft = it },
        "如 客厅音箱",
    )
    actionRow {
        primaryButton(
            "确认批准",
            loading = { state.actionBusy },
            loadingText = "批准中…",
            icon = IconToken.Check,
        ) {
            state.approve(ctx, row.deviceId) { ok, msg ->
                toast.show(msg, if (ok) "ok" else "error")
                if (ok) state.refresh(ctx)
            }
        }
    }
}

/** WiFi 配置面板：ssid/psk 表单 + 下发（enabled 随在线）；202 只代表已排队，结果看灯环。 */
private fun ViewContainer<*, *>.echoMuseWifiPanel(
    row: EchoMuseDeviceRow,
    state: EchoMuseState,
    toast: ToastState,
    ctx: Pager,
) {
    labeledField("WiFi 名称（SSID）", { state.wifiSsid }, { state.wifiSsid = it }, "路由器名称")
    labeledField("WiFi 密码（PSK）", { state.wifiPsk }, { state.wifiPsk = it }, "密码")
    actionRow {
        primaryButton(
            "下发 WiFi 配置",
            enabled = { row.online },
            loading = { state.actionBusy },
            loadingText = "下发中…",
            icon = IconToken.Wifi,
        ) {
            state.sendWifi(ctx, row.deviceId) { ok, msg ->
                toast.show(msg, if (ok) "ok" else "error")
            }
        }
    }
    vif({ !row.online }) {
        textNote("设备离线：WiFi 配置指令需要设备在线才能接收，唤醒设备后可用。", micro = true)
    }
}

/**
 * 唤醒词设置面板（草稿在 [EchoMuseState]，进入面板时按设备快照重填）。
 *
 * 与 WiFi 不同：本表单**离线也可用**——服务端持久化后设备下次接入自动推送，
 * 202 = 设备在线已即时下发，200 = 已保存待上线。枚举档位下拉（固件协议值）。
 *
 * 信息面按官方 dashboard.jsx（wilbowes/EchoMuse HEAD e2f2f4e）Wake word 卡移植：
 * - 「唤醒词检测位置」三档官方文案，shadow 为诊断档、仅当前已处于该档时显示；
 * - 灵敏度滑杆用官方量程 0.1~0.975、步进 0.025、Precise↔Eager 反向方向，
 *   轨外存储值把手钉在端点、读数保真（[EchoMuseWakeWord.trackProgress]）；
 * - 唤醒提示音附官方语义（听到唤醒词播放短促上升音），档位仅开启时显示。
 *
 * 唤醒词模型下拉（[EchoMuseWakeWord]）：存储为空也显示设备默认 hey jarvis；
 * 选项 = 官方四款 + 服务端 `oww-models/` 已上传的自定义模型 + 已存自定义值
 * （[EchoMuseWakeWord.buildOwwOptions]）。上传 ≠ 安装：设备端 config 对空
 * owwModel 不生效、设备只认本地已装模型，所以保存恒发非空生效模型名，
 * 「管理模型文件」入口经 [onOpenModelFiles] 跳「文件」页上传/清理模型。
 */
private fun ViewContainer<*, *>.echoMuseWakeSettings(
    deviceId: String,
    state: EchoMuseState,
    toast: ToastState,
    ctx: Pager,
    onOpenModelFiles: () -> Unit = {},
) {
    switchRow("响应唤醒词（关闭后按键仍可用）", state.wakeWordEnabled) { state.wakeWordEnabled = !state.wakeWordEnabled }
    dropdownField(
        "唤醒词检测位置",
        { state.owwModes.firstOrNull { it.first == state.owwOnDevice }?.second ?: "未知" },
        { state.owwModes }, { state.owwOnDevice }, { state.owwModeOpen },
        { state.owwModeOpen = !state.owwModeOpen }, { state.owwOnDevice = it },
    )
    Text {
        attr {
            fontSize(AdminType.caption)
            color(AdminColors.textTertiary)
            text(EchoMuseWakeWord.modeNote(state.owwOnDevice))
        }
    }
    dropdownField(
        "唤醒词模型",
        { state.owwModelOptions.firstOrNull { it.first == state.owwModel }?.second ?: state.owwModel },
        { state.owwModelOptions }, { state.owwModel }, { state.owwModelOpen },
        { state.owwModelOpen = !state.owwModelOpen }, { state.owwModel = it },
    )
    Text {
        attr {
            fontSize(AdminType.caption)
            color(AdminColors.textTertiary)
            text("当前唤醒词：「${EchoMuseWakeWord.displayName(state.owwModel)}」（模型 ${state.owwModel}${if (state.owwModelStored.isBlank()) "，设备默认" else ""}）。官方内置共 4 款：hey jarvis、alexa、hey mycroft、hey rhasspy。")
        }
    }
    Text {
        attr {
            fontSize(AdminType.caption)
            color(AdminColors.textTertiary)
            text("保存后服务端把 owwModel 随 config 下发，设备立即换用新模型本地评分。设备只认本地已装的模型（开通时已带全部 4 款官方模型）；自定义模型可先上传到服务端 oww-models/ 供这里选择，但上传 ≠ 安装——设备没装过的模型选了也听不到唤醒词，按键仍可唤醒。")
        }
    }
    // 服务端模型文件入口：上传/清理 oww-models/ 下的自定义模型（跳「文件」页对应目录）
    actionRow {
        secondaryButton("管理模型文件（oww-models/）", icon = IconToken.Folder) { onOpenModelFiles() }
    }

    // 灵敏度滑杆：移植官方 dashboard.jsx 量程/步进/方向——左「Precise（阈值高，
    // 少误触易漏唤）」右「Eager（阈值低，易触发易误触）」，与阈值反向。轨外
    // 存储值（旧面板可写 0.05 或 1.0）把手钉在端点、读数保持存储值，拖动才
    // 写入官方量程内的新值（官方 wakeTrackFor 行为）。
    fieldLabel("唤醒灵敏度（官方量程 0.10～0.975，步进 0.025）")
    View {
        attr {
            flexDirectionRow()
            alignItemsCenter()
            marginTop(AdminSpace.xs)
        }
        Slider {
            attr {
                // ⚠️ 必须是显式宽度，不能用 flex(1f)（理由见 [echoMuseSliderWidth]）
                width(echoMuseSliderWidth(ctx))
                height(28f)
                currentProgress(EchoMuseWakeWord.trackProgress(state.owwThreshold))
                progressColor(AdminColors.accent)
                trackColor(AdminColors.divider)
                thumbColor(AdminColors.accent)
            }
            event {
                progressDidChanged { p ->
                    state.owwThreshold = EchoMuseWakeWord.quantizeTrack(p)
                }
            }
        }
        Text {
            attr {
                marginLeft(AdminSpace.sm)
                width(48f)
                fontSize(AdminType.caption)
                color(AdminColors.textSecondary)
                text((kotlin.math.round(state.owwThreshold * 1000) / 1000.0).toString())
            }
        }
    }
    View {
        attr {
            flexDirectionRow()
            marginTop(AdminSpace.xs)
        }
        Text {
            attr {
                fontSize(AdminType.caption)
                color(AdminColors.textTertiary)
                text("Precise（严格）")
            }
        }
        View { attr { flex(1f) } }
        Text {
            attr {
                fontSize(AdminType.caption)
                color(AdminColors.textTertiary)
                text("Eager（积极）")
            }
        }
    }
    Text {
        attr {
            fontSize(AdminType.caption)
            color(AdminColors.textTertiary)
            marginTop(AdminSpace.xs)
            text("官方语义：唤醒所需置信度——普通对话会误唤醒就向 Precise 调高，很少响应就向 Eager 调低。上限 0.975 而非 1.0：openWakeWord 分数低于 1.0 即饱和、判定为 score≥阈值，1.0 意味着永不唤醒（官方实测 18,021 帧峰值 0.999）。")
        }
    }

    switchRow("唤醒提示音", state.wakeSound) { state.wakeSound = !state.wakeSound }
    Text {
        attr {
            fontSize(AdminType.caption)
            color(AdminColors.textTertiary)
            text("听到唤醒词时播放短促上升提示音（官方 wake sound）。")
        }
    }
    // 官方行为：提示音档位只在开关打开时显示
    vif({ state.wakeSound }) {
        dropdownField(
            "提示音档位",
            { state.wakeLevels.firstOrNull { it.first == state.wakeSoundLevel }?.second ?: "未知" },
            { state.wakeLevels }, { state.wakeSoundLevel }, { state.wakeLevelOpen },
            { state.wakeLevelOpen = !state.wakeLevelOpen }, { state.wakeSoundLevel = it },
        )
    }
    labeledIntField("麦克风编号（0=中心，0~6）", { state.wakeMic }, { state.wakeMic = it }, "0")

    actionRow {
        primaryButton(
            "保存唤醒词设置",
            loading = { state.actionBusy },
            loadingText = "保存中…",
            icon = IconToken.Save,
        ) {
            state.saveSettings(ctx, deviceId) { ok, msg ->
                toast.show(msg, if (ok) "ok" else "error")
            }
        }
    }
    textNote(
        "设备在线时设置即时下发；离线时保存到服务端，设备下次接入自动生效（结果看设备灯环）。",
        micro = true,
    )
}
