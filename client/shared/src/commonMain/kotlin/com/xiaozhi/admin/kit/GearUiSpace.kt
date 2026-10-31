package com.xiaozhi.admin.kit

import com.tencent.kuikly.compose.ui.unit.dp

/**
 * 间距 token 的**历史入口**（已降级为 [LocalAdminSpacing] 的兼容别名）。
 *
 * Q9 裁决：迁移后不引入 GearUI spacing 体系，也不新造数值，直接把这套数搬过来，
 * 保证版式节奏与迁移前逐像素一致。本对象保留**原常量值**（scale=1 时即旧值），作为
 * 向后兼容别名；scale=1 下与迁移前逐像素一致，守 Q9 像素守恒。
 *
 * 需要随视口 / fontScale 同源缩放的新代码，请改用 `LocalAdminSpacing.current.xxx`
 * （由 [provideAdminSpacing] 注入）。本别名不再参与缩放，仅用于尚未迁移的旧调用点。
 *
 * ⚠️ 调用点写 `GearUiSpace.md`，等价于旧的 `16.dp`；新代码请直接用 `LocalAdminSpacing.current.md`。
 */
object GearUiSpace {
    @Deprecated("改用 LocalAdminSpacing.current.xs", ReplaceWith("LocalAdminSpacing.current.xs"))
    val xs = 8.dp

    @Deprecated("改用 LocalAdminSpacing.current.sm", ReplaceWith("LocalAdminSpacing.current.sm"))
    val sm = 12.dp

    @Deprecated("改用 LocalAdminSpacing.current.md", ReplaceWith("LocalAdminSpacing.current.md"))
    val md = 16.dp

    @Deprecated("改用 LocalAdminSpacing.current.lg", ReplaceWith("LocalAdminSpacing.current.lg"))
    val lg = 20.dp

    @Deprecated("改用 LocalAdminSpacing.current.xl", ReplaceWith("LocalAdminSpacing.current.xl"))
    val xl = 24.dp

    @Deprecated("改用 LocalAdminSpacing.current.xxl", ReplaceWith("LocalAdminSpacing.current.xxl"))
    val xxl = 28.dp

    @Deprecated("改用 LocalAdminSpacing.current.xxxl", ReplaceWith("LocalAdminSpacing.current.xxxl"))
    val xxxl = 32.dp

    @Deprecated("改用 LocalAdminSpacing.current.pagePadding", ReplaceWith("LocalAdminSpacing.current.pagePadding"))
    val pagePadding = 24.dp

    @Deprecated("改用 LocalAdminSpacing.current.gutter", ReplaceWith("LocalAdminSpacing.current.gutter"))
    val gutter = 20.dp

    @Deprecated("改用 LocalAdminSpacing.current.cardPadding", ReplaceWith("LocalAdminSpacing.current.cardPadding"))
    val cardPadding = 20.dp

    @Deprecated("改用 LocalAdminSpacing.current.fieldGap", ReplaceWith("LocalAdminSpacing.current.fieldGap"))
    val fieldGap = 12.dp
}
