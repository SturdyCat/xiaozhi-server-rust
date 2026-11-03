package com.xiaozhi.admin

/**
 * 密钥打码显示口径（纯函数，零 Kuikly 依赖，由 `:adminCoreTests` 直接回归）。
 *
 * 管理台所有证书/密钥输入框（`labeledSecretField`）在「隐藏」态显示打码版本：
 * **头尾各保留 3 字符，中间用 `*` 占位**（`*` 个数与被遮住的原字符数一致，长度不泄露也不失真）。
 *
 * 边界：
 * - 空串原样返回（空字段没有可打码的内容，显示占位提示）；
 * - 长度 ≤ 6 时整体打码——头尾各 3 字符会重叠甚至裸露全部内容，短密钥一个字符都不显示；
 * - 不做任何字符替换/裁剪，输入输出一一对应，同一输入永远得到同一打码串。
 */
fun maskSecret(value: String, keep: Int = 3): String {
    if (value.isEmpty()) return ""
    if (value.length <= keep * 2) return "*".repeat(value.length)
    return value.take(keep) + "*".repeat(value.length - keep * 2) + value.takeLast(keep)
}
