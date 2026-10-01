package com.xiaozhi.admin

import com.tencent.kuikly.core.base.Border
import com.tencent.kuikly.core.base.BorderStyle
import com.tencent.kuikly.core.base.Color
import com.tencent.kuikly.core.base.ViewBuilder
import com.tencent.kuikly.core.base.ViewContainer
import com.tencent.kuikly.core.directives.velse
import com.tencent.kuikly.core.directives.vif
import com.tencent.kuikly.core.views.ActivityIndicator
import com.tencent.kuikly.core.views.Input
import com.tencent.kuikly.core.views.Scroller
import com.tencent.kuikly.core.views.Text
import com.tencent.kuikly.core.views.View

/**
 * 小智管理后台 · 设计 Token 与可复用组件（纯 UI，无平台依赖）。
 *
 * 视觉走现代原生风（macOS/iPad HIG）。所有颜色统一 AdminColors.*（8 位 hex），
 * 字号 AdminType.*、间距 AdminSpace.*、圆角 AdminShape.*，组件全部封装为
 * `ViewContainer<*,*>.xxx(...)` 文件级扩展供各 section 复用。
 *
 * ⚠️ 传统 Kuikly DSL 约束：
 * - 叶子组件 Text/Input 不支持 padding：Text 用 margin；Input 由 labeledField 外层容器承担
 *   背景/边框/内边距（iOS 渲染层 KRTextFieldView 无 padding prop，文字会贴边）；
 * - 组件一律写成 ViewContainer 扩展，在目标容器闭包内**非限定**调用（经 ctx.xxx() 调用会
 *   把节点挂到 Pager 根容器，布局逃逸）。
 */

/**
 * 暗色系设计 Token。窗口/侧栏近黑底，卡片/输入组略亮「浮起」；
 * 品牌绿 accent 保留 #07C160，暗底「选中/链接」文字用更亮 #4CD964 保证对比度。
 */
object AdminColors {
    val windowBg = Color(0xFF1C1C1EL)
    val sidebarBg = Color(0xFF1C1C1EL)
    /** 侧边栏选中项背景：cardBg 同级的亮灰胶囊（在近黑侧栏上清晰可辨，macOS HIG 风格）。 */
    val sidebarSelectedBg = Color(0xFF2C2C2EL)
    val cardBg = Color(0xFF2C2C2EL)
    val cardHover = Color(0xFF3A3A3CL)
    val fieldBg = Color(0xFF2C2C2EL)
    val insetBg = Color(0xFF3A3A3CL)

    val textPrimary = Color(0xFFF5F5F7L)
    val textSecondary = Color(0xFFAEAEB2L)
    val textTertiary = Color(0xFF8E8E93L)
    val textPlaceholder = Color(0xFF6E6E73L)
    val textOnAccent = Color(0xFFFFFFFFL)
    val textAccent = Color(0xFF4CD964L)

    val divider = Color(0xFF38383AL)
    val dividerStrong = Color(0xFF48484AL)

    val accent = Color(0xFF07C160L)
    val accentHover = Color(0xFF06B257L)
    val accentActive = Color(0xFF059A4CL)
    val accentDisable = Color(0xFF3A5A47L)
    val accentTintBg = Color(0x332B5E3FL)
    val accentTintText = Color(0xFF4CD964L)

    val danger = Color(0xFFFF453AL)
    val dangerHover = Color(0xFFE53429L)
    val dangerActive = Color(0xFFD12E24L)
    /** 禁用态红（录音按钮禁用/加载时用；比 danger 暗、去饱和，与 accentDisable 同风格） */
    val dangerDisable = Color(0xFF5A3230L)
    val dangerTintBg = Color(0x33FF453AL)
    val dangerTintText = Color(0xFFFF6961L)

    val warning = Color(0xFFFF9F0AL)
    val warningTintBg = Color(0x33FF9F0AL)
    val warningTintText = Color(0xFFFFD479L)

    val trackBg = Color(0xFF3A3A3CL)
    val switchOff = Color(0xFF3A3A3CL)
    val knob = Color(0xFFFFFFFFL)

    val shadowSm = Color(0x40000000L)
    val focusRing = Color(0x3307C160L)

    /** 完全透明（分段控件未选中段 / 容器化 Input 背景用） */
    val transparent = Color(0x00000000L)
}

object AdminShape {
    val radiusSm = 8f
    val radiusMd = 10f
    val radiusLg = 12f
    val radiusPill = 9999f
}

