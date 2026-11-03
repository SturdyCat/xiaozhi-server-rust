package com.xiaozhi.admin.kit
import com.xiaozhi.admin.kit.LocalAdminSpacing

import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import com.gearui.components.input.Input
import com.gearui.components.switch.Switch
import com.gearui.components.textarea.Textarea
import com.gearui.foundation.field.FieldVariant
import com.gearui.foundation.primitives.Text
import com.gearui.theme.Theme
import com.tencent.kuikly.compose.foundation.layout.Arrangement
import com.tencent.kuikly.compose.foundation.layout.Column
import com.tencent.kuikly.compose.foundation.layout.Row
import com.tencent.kuikly.compose.foundation.layout.Spacer
import com.tencent.kuikly.compose.foundation.layout.fillMaxWidth
import com.tencent.kuikly.compose.foundation.layout.heightIn
import com.tencent.kuikly.compose.foundation.layout.padding
import com.tencent.kuikly.compose.ui.Alignment
import com.tencent.kuikly.compose.ui.Modifier
import com.tencent.kuikly.compose.ui.semantics.contentDescription
import com.tencent.kuikly.compose.ui.semantics.semantics
import com.tencent.kuikly.compose.ui.text.input.KeyboardType
import com.tencent.kuikly.compose.ui.unit.dp
import com.xiaozhi.admin.contracts.IconToken
import com.xiaozhi.admin.icons.GearIcon
import com.xiaozhi.admin.maskSecret

/**
 * 表单字段族（Compose 版）—— 旧 `ui/Form.kt` 的等价物，基座换成 GearUI 的
 * `Input` / `Textarea` / `Switch`。
 *
 * ## 为什么整数/小数字段仍然用 `Input` 而不是 GearUI 的 `NumberField`
 *
 * 方案 §4.1-8 把「整数 → `NumberField`、小数 → `DecimalNumberField`」列为映射方向，但那条
 * 映射的**前提是状态层用数值类型**。本项目 411 个字段里，端口/线程/阈值这类配置在状态层是
 * **`String`**，且保存路径按 `toIntOrNull() ?: 默认值` 回落（见 [INT_INPUT_FILTER] 的注释：
 * 脏输入会被**静默**改写成默认值，比报错更隐蔽）。
 *
 * 换成 `NumberField(value: Double?)` 必须同时改状态层字段类型与方法签名 —— 这违反 T05 的硬约束
 * 「字段名、类型、默认值、方法签名零改动」。因此这里保留 `Input` + **原样的两个过滤器**，
 * 语义与迁移前**完全一致**：键入即时剥离非法字符，状态里永远是干净的字符串。
 * 数字键盘由 `keyboardType` 保证（T05 的字段核对清单要求逐字段确认）。
 *
 * ## `INT_INPUT_FILTER` / `DECIMAL_INPUT_FILTER` 是业务校验，不是样式
 *
 * 二者从 `ui/Form.kt` **逐字搬来**，实现未改一个字符。
 */

/**
 * 整数输入过滤：只保留 ASCII 数字。整数字段（端口/线程数/条数等）禁止小数点、
 * 负号、字母——否则保存路径 `toIntOrNull() ?: 默认值` 会把脏输入**静默回落成默认值**
 *（比如把 "80." 存成 8000），比报错更隐蔽。
 */
internal val INT_INPUT_FILTER: (String) -> String = { raw -> raw.filter { it.isDigit() } }

/** 小数输入过滤：数字 + 至多一个小数点（temperature/speed/threshold 等浮点字段防脏输入）。 */
internal val DECIMAL_INPUT_FILTER: (String) -> String = { raw ->
    buildString {
        var dotSeen = false
        for (ch in raw) {
            when {
                ch.isDigit() -> append(ch)
                ch == '.' && !dotSeen -> {
                    dotSeen = true
                    append(ch)
                }
            }
        }
    }
}

/**
 * 标签行：左「中文名 + 可选 ⓘ」，右可选控制动作（如「获取模型列表」）。
 *
 * 与旧 `fieldLabel` 同一节奏：上留 [LocalAdminSpacing.current.fieldGap]、下留 [LocalAdminSpacing.current.xs]。
 */
@Composable
fun fieldLabelRow(
    label: String,
    help: String = "",
    trailing: (@Composable () -> Unit)? = null,
) {
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .padding(top = LocalAdminSpacing.current.fieldGap, bottom = LocalAdminSpacing.current.xs),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(
            text = label,
            style = Theme.typography.label,
            color = Theme.colors.foreground,
        )
        if (help.isNotEmpty()) GearInfoBadge(title = label, help = help)
        Spacer(Modifier.weight(1f))
        if (trailing != null) trailing()
    }
}

/** 字段级生效语义提示（"需重启 server 生效"之类），只有非空时才占位。 */
@Composable
private fun fieldNote(note: String) {
    if (note.isEmpty()) return
    Text(
        text = note,
        modifier = Modifier.padding(top = 4.dp),
        style = Theme.typography.caption,
        color = Theme.colors.warning,
    )
}

