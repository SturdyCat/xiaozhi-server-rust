package com.xiaozhi.admin.ui

import com.tencent.kuikly.core.base.Color
import com.tencent.kuikly.core.views.Text

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
/**
 * 颜色数值的**唯一真源**（0xAARRGGBB 的 Long 字面量）。
 *
 * 为什么单独抽一张表：管理台现在有两条渲染栈——传统 Kuikly DSL（[AdminColors]，颜色类型是
 * `com.tencent.kuikly.core.base.Color`）与 Compose 磁贴（`com.xiaozhi.admin.compose.AdminC`，
 * 颜色类型是 `com.tencent.kuikly.compose.ui.graphics.Color`）。两侧的 `Color(Long)` 构造都按
 * ARGB 解析，因此同一份 Long 可以直接喂给两边，**不在任何一侧复制 hex 字面量**。
 *
 * ⛔ 新增/调整颜色只改这里；两侧包装自动跟随。
 */
object AdminToken {
    const val WINDOW_BG = 0xFF1C1C1EL
    const val SIDEBAR_BG = 0xFF1C1C1EL
    const val SIDEBAR_SELECTED_BG = 0xFF2C2C2EL
    const val CARD_BG = 0xFF2C2C2EL
    const val CARD_HOVER = 0xFF3A3A3CL
    const val FIELD_BG = 0xFF2C2C2EL
    const val INSET_BG = 0xFF3A3A3CL
    const val TOAST_BG = 0xFF3A3A3CL
    const val TEXT_PRIMARY = 0xFFF5F5F7L
    const val TEXT_SECONDARY = 0xFFAEAEB2L
    const val TEXT_TERTIARY = 0xFF8E8E93L
    const val TEXT_PLACEHOLDER = 0xFF6E6E73L
    const val TEXT_ON_ACCENT = 0xFFFFFFFFL
    const val TEXT_ACCENT = 0xFF4CD964L
    const val DIVIDER = 0xFF38383AL
    const val DIVIDER_STRONG = 0xFF48484AL
    const val ACCENT = 0xFF07C160L
    const val ACCENT_HOVER = 0xFF06B257L
    const val ACCENT_ACTIVE = 0xFF059A4CL
    const val ACCENT_TINT_BG = 0x332B5E3FL
    const val ACCENT_TINT_TEXT = 0xFF4CD964L
    const val DANGER = 0xFFFF453AL
    const val DANGER_HOVER = 0xFFE53429L
    const val DANGER_ACTIVE = 0xFFD12E24L
    const val DANGER_TINT_BG = 0x33FF453AL
    const val DANGER_TINT_TEXT = 0xFFFF6961L
    const val DISABLED_BG = 0xFF636366L
    const val DISABLED_TEXT = 0xFFD1D1D6L
    const val WARNING = 0xFFFF9F0AL
    const val WARNING_TINT_BG = 0x33FF9F0AL
    const val WARNING_TINT_TEXT = 0xFFFFD479L
    const val TRACK_BG = 0xFF3A3A3CL
    const val SWITCH_OFF = 0xFF3A3A3CL
    const val KNOB = 0xFFFFFFFFL
    const val SHADOW_SM = 0x40000000L
    const val FOCUS_RING = 0x3307C160L
    const val OVERLAY = 0x66000000L
    const val TRANSPARENT = 0x00000000L

    // ---- 终端（控制台 tab）----
    // 取官方 dashboard 的 Shell 主题（背景 #1c1f18 / 前景 #c8d4b0 / 光标 #9aba80），
    // 但饱和度按管理台的近黑底微调：终端块要能一眼认出「这是另一个世界」，
    // 同时又不能亮到抢走卡片层级。⛔ 终端内部不再引用其它 token，避免两套语义打架。
    /** 终端底色（官方 `background: '#1c1f18'`）：偏绿的近黑，和 cardBg 的纯灰区分开。 */
    const val TERM_BG = 0xFF1C1F18L
    /** 终端正文（官方 `foreground: '#c8d4b0'`）：带绿调的浅色，长读不刺眼。 */
    const val TERM_FG = 0xFFC8D4B0L
    /** 终端弱化文字（连接提示、提示符后缀、空态）：同色系降一档。 */
    const val TERM_DIM = 0xFF6E7A5EL
    /** 终端强调色（提示符 `❯`、光标块，官方 `cursor: '#9aba80'`）。 */
    const val TERM_ACCENT = 0xFF9ABA80L
}

object AdminColors {
    val windowBg = Color(AdminToken.WINDOW_BG)
    val sidebarBg = Color(AdminToken.SIDEBAR_BG)
    /** 侧边栏选中项背景：cardBg 同级的亮灰胶囊（在近黑侧栏上清晰可辨，macOS HIG 风格）。 */
    val sidebarSelectedBg = Color(AdminToken.SIDEBAR_SELECTED_BG)
    val cardBg = Color(AdminToken.CARD_BG)
    val cardHover = Color(AdminToken.CARD_HOVER)
    val fieldBg = Color(AdminToken.FIELD_BG)
    val insetBg = Color(AdminToken.INSET_BG)

