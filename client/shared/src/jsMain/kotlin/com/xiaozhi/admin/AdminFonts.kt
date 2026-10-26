package com.xiaozhi.admin

/**
 * Web：`monospace` 是 CSS **通用族名**，浏览器必定命中（各平台解析成该系统自带的等宽字体，
 * 如 macOS 的 Menlo / Windows 的 Consolas）。写具体族名反而会在没装该字体的机器上回退。
 */
actual val ADMIN_MONO_FONT: String = "monospace"