/**
 * 标签 + 单行输入框。
 *
 * @param value         当前值（状态层字段）
 * @param onValueChange 写回状态；[inputFilter] 在比较**之前**执行（剥离后与现值相同就不回调，
 *                      不打无谓的 dirty 标 —— 与旧实现同口径）
 * @param note          字段级提示（如"需重启 server 生效"），空串不占位
 * @param help          字段详解正文；非空时标签右侧挂 ⓘ
 * @param trailing      标签行的右侧控制动作
 * @param inputFilter   数字过滤：[INT_INPUT_FILTER] / [DECIMAL_INPUT_FILTER]，null = 不过滤
 * @param readOnly      只读展示（值仍完整显示，仅不可编辑；[labeledSecretField] 的打码态用）
 * @param suffix        输入框内右端插槽（如 [labeledSecretField] 的眼睛按钮），空 = 不占位
 * @param onSubmit      回车提交（控制台命令行这类必须能提交的输入，调用方仍须给可见按钮兜底）
 */
@Composable
fun labeledField(
    label: String,
    value: String,
    onValueChange: (String) -> Unit,
    modifier: Modifier = Modifier,
    placeholder: String = "",
    note: String = "",
    help: String = "",
    trailing: (@Composable () -> Unit)? = null,
    enabled: Boolean = true,
    readOnly: Boolean = false,
    onSubmit: (() -> Unit)? = null,
    inputFilter: ((String) -> String)? = null,
    keyboardType: KeyboardType = KeyboardType.Text,
    suffix: (@Composable () -> Unit)? = null,
) {
    Column(modifier.fillMaxWidth()) {
        fieldLabelRow(label, help, trailing)
        Input(
            value = value,
            onValueChange = { raw ->
                val next = inputFilter?.invoke(raw) ?: raw
                if (next != value) onValueChange(next)
            },
            modifier = Modifier.fillMaxWidth(),
            placeholder = placeholder,
            enabled = enabled,
            readOnly = readOnly,
            // 管理台字段一律在卡片上 → SECONDARY 变体（GearUI 的口径：卡片/弹层内用 SECONDARY）
            variant = FieldVariant.SECONDARY,
            keyboardType = keyboardType,
            onSend = onSubmit,
            suffix = suffix,
        )
        fieldNote(note)
    }
}

/**
 * 标签 + **整数**输入框：[labeledField] + [INT_INPUT_FILTER]。
 * 整型配置字段（端口/线程/条数）必须用它，不要用裸 [labeledField]。
 */
@Composable
fun labeledIntField(
    label: String,
    value: String,
    onValueChange: (String) -> Unit,
    modifier: Modifier = Modifier,
    placeholder: String = "",
    note: String = "",
    help: String = "",
    trailing: (@Composable () -> Unit)? = null,
    enabled: Boolean = true,
) = labeledField(
    label = label,
    value = value,
    onValueChange = onValueChange,
    modifier = modifier,
    placeholder = placeholder,
    note = note,
    help = help,
    trailing = trailing,
    enabled = enabled,
    inputFilter = INT_INPUT_FILTER,
    keyboardType = KeyboardType.Number,
)

/**
 * 标签 + **小数**输入框：[labeledField] + [DECIMAL_INPUT_FILTER]——
 * 放行数字与至多一个小数点（temperature/speed/threshold 等浮点字段）。
 */
@Composable
fun labeledDecimalField(
    label: String,
    value: String,
    onValueChange: (String) -> Unit,
    modifier: Modifier = Modifier,
    placeholder: String = "",
    note: String = "",
    help: String = "",
    trailing: (@Composable () -> Unit)? = null,
    enabled: Boolean = true,
) = labeledField(
    label = label,
    value = value,
    onValueChange = onValueChange,
    modifier = modifier,
    placeholder = placeholder,
    note = note,
    help = help,
    trailing = trailing,
    enabled = enabled,
    inputFilter = DECIMAL_INPUT_FILTER,
    keyboardType = KeyboardType.Decimal,
)

/**
 * 标签 + **密钥**输入框（眼睛查看）——证书/密钥类配置字段的统一零件。
 *
 * 与 [labeledField] 的差异：
 * - **隐藏态（默认）显示打码版本**：头尾各保留 3 字符、中间 `*` 占位（口径见
 *   [com.xiaozhi.admin.maskSecret]）；空字段没有可打码内容，照常显示 [placeholder] 提示。
 *   打码串是只读视图（`readOnly`），此态到达的键盘写入一律丢弃，`*` 不会混进配置草稿。
 * - **点击输入框右端的眼睛切换明文**：显示真实值并进入可编辑状态，再点一次恢复打码；
 *   两个状态用 `Eye` / `EyeOff` 图标区分，并带中文 contentDescription。
 * - **空字段可直接键入**：第一个字符到达时自动切到明文编辑态，之后的输入/粘贴都是真实值
 *   ——不必先点眼睛才开始填新密钥。
 *
 * 刻意**不走 `KeyboardType.Password`**：跨端原生密码通道会把显示整体变成圆点（Kuikly iOS
 * 走 secureTextEntry，视觉转换不过桥，见 GearUI `Input` 内注释），呈现不了"头尾 3 字符"；
 * 打码由字符串层完成，行为四端一致。
 */