/**
 * 字号 token（pt）。已按「远看清晰 / iPad 触屏操作」放大两轮：
 * display=34 顶栏大标题；section=21 卡片标题；body=17 正文/标签/输入/按钮；
 * bodySm=16 分段控件；caption=15 提示；micro=13 徽标/版本。
 */
object AdminType {
    val display = 34f
    val title = 22f
    val section = 21f
    val body = 17f
    val bodySm = 16f
    val caption = 15f
    val micro = 13f
}

/**
 * 间距 token（pt），按 4 的倍数规范化，杜绝魔法数字：
 * xs=8 / sm=12 / md=16 / lg=20 / xl=24；
 * pagePadding 页面左右留白；gutter 双列卡片间距（兼做卡片纵向间距）；
 * cardPadding 卡片内边距；fieldGap 表单字段间距。
 */
object AdminSpace {
    val xs = 8f
    val sm = 12f
    val md = 16f
    val lg = 20f
    val xl = 24f
    val xxl = 28f
    val xxxl = 32f
    val pagePadding = 24f
    val gutter = 20f
    val cardPadding = 20f
    val fieldGap = 12f
}

// ===================== 基础排版 =====================

/** 分组标题：section(21)/500，textPrimary。 */
fun ViewContainer<*, *>.sectionTitle(title: String) {
    Text {
        attr {
            fontSize(AdminType.section)
            fontWeightMedium()
            color(AdminColors.textPrimary)
            marginBottom(AdminSpace.sm)
            text(title)
        }
    }
}

/** 卡片内 1px 横向分隔线（行间分隔）。 */
fun ViewContainer<*, *>.dividerH() {
    View {
        attr {
            height(1f)
            backgroundColor(AdminColors.divider)
            marginBottom(AdminSpace.sm)
        }
    }
}

// ===================== 卡片与双列网格 =====================

/**
 * 卡片描述：标题 + 内容。供 cardGrid 统一排版（单列 / 宽屏双列），内容延迟渲染。
 */
class AdminCard(val title: String, val withDivider: Boolean = true, val content: ViewBuilder)

/**
 * 分组卡片：cardBg、radiusLg(12)、1px divider 边框、内边距 cardPadding(20)。
 * 不带左右外边距——页面留白由 Scroller 的 pagePadding 提供，双列间距由 cardGrid 控制，
 * 避免嵌套容器时边距叠加（嵌套卡片在内层卡片里也正好贴 padding）。
 */
fun ViewContainer<*, *>.groupedCard(title: String, withDivider: Boolean = true, content: ViewBuilder) {
    View {
        attr {
            flexDirectionColumn()
            backgroundColor(AdminColors.cardBg)
            borderRadius(AdminShape.radiusLg)
            border(Border(1f, BorderStyle.SOLID, AdminColors.divider))
            padding(AdminSpace.cardPadding)
            marginBottom(AdminSpace.gutter)
        }
        sectionTitle(title)
        if (withDivider) dividerH()
        content()
    }
}

/**
 * 卡片行原语：一行最多两列、等宽（各 flex(1)），列间距 gutter；right 为 null 时左卡独占整行。
 * left/right 为 ViewBuilder，在对应列容器闭包内调用（接收者=列容器，节点正确挂载）。
 * cardGrid 与各 section 的手工双列（homeSection/renderBench）都基于它。
 */
fun ViewContainer<*, *>.cardRow(left: ViewBuilder, right: ViewBuilder? = null) {
    View {
        attr { flexDirectionRow() }
        View {
            attr { flex(1f) }
            left()
        }
        if (right != null) {
            View { attr { width(AdminSpace.gutter) } }
            View {
                attr { flex(1f) }
                right()
            }
        }
    }
}

/**
 * 卡片网格（响应式封装）：wide() 为 true 时卡片两两一行、等宽双列（cardRow）；
 * 否则单列纵向排。卡片数量奇数时最后一行独占整行。
 *
 * ⚠️ wide 必须是 lambda 且只在 vif 闭包内读取（如 `{ pagerData.pageViewWidth >= 900f }`）：
 * Kuikly 只对 attr/vif 闭包内的 observable 读取做响应式跟踪，构建期一次性求值的 Boolean
 * 在窗口 resize 后不会重算（这就是此前「缩小窗口双列不变单列」的根因）。
 * pageViewWidth 是响应式字段（见 kuikly-ui-framework 技能文档 pager-lifecycle.md），
 * 窗口尺寸变化会触发 vif 分支重建，实现双列 ⇄ 单列切换。
 */