    /** 顶部浮动 toast 背景：比卡片再亮一档的浮层色，保证覆盖在内容之上时层次清晰。 */
    val toastBg = Color(AdminToken.TOAST_BG)

    val textPrimary = Color(AdminToken.TEXT_PRIMARY)
    val textSecondary = Color(AdminToken.TEXT_SECONDARY)
    val textTertiary = Color(AdminToken.TEXT_TERTIARY)
    val textPlaceholder = Color(AdminToken.TEXT_PLACEHOLDER)
    val textOnAccent = Color(AdminToken.TEXT_ON_ACCENT)
    val textAccent = Color(AdminToken.TEXT_ACCENT)

    val divider = Color(AdminToken.DIVIDER)
    val dividerStrong = Color(AdminToken.DIVIDER_STRONG)

    val accent = Color(AdminToken.ACCENT)
    val accentHover = Color(AdminToken.ACCENT_HOVER)
    val accentActive = Color(AdminToken.ACCENT_ACTIVE)
    val accentTintBg = Color(AdminToken.ACCENT_TINT_BG)
    val accentTintText = Color(AdminToken.ACCENT_TINT_TEXT)

    val danger = Color(AdminToken.DANGER)
    val dangerHover = Color(AdminToken.DANGER_HOVER)
    val dangerActive = Color(AdminToken.DANGER_ACTIVE)
    val dangerTintBg = Color(AdminToken.DANGER_TINT_BG)
    val dangerTintText = Color(AdminToken.DANGER_TINT_TEXT)

    /** 禁用/处理中按钮背景：中性灰（iOS 暗色 systemGray2）。比页面/卡片底色明显亮一档，
     *  一眼可辨「不可用」；⛔ 不要用 insetBg/trackBg（#3A3A3C）当禁用底——与暗色主题
     *  背景几乎同色，看起来像「没画按钮」而不是「按钮被禁用」。 */
    val disabledBg = Color(AdminToken.DISABLED_BG)

    /** 禁用/处理中按钮文字：浅灰（在中灰底上可读，明显弱于正常态白字） */
    val disabledText = Color(AdminToken.DISABLED_TEXT)

    val warning = Color(AdminToken.WARNING)
    val warningTintBg = Color(AdminToken.WARNING_TINT_BG)
    val warningTintText = Color(AdminToken.WARNING_TINT_TEXT)

    val trackBg = Color(AdminToken.TRACK_BG)
    val switchOff = Color(AdminToken.SWITCH_OFF)
    val knob = Color(AdminToken.KNOB)

    val shadowSm = Color(AdminToken.SHADOW_SM)
    val focusRing = Color(AdminToken.FOCUS_RING)

    /** 抽屉遮罩（P1-4 窄屏侧边栏）：40% 黑，盖在内容上让抽屉浮起。 */
    val overlay = Color(AdminToken.OVERLAY)

    /** 完全透明（分段控件未选中段 / 容器化 Input 背景用） */
    val transparent = Color(AdminToken.TRANSPARENT)

    /** 终端底色（控制台 tab 的整块底色）。 */
    val termBg = Color(AdminToken.TERM_BG)

    /** 终端正文（等宽输出、用户命令回显）。 */
    val termFg = Color(AdminToken.TERM_FG)

    /** 终端弱化文字（连接提示、空态、命令历史提示）。 */
    val termDim = Color(AdminToken.TERM_DIM)

    /** 终端强调色（提示符、发送按钮文字）。 */
    val termAccent = Color(AdminToken.TERM_ACCENT)
}

object AdminShape {
    val radiusSm = 8f
    val radiusMd = 10f
    val radiusLg = 12f
    val radiusPill = 9999f
}

/**
 * 字号 token（pt，按设备动态缩放）。
 *
 * 基准：body=14（默认字号），caption/bodySm/micro 一律不低于 14（最小 14 号），
 * 标题类按层级放大：section=20 / title=24 / display=32。
 *
 * 设备适配：[scale] 由 AdminPage 创建时按 `pagerData.pageViewWidth` 计算一次
 * （视口越宽系数越大，1.0~1.5 之间，窄屏/手机保持 1.0 不缩小），
 * 各 token 用 getter 在取值时乘上系数，调用点仍写 `fontSize(AdminType.body)` 不变。
 * 注意：系数在页面创建时确定，窗口中途 resize 不重排字号（Kuikly attr 不响应）。
 */
object AdminType {
    /** 设备缩放系数，由 AdminPage 初始化（默认 1.0）。 */
    var scale = 1f

    val display get() = 32f * scale
    val title get() = 24f * scale
    val section get() = 20f * scale
    val body get() = 14f * scale
    val bodySm get() = 14f * scale
    val caption get() = 14f * scale
    val micro get() = 14f * scale

    /** 按页面视口宽度计算缩放系数：基准 1200pt 宽为 1.0，每 +600pt 加 0.2，封顶 1.5。 */
    fun scaleFor(pageViewWidth: Float): Float =
        (1f + ((pageViewWidth - 1200f).coerceAtLeast(0f) / 600f) * 0.2f).coerceAtMost(1.5f)
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
