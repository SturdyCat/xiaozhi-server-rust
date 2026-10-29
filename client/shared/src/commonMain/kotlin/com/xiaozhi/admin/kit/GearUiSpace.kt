package com.xiaozhi.admin.kit

import com.tencent.kuikly.compose.ui.unit.dp

/**
 * 间距 token（Dp 版）—— 原 `ui/AdminSpace` 的**数值一字不改**，只把 `Float` 换成 `Dp`。
 *
 * Q9 裁决：迁移后**不引入** GearUI 的 spacing 体系，也不新造一套数值，直接把这 8+3 个数搬过来，
 * 保证版式节奏与迁移前逐像素一致（否则 K 系列视觉约束全部失效，截图对比无从谈起）。
 *
 * 原注释（保留）：按 4 的倍数规范化，杜绝魔法数字；
 * `pagePadding` 页面左右留白；`gutter` 双列卡片间距（兼做卡片纵向间距）；
 * `cardPadding` 卡片内边距；`fieldGap` 表单字段间距。
 *
 * ⚠️ 调用点写 `GearUiSpace.md`，不要写裸 `16.dp`。
 */
object GearUiSpace {
    val xs = 8.dp
    val sm = 12.dp
    val md = 16.dp
    val lg = 20.dp
    val xl = 24.dp
    val xxl = 28.dp
    val xxxl = 32.dp

    val pagePadding = 24.dp
    val gutter = 20.dp
    val cardPadding = 20.dp
    val fieldGap = 12.dp
}
