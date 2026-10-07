package com.xiaozhi.admin

import com.tencent.kuikly.core.base.Color
import com.tencent.kuikly.core.views.Text
import com.tencent.kuikly.core.views.TextArea
import com.tencent.kuikly.core.views.View
import com.tencent.kuikly.core.views.compose.Button

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
    val accentTintBg = Color(0x332B5E3FL)
    val accentTintText = Color(0xFF4CD964L)

    val danger = Color(0xFFFF453AL)
    val dangerHover = Color(0xFFE53429L)
    val dangerActive = Color(0xFFD12E24L)
    val dangerTintBg = Color(0x33FF453AL)
    val dangerTintText = Color(0xFFFF6961L)

    /** 禁用/处理中按钮背景：中性灰（iOS 暗色 systemGray2）。比页面/卡片底色明显亮一档，
     *  一眼可辨「不可用」；⛔ 不要用 insetBg/trackBg（#3A3A3C）当禁用底——与暗色主题
     *  背景几乎同色，看起来像「没画按钮」而不是「按钮被禁用」。 */
    val disabledBg = Color(0xFF636366L)

    /** 禁用/处理中按钮文字：浅灰（在中灰底上可读，明显弱于正常态白字） */
    val disabledText = Color(0xFFD1D1D6L)

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