fun ViewContainer<*, *>.cardGrid(cards: List<AdminCard>, wide: () -> Boolean) {
    vif({ wide() }) {
        // vif 闭包内仅一个普通根节点（列容器），内部循环铺 cardRow
        View {
            attr { flexDirectionColumn() }
            var index = 0
            while (index < cards.size) {
                val left = cards[index]
                val right = cards.getOrNull(index + 1)
                cardRow(
                    { groupedCard(left.title, left.withDivider, left.content) },
                    right?.let { card -> { groupedCard(card.title, card.withDivider, card.content) } },
                )
                index += 2
            }
        }
    }
    velse {
        View {
            attr { flexDirectionColumn() }
            cards.forEach { card -> groupedCard(card.title, card.withDivider, card.content) }
        }
    }
}

// ===================== 输入 =====================

/** 字段标签（只读）：顶部 fieldGap 与上一字段拉开间距。 */
fun ViewContainer<*, *>.fieldLabel(label: String) {
    Text {
        attr {
            fontSize(AdminType.body)
            fontWeightMedium()
            color(AdminColors.textSecondary)
            marginTop(AdminSpace.fieldGap)
            marginBottom(AdminSpace.xs)
            text(label)
        }
    }
}

/**
 * 标签 + 输入框：默认高 48（触屏友好）。
 * ⚠️ Input 内边距方案：iOS 渲染层 Input 无 padding 属性、文字贴边，
 * 故由外层容器承担 fieldBg 背景 / 1px 边框 / radiusSm / 左右内边距 sm(12)，
 * Input 透明背景 flex(1) 填满容器。
 * ⚠️ Input 必须显式传 textDidChange 回调，否则输入不更新。
 */
fun ViewContainer<*, *>.labeledField(
    label: String,
    getValue: () -> String,
    onChange: (String) -> Unit,
    placeholder: String = "",
    height: Float = 48f,
) {
    fieldLabel(label)
    View {
        attr {
            flexDirectionRow()
            alignItemsCenter()
            height(height)
            backgroundColor(AdminColors.fieldBg)
            border(Border(1f, BorderStyle.SOLID, AdminColors.divider))
            borderRadius(AdminShape.radiusSm)
            paddingLeft(AdminSpace.sm)
            paddingRight(AdminSpace.sm)
        }
        Input {
            attr {
                flex(1f)
                height(height)
                backgroundColor(AdminColors.transparent)
                fontSize(AdminType.body)
                color(AdminColors.textPrimary)
                text(getValue())
                placeholder(placeholder)
            }
            event { textDidChange { params -> onChange(params.text) } }
        }
    }
}

// ===================== 按钮 =====================

/**
 * 主按钮：高 48（触屏）、paddingH 20、minWidth 96、radiusSm、allCenter；
 * bg accent（禁用 accentDisable + 文字 0xFFF4FCF8）；文字 textOnAccent body(17)/500。
 * - danger=true：录音等待止类按钮用红色系（danger/dangerDisable）。
 * - loading=true：按钮内显示菊花（白色 ActivityIndicator）+ 文案，配合 enabled=false
 *   表达「处理中」并阻止重复点击（文案由调用方经 vif/velse 分支切换）。
 */
fun ViewContainer<*, *>.primaryButton(
    text: String,
    enabled: Boolean = true,
    danger: Boolean = false,
    loading: Boolean = false,
    onClick: () -> Unit,
) {
    val bg = when {
        danger && (!enabled && !loading) -> AdminColors.dangerDisable
        danger -> AdminColors.danger
        !enabled && !loading -> AdminColors.accentDisable
        else -> AdminColors.accent
    }
    View {
        attr {
            height(48f)
            minWidth(96f)
            paddingLeft(AdminSpace.lg)
            paddingRight(AdminSpace.lg)
            borderRadius(AdminShape.radiusSm)
            flexDirectionRow()
            allCenter()
            backgroundColor(bg)
        }
        event { click { if (enabled && !loading) onClick() } }
        if (loading) {
            // 白色菊花（暗色/彩色底上可见）：isGrayStyle(false) → "white"
            ActivityIndicator {
                attr {
                    isGrayStyle(false)
                    marginRight(AdminSpace.xs)
                }
            }
        }
        Text {
            attr {
                fontSize(AdminType.body)
                fontWeightMedium()
                color(if (enabled || loading) AdminColors.textOnAccent else Color(0xFFF4FCF8L))
                text(text)
            }
        }
    }
}