@Composable
fun labeledSecretField(
    label: String,
    value: String,
    onValueChange: (String) -> Unit,
    modifier: Modifier = Modifier,
    placeholder: String = "",
    note: String = "",
    help: String = "",
    trailing: (@Composable () -> Unit)? = null,
    enabled: Boolean = true,
) {
    // 眼睛开合是纯展示态（AGENTS 口径：纯展示态放 remember，不进状态层）
    var revealed by remember { mutableStateOf(false) }
    labeledField(
        label = label,
        value = if (revealed) value else maskSecret(value),
        onValueChange = { raw ->
            when {
                revealed -> onValueChange(raw)
                // 空字段首键入（含粘贴）：直接以真实值进入明文编辑，后续输入不再经过打码态
                value.isEmpty() && raw.isNotEmpty() -> {
                    revealed = true
                    onValueChange(raw)
                }
                // 其余打码态写入全部丢弃：打码展示是只读视图，* 不能进草稿
            }
        },
        modifier = modifier,
        placeholder = placeholder,
        note = note,
        help = help,
        trailing = trailing,
        enabled = enabled,
        readOnly = !revealed && value.isNotEmpty(),
        suffix = {
            GearIcon(
                token = if (revealed) IconToken.EyeOff else IconToken.Eye,
                size = 16.dp,
                tint = Theme.colors.mutedForeground,
                modifier = Modifier.semantics {
                    contentDescription = if (revealed) "隐藏密钥" else "显示密钥"
                },
                onClick = if (enabled) {
                    { revealed = !revealed }
                } else null,
            )
        },
    )
}

/**
 * 标签 + 多行输入框：散文 / 提示词 / 系统提示等长文本。用 GearUI `Textarea`（`autosize`）。
 *
 * 旧实现给的是固定 100pt 高的容器；这里 `autosize = true` + `minLines = 4`，长文本自然长高，
 * 语义更强且不依赖魔法高度。
 */
@Composable
fun labeledTextArea(
    label: String,
    value: String,
    onValueChange: (String) -> Unit,
    modifier: Modifier = Modifier,
    placeholder: String = "",
    note: String = "",
    help: String = "",
    trailing: (@Composable () -> Unit)? = null,
    enabled: Boolean = true,
    minLines: Int = 4,
) {
    Column(modifier.fillMaxWidth()) {
        fieldLabelRow(label, help, trailing)
        Textarea(
            value = value,
            onValueChange = onValueChange,
            modifier = Modifier.fillMaxWidth(),
            enabled = enabled,
            variant = FieldVariant.SECONDARY,
            placeholder = placeholder,
            minLines = minLines,
            autosize = true,
            additionInfo = note.ifEmpty { null },
        )
    }
}

/**
 * 开关行：左「中文名 + 可选 ⓘ（+ 一行说明）」，右开关，左右两端对齐。
 *
 * `checked` 是当前值，`onToggle` 写回状态 —— 旧实现是「checked 仅初始值、组件内部自治」，
 * Compose 版改成受控（状态唯一真源），这是 Compose 的正确语义，也与 `stateVar` 改造一致。
 */
@Composable
fun switchRow(
    label: String,
    checked: Boolean,
    onToggle: (Boolean) -> Unit,
    modifier: Modifier = Modifier,
    help: String = "",
    description: String = "",
    enabled: Boolean = true,
) {
    Row(
        modifier = modifier
            .fillMaxWidth()
            .heightIn(min = 52.dp)
            .padding(top = LocalAdminSpacing.current.fieldGap),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Column(Modifier.weight(1f)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text(
                    text = label,
                    style = Theme.typography.label,
                    color = Theme.colors.foreground,
                )
                if (help.isNotEmpty()) GearInfoBadge(title = label, help = help)
            }
            if (description.isNotEmpty()) {
                Text(
                    text = description,
                    style = Theme.typography.caption,
                    color = Theme.colors.mutedForeground,
                )
            }
        }
        Switch(
            checked = checked,
            onCheckedChange = onToggle,
            enabled = enabled,
            contentDescription = label,
        )
    }
}

/**
 * 卡片内动作行：按钮/徽标靠右且垂直居中，顶部留 [LocalAdminSpacing.current.sm]。
 *
 * ⚠️ 与旧实现的差异（登记为 A 类版式差异）：旧版是 `flexWrap`，窄屏会换行；
 * Compose 的 `Row` 不换行。管理台的 actionRow 通常只有 1–3 个按钮，实际不会溢出；
 * T07 起若出现溢出，改用 `FlowRow` 或改成两行布局，届时按页面单独处理。
 */
@Composable
fun actionRow(
    modifier: Modifier = Modifier,
    content: @Composable () -> Unit,
) {
    Row(
        modifier = modifier
            .fillMaxWidth()
            .padding(top = LocalAdminSpacing.current.sm),
        horizontalArrangement = Arrangement.spacedBy(LocalAdminSpacing.current.xs, Alignment.End),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        content()
    }
}
