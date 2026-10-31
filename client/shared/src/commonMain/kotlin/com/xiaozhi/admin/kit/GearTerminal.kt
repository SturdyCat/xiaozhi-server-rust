package com.xiaozhi.admin.kit
import com.xiaozhi.admin.kit.LocalAdminSpacing

import androidx.compose.runtime.Composable
import com.gearui.foundation.primitives.BasicTextField
import com.tencent.kuikly.compose.foundation.background
import com.tencent.kuikly.compose.foundation.clickable
import com.tencent.kuikly.compose.foundation.layout.Column
import com.tencent.kuikly.compose.foundation.layout.ColumnScope
import com.tencent.kuikly.compose.foundation.layout.fillMaxWidth
import com.tencent.kuikly.compose.foundation.layout.padding
import com.tencent.kuikly.compose.foundation.shape.RoundedCornerShape
import com.tencent.kuikly.compose.foundation.text.BasicText
import com.tencent.kuikly.compose.ui.Modifier
import com.tencent.kuikly.compose.ui.draw.clip
import com.tencent.kuikly.compose.ui.graphics.Color
import com.tencent.kuikly.compose.ui.graphics.SolidColor
import com.tencent.kuikly.compose.ui.text.TextStyle
import com.tencent.kuikly.compose.ui.text.font.FontFamily
import com.tencent.kuikly.compose.ui.unit.dp
import com.tencent.kuikly.compose.ui.unit.sp

/**
 * 深色终端块（K2 保留项）—— 控制台页与 EchoMuse 控制台 tab 共用。
 *
 * ## K2：这套颜色**不许主题化**
 *
 * 网页版控制台是深色终端（官方 `terminal` 区域 `#1c1f18` 底 + `#c8d4b0` 绿调前景），
 * 用户在浅色管理台里点进控制台时**期望看到深色终端**——把正文反色成浅色会直接违背这条视觉约束。
 * 因此这里用**私有常量**而不是 `Theme.colors`：色值一字不改，也不导出成公共 token
 * （E10 的 grep 判据按符号名，不按数值，故不构成"自研 token 复活"）。
 *
 * 旧实现从 `AdminToken.TERM_*` 取色（`0xFF1C1F18L` 等）；本文件把同样的数值搬成 [GearTerminalPalette]，
 * 数值逐位一致。
 */
object GearTerminalPalette {
    /** 终端底色（整块容器背景） */
    val bg: Color = Color(0xFF1C1F18)

    /** 终端正文（等宽输出、用户命令回显） */
    val fg: Color = Color(0xFFC8D4B0)

    /** 弱化文字（连接提示、提示符后缀、空态） */
    val dim: Color = Color(0xFF6E7A5E)

    /** 强调色（提示符 `❯`、光标块） */
    val accent: Color = Color(0xFF9ABA80)
}

/** 终端文字档位 —— 只做语义命名，色值见 [GearTerminalPalette]。 */
enum class GearTerminalTone {
    /** 正文 */
    Fg,

    /** 弱化（提示、空态） */
    Dim,

    /** 强调（提示符、命令回显） */
    Accent,
}

/**
 * 终端容器：深色底 + 圆角 + 内边距，纵向排列。
 *
 * @param content 终端内容（通常是若干 [GearTerminalText]）
 */
@Composable
fun GearTerminal(
    modifier: Modifier = Modifier,
    content: @Composable ColumnScope.() -> Unit,
) {
    Column(
        modifier = modifier
            .fillMaxWidth()
            .clip(RoundedCornerShape(10.dp))
            .background(GearTerminalPalette.bg)
            .padding(horizontal = LocalAdminSpacing.current.sm, vertical = LocalAdminSpacing.current.xs),
        content = content,
    )
}

/**
 * 终端正文/提示文字：等宽字体 + 深色档位配色。
 *
 * 用 Kuikly Compose 的 `BasicText` 而不是 GearUI `Text` —— 后者的字号/字族只能来自
 * `Theme.typography` 的 token，而终端必须是**等宽**且**固定 13sp**（不随主题排版缩放，
 * 否则 ANSI 表格与列对齐会错位）。
 */
@Composable
fun GearTerminalText(
    text: String,
    modifier: Modifier = Modifier,
    tone: GearTerminalTone = GearTerminalTone.Fg,
    /** 行内用法（提示符、顶栏标题）传 `false`：不占满整行，跟随内容宽度。 */
    fill: Boolean = true,
) {
    val color = when (tone) {
        GearTerminalTone.Fg -> GearTerminalPalette.fg
        GearTerminalTone.Dim -> GearTerminalPalette.dim
        GearTerminalTone.Accent -> GearTerminalPalette.accent
    }
    BasicText(
        text = text,
        modifier = if (fill) modifier.fillMaxWidth() else modifier,
        style = TextStyle(
            color = color,
            fontSize = 13.sp,
            fontFamily = FontFamily.Monospace,
        ),
    )
}

/**
 * 终端里的**文字动作键**（中断 / 清屏 / 重连 / 执行 / 跟随）。
 *
 * 终端里没有按钮：官方 dashboard 的 shell 只有命令与提示符，任何带边框的控件都会把
 * 「一块终端」重新切成「终端 + 工具条」。所以这里是等宽文字 + 无边框点击区，
 * 禁用态退到 [GearTerminalPalette.dim]，与提示符后缀同一档。
 *
 * [onClick] 放在参数表**最后**：调用方可以用尾随 lambda
 * （`GearTerminalKey("中断", enabled = true) { … }`）。
 *
 * @param enabled 置灰即不可点（`clickable(enabled = false)` 同时摘掉点击与语义动作）
 */
@Composable
fun GearTerminalKey(
    label: String,
    enabled: Boolean,
    modifier: Modifier = Modifier,
    onClick: () -> Unit,
) {
    BasicText(
        text = label,
        modifier = modifier.clickable(enabled = enabled) { onClick() },
        style = TextStyle(
            color = if (enabled) GearTerminalPalette.accent else GearTerminalPalette.dim,
            fontSize = 12.sp,
            fontFamily = FontFamily.Monospace,
        ),
    )
}

/**
 * 终端的**无壳输入行**：无边框、无背景、等宽、光标用强调色。
 *
 * 用 GearUI 的 `BasicTextField`（「不携带边框/背景/内边距」的裸输入原语）而不是 `Input`：
 * 后者会画出主题色的字段框，在深色终端里是一块浅色补丁（K2 的直接违背）。
 * 只覆盖文字色/字族/光标色 —— 这三样都在本文件内取 [GearTerminalPalette]，页面侧不出现色值。
 *
 * @param onSubmit 回车提交（IME Done）；调用方仍须给可见的「执行」键兜底（H5 的
 *                `Input.inputReturn` 官方标注「实现中」，只留回车等于 Web 端没有提交入口）
 */
@Composable
fun GearTerminalInput(
    value: String,
    onValueChange: (String) -> Unit,
    modifier: Modifier = Modifier,
    placeholder: String = "",
    enabled: Boolean = true,
    onSubmit: (() -> Unit)? = null,
) {
    BasicTextField(
        value = value,
        onValueChange = onValueChange,
        modifier = modifier,
        placeholder = placeholder,
        enabled = enabled,
        singleLine = true,
        onImeDone = onSubmit,
        cursorBrush = SolidColor(GearTerminalPalette.accent),
        textStyle = TextStyle(
            color = GearTerminalPalette.fg,
            fontSize = 13.sp,
            fontFamily = FontFamily.Monospace,
        ),
    )
}