/** 次按钮：高 44、radiusSm、bg insetBg、文字 textPrimary body(17)/500。支持禁用/加载态。 */
fun ViewContainer<*, *>.secondaryButton(
    text: String,
    enabled: Boolean = true,
    loading: Boolean = false,
    onClick: () -> Unit,
) {
    View {
        attr {
            height(44f)
            paddingLeft(AdminSpace.lg)
            paddingRight(AdminSpace.lg)
            borderRadius(AdminShape.radiusSm)
            flexDirectionRow()
            allCenter()
            backgroundColor(if (enabled && !loading) AdminColors.insetBg else AdminColors.trackBg)
        }
        event { click { if (enabled && !loading) onClick() } }
        if (loading) {
            ActivityIndicator {
                attr {
                    isGrayStyle(true)
                    marginRight(AdminSpace.xs)
                }
            }
        }
        Text {
            attr {
                fontSize(AdminType.body)
                fontWeightMedium()
                color(if (enabled && !loading) AdminColors.textPrimary else AdminColors.textTertiary)
                text(text)
            }
        }
    }
}

/** 卡片内动作行：横向排按钮/徽标，顶部留 sm(12) 间距；元素间距由调用处 `View { width(12f) }` 控制。 */
fun ViewContainer<*, *>.actionRow(content: ViewBuilder) {
    View {
        attr {
            flexDirectionRow()
            alignItemsCenter()
            marginTop(AdminSpace.sm)
        }
        content()
    }
}

// ===================== 分段控件 =====================

/**
 * 分段控件：轨道 trackBg、radiusPill、内距 2、高 38（段高 34，触屏友好）；
 * 普通段文字 bodySm(16)/500 textSecondary，选中段 bg cardBg + 文字 textPrimary。
 */
fun ViewContainer<*, *>.segmentedControl(options: List<String>, selectedIndex: Int, onSelect: (Int) -> Unit) {
    View {
        attr {
            flexDirectionRow()
            height(38f)
            backgroundColor(AdminColors.trackBg)
            borderRadius(AdminShape.radiusPill)
            padding(2f)
        }
        options.forEachIndexed { index, label ->
            val selected = index == selectedIndex
            View {
                attr {
                    flex(1f)
                    height(34f)
                    borderRadius(AdminShape.radiusSm)
                    allCenter()
                    backgroundColor(if (selected) AdminColors.cardBg else AdminColors.transparent)
                }
                event { click { onSelect(index) } }
                Text {
                    attr {
                        fontSize(AdminType.bodySm)
                        fontWeightMedium()
                        color(if (selected) AdminColors.textPrimary else AdminColors.textSecondary)
                        text(label)
                    }
                }
            }
        }
    }
}

// ===================== 开关行 =====================

/**
 * 开关行：高 52（触屏），左标签 body(17)/500 textPrimary，右 Switch。
 * 轨道 46×26 radiusPill(13)，开=accent/关=switchOff；旋钮 22 radiusPill(11)，
 * marginLeft 2/22、垂直居中。整行点击经开关轨道手势切换。
 */
fun ViewContainer<*, *>.switchRow(label: String, checked: Boolean, onToggle: () -> Unit) {
    View {
        attr {
            flexDirectionRow()
            alignItemsCenter()
            height(52f)
            marginTop(AdminSpace.fieldGap)
        }
        Text {
            attr {
                flex(1f)
                fontSize(AdminType.body)
                fontWeightMedium()
                color(AdminColors.textPrimary)
                text(label)
            }
        }
        View {
            attr {
                width(46f)
                height(26f)
                borderRadius(13f)
                backgroundColor(if (checked) AdminColors.accent else AdminColors.switchOff)
            }
            event { click { onToggle() } }
            View {
                attr {
                    width(22f)
                    height(22f)
                    borderRadius(11f)
                    backgroundColor(AdminColors.knob)
                    marginLeft(if (checked) 22f else 2f)
                    marginTop(2f)
                }
            }
        }
    }
}

// ===================== 下拉选择 =====================

