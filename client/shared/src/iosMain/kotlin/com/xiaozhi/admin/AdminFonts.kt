package com.xiaozhi.admin

/**
 * iOS（UIKit）：`Menlo` 是系统自带等宽字体，`UIFont(name:size:)` 能直接按族名命中。
 * 族名写错不会崩——`UIFont` 返回 nil 时渲染层回退默认字体，只是失去等宽外观。
 */
actual val ADMIN_MONO_FONT: String = "Menlo"