/**
 * 下拉选择字段：label + 选择框（显示当前项 + ▾/▴）+ 展开面板（独立 Scroller，高 320，可滚动）。
 *
 * 展开状态由调用方持有（isOpen lambda 读取 observable）——放在状态类里而不是组件内部，
 * 避免 body 重建把展开态重置（Kuikly 的 Pager body 会随 observable 变化重跑）。
 * 选项列表用普通 while 循环铺（vfor 只接受 ObservableList；固定列表用循环更直接，
 * 与 cardGrid 的写法一致）。选中项高亮为 accentTintText + 左侧 3px 指示条。
 */
fun ViewContainer<*, *>.dropdownField(
    label: String,
    currentLabel: () -> String,
    options: List<Pair<String, String>>,
    selectedId: () -> String,
    isOpen: () -> Boolean,
    onToggle: () -> Unit,
    onSelect: (String) -> Unit,
) {
    fieldLabel(label)
    View {
        attr {
            flexDirectionRow()
            alignItemsCenter()
            height(48f)
            backgroundColor(AdminColors.fieldBg)
            border(Border(1f, BorderStyle.SOLID, AdminColors.divider))
            borderRadius(AdminShape.radiusSm)
            paddingLeft(AdminSpace.sm)
            paddingRight(AdminSpace.sm)
        }
        event { click { onToggle() } }
        Text {
            attr {
                flex(1f)
                fontSize(AdminType.body)
                color(AdminColors.textPrimary)
                text(currentLabel())
            }
        }
        Text {
            attr {
                fontSize(AdminType.caption)
                color(AdminColors.textSecondary)
                text(if (isOpen()) "▴" else "▾")
            }
        }
    }
    vif({ isOpen() }) {
        Scroller {
            attr {
                height(320f)
                backgroundColor(AdminColors.cardBg)
                border(Border(1f, BorderStyle.SOLID, AdminColors.divider))
                borderRadius(AdminShape.radiusSm)
                marginTop(AdminSpace.xs)
            }
            var i = 0
            while (i < options.size) {
                val opt = options[i]
                val selected = opt.first == selectedId()
                View {
                    attr {
                        height(44f)
                        flexDirectionRow()
                        alignItemsCenter()
                        paddingLeft(AdminSpace.sm)
                        paddingRight(AdminSpace.md)
                    }
                    event { click { onSelect(opt.first) } }
                    if (selected) {
                        View {
                            attr {
                                width(3f)
                                height(20f)
                                borderRadius(AdminShape.radiusPill)
                                backgroundColor(AdminColors.accent)
                                marginRight(AdminSpace.xs)
                            }
                        }
                    }
                    Text {
                        attr {
                            flex(1f)
                            fontSize(AdminType.body)
                            color(if (selected) AdminColors.accentTintText else AdminColors.textPrimary)
                            text(opt.second)
                        }
                    }
                }
                if (i < options.size - 1) {
                    View {
                        attr {
                            height(1f)
                            marginLeft(AdminSpace.md)
                            backgroundColor(AdminColors.divider)
                        }
                    }
                }
                i++
            }
        }
    }
}

// ===================== 音频波形播放器 =====================

/**
 * 波形播放控件（录音试听 / TTS 回放共用）：
 * [▶/‖ 圆钮] [32 根波形柱（已播 accent 高亮，未播 trackBg）] [时间 0:03/0:12]
 *
 * - "流过"视觉：进度推进时柱子逐根点亮（attr 闭包内读 progress observable，局部更新颜色）。
 * - wave() 返回 64 桶峰值 0~1（原生 getAudioState 采样），渲染时按 32 柱两两取大。
 * - 整块可点（含波形区）切换播放/停止；enabled=false（合成中/无内容）置灰并屏蔽点击。
 * - 进度/波形由调用方轮询原生后写入 observable（TestBenchState），本组件只做纯渲染。
 */
fun ViewContainer<*, *>.waveformPlayer(
    wave: () -> List<Float>,
    progress: () -> Float,
    playing: () -> Boolean,
    durationText: () -> String,
    enabled: Boolean = true,
    onToggle: () -> Unit,
) {
    View {
        attr {
            flexDirectionRow()
            alignItemsCenter()
            marginTop(AdminSpace.fieldGap)
        }
        event { click { if (enabled) onToggle() } }
        // 播放/停止圆钮
        View {
            attr {
                width(44f)
                height(44f)
                borderRadius(AdminShape.radiusPill)
                allCenter()
                backgroundColor(if (enabled) AdminColors.accent else AdminColors.accentDisable)
            }
            Text {
                attr {
                    fontSize(AdminType.title)
                    color(AdminColors.textOnAccent)
                    text(if (playing()) "‖" else "▶")
                }
            }
        }
        // 波形柱（32 根，等宽；已播放点亮 accent）
        View {
            attr {
                flex(1f)
                height(48f)
                flexDirectionRow()
                alignItemsCenter()
                marginLeft(AdminSpace.sm)
            }
            var i = 0
            while (i < WAVE_BARS) {
                // 64 桶两两取大映射到 32 根柱
                val v = maxOf(wave().getOrElse(i * 2) { 0.1f }, wave().getOrElse(i * 2 + 1) { 0.1f })
                val played = progress() * WAVE_BARS > i
                View {
                    attr {
                        flex(1f)
                        height(6f + v * 36f)
                        marginRight(2f)
                        borderRadius(2f)
                        backgroundColor(if (played) AdminColors.accent else AdminColors.trackBg)
                    }
                }
                i++
            }
        }
        Text {
            attr {
                fontSize(AdminType.micro)
                color(AdminColors.textTertiary)
                marginLeft(AdminSpace.xs)
                text(durationText())
            }
        }
    }
}

/** 波形柱数量（getAudioState 返回 64 桶，渲染 32 根柱：两桶取一）。 */
private const val WAVE_BARS = 32

// ===================== 状态徽标 =====================

/**
 * 状态徽标：胶囊高 26、paddingH 10、圆点 8 + 文字 micro(13)/600。
 * state: connected/recording/busy/idle/error 对应 accent/danger/warning/中性/danger 色。
 */
fun ViewContainer<*, *>.statusBadge(state: String, text: String) {
    val dotColor: Color
    val bgColor: Color
    val fgColor: Color
    when (state) {
        "connected" -> {
            dotColor = AdminColors.accent
            bgColor = AdminColors.accentTintBg
            fgColor = AdminColors.accentTintText
        }
        "recording", "error" -> {
            dotColor = AdminColors.danger
            bgColor = AdminColors.dangerTintBg
            fgColor = AdminColors.dangerTintText
        }
        "busy" -> {
            dotColor = AdminColors.warning
            bgColor = AdminColors.warningTintBg
            fgColor = AdminColors.warningTintText
        }
        else -> {
            dotColor = AdminColors.textPlaceholder
            bgColor = AdminColors.insetBg
            fgColor = AdminColors.textTertiary
        }
    }
    View {
        attr {
            flexDirectionRow()
            alignItemsCenter()
            height(26f)
            borderRadius(AdminShape.radiusPill)
            backgroundColor(bgColor)
            paddingLeft(10f)
            paddingRight(10f)
        }
        View {
            attr {
                width(8f)
                height(8f)
                borderRadius(AdminShape.radiusPill)
                backgroundColor(dotColor)
                marginRight(6f)
            }
        }
        Text {
            attr {
                fontSize(AdminType.micro)
                fontWeightMedium()
                color(fgColor)
                text(text)
            }
        }
    }
}

// ===================== 顶栏 =====================

/**
 * 大标题栏：高 64、flexRow 居中、左右内距 xl(24)、bg windowBg、底部 1px divider。
 * 左 largeTitle(display=34)/500 textPrimary；右可选按钮/徽标（trailing ViewBuilder）。
 * ⚠️ title 是 lambda，须在 Text 的 attr 闭包内读取（如 `{ "配置" }` 或依赖 observable 的
 * `{ when (ctx.selectedSection) ... }`），切 section 时标题才会响应式更新。
 * ⚠️ 顶部避让区（macOS 红黄绿窗口按钮）由 AdminShell 的内容区/侧边栏 paddingTop 控制。
 */
fun ViewContainer<*, *>.largeTitleBar(title: () -> String, trailing: ViewBuilder? = null) {
    View {
        attr {
            flexDirectionRow()
            alignItemsCenter()
            height(64f)
            paddingLeft(AdminSpace.xl)
            paddingRight(AdminSpace.xl)
            backgroundColor(AdminColors.windowBg)
        }
        Text {
            attr {
                flex(1f)
                fontSize(AdminType.display)
                fontWeightMedium()
                color(AdminColors.textPrimary)
                text(title())
            }
        }
        if (trailing != null) {
            trailing()
        }
    }
    View {
        attr {
            height(1f)
            backgroundColor(AdminColors.divider)
        }
    }
}
