package com.xiaozhi.admin.compose

import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import com.alorma.compose.settings.ui.SettingsCheckbox
import com.alorma.compose.settings.ui.SettingsMenuLink
import com.alorma.compose.settings.ui.SettingsRadioButton
import com.alorma.compose.settings.ui.SettingsSegmented
import com.alorma.compose.settings.ui.SettingsSwitch
import com.alorma.compose.settings.ui.SettingsTileScaffold
import com.tencent.kuikly.compose.foundation.background
import com.tencent.kuikly.compose.foundation.clickable
import com.tencent.kuikly.compose.foundation.layout.Box
import com.tencent.kuikly.compose.foundation.layout.Column
import com.tencent.kuikly.compose.foundation.layout.Row
import com.tencent.kuikly.compose.foundation.layout.fillMaxWidth
import com.tencent.kuikly.compose.foundation.layout.height
import com.tencent.kuikly.compose.foundation.layout.padding
import com.tencent.kuikly.compose.foundation.text.BasicTextField
import com.tencent.kuikly.compose.material3.Text
import com.tencent.kuikly.compose.ui.Alignment
import com.tencent.kuikly.compose.ui.Modifier
import com.tencent.kuikly.compose.ui.draw.clip
import com.tencent.kuikly.compose.ui.graphics.SolidColor
import com.tencent.kuikly.compose.ui.text.TextStyle
import com.tencent.kuikly.compose.ui.unit.dp
import com.tencent.kuikly.compose.ui.unit.sp
import com.xiaozhi.admin.ui.AdminSpace
import com.xiaozhi.admin.ui.AdminType

/**
 * 管理台的 **Settings 磁贴控件层**：把 Compose-Settings 的原子组件收敛成与旧 DSL 控件
 * 一一对应的调用面，供迁移页面使用。
 *
 * ## 为什么要有这一层
 * 旧管理台的表单控件只有四类语义，散在三个文件里：
 * `switchRow`（开关）、`checkBoxRow`（多选）、`segmentedChoice` / `dropdownField`（单选）。
 * 直接在各页面写 `SettingsXxx(colors = ..., switchColors = ..., shape = ...)` 会让每处调用
 * 都重复四五个主题参数（Kuikly 不能注入 colorScheme，只能逐组件传），一旦 token 调整就要
 * 全量扫一遍。本层把主题参数固定下来，页面只写业务语义。
 *
 * ## 与旧控件的能力对照
 * | 旧 DSL | 本层 | 上游能力 |
 * | --- | --- | --- |
 * | `switchRow` | [AdminSwitchTile] | `SettingsSwitch` |
 * | `checkBoxRow` | [AdminCheckTile] | `SettingsCheckbox` |
 * | `segmentedChoice` | [AdminSegmentedTile] | `SettingsSegmented` |
 * | `dropdownField` | [AdminDropdownTile] | **无**：`SettingsMenuLink` + 自绘选项面板 |
 * | `radioChoice` 类单选 | [AdminRadioTile] | `SettingsRadioButton` |
 * | `infoRow` | [AdminInfoTile] | `SettingsTileScaffold` |
 *
 * ⚠️ 上游 Compose-Settings 只有 Checkbox / RadioButton / Segmented / Slider / Switch / MenuLink，
 * **没有下拉选择**、也没有文本输入。下拉用 MenuLink 作磁贴骨架 + 展开面板补齐（见
 * [AdminDropdownTile]）；文本输入待迁到「服务 / 音频」这类数字段时再补，不要为了凑数
 * 先造一个没人用的组件。
 */

/** 下拉/分段选项的统一形状：`id to 展示文案`。与 DSL 侧 `List<Pair<String, String>>` 同构。 */
internal typealias TileOption = Pair<String, String>

/**
 * 开关磁贴：对应旧 `switchRow`。
 *
 * `subtitle` 承接旧控件的 `help`——旧实现把长说明塞进 ⓘ 弹层，磁贴版式下说明直接作为
 * supporting text 常驻，信息不再需要二次点击才能看到（用户明确反对把关键信息降级）。
 */
@Composable
fun AdminSwitchTile(
    title: String,
    state: Boolean,
    onCheckedChange: (Boolean) -> Unit,
    subtitle: String? = null,
    enabled: Boolean = true,
) {
    SettingsSwitch(
        state = state,
        title = { Text(text = title) },
        subtitle = subtitle?.let { text -> { Text(text = text) } },
        enabled = enabled,
        colors = adminListItemColors(),
        switchColors = adminSwitchColors(),
        shape = adminTileShape(),
        onCheckedChange = onCheckedChange,
    )
}

/** 多选磁贴（单个勾选项）：对应旧 `checkBoxRow`。 */
@Composable
fun AdminCheckTile(
    title: String,
    state: Boolean,
    onCheckedChange: (Boolean) -> Unit,
    subtitle: String? = null,
    enabled: Boolean = true,
) {
    SettingsCheckbox(
        state = state,
        title = { Text(text = title) },
        subtitle = subtitle?.let { text -> { Text(text = text) } },
        enabled = enabled,
        colors = adminListItemColors(),
        checkboxColors = adminCheckboxColors(),
        shape = adminTileShape(),
        onCheckedChange = onCheckedChange,
    )
}

/** 单选磁贴（一组 [AdminRadioTile] 由调用方用 [AdminChoiceGroup] 包起来）。 */
@Composable
fun AdminRadioTile(
    title: String,
    state: Boolean,
    onClick: () -> Unit,
    subtitle: String? = null,
    enabled: Boolean = true,
) {
    SettingsRadioButton(
        state = state,
        title = { Text(text = title) },
        subtitle = subtitle?.let { text -> { Text(text = text) } },
        enabled = enabled,
        colors = adminListItemColors(),
        checkboxColors = adminRadioButtonColors(),
        shape = adminTileShape(),
        onClick = onClick,
    )
}

/**
 * 单选/多选组：一条组标题 + 若干磁贴。
 *
 * 旧 DSL 的 `checkBoxRow` 组与 `segmentedChoice` 都有"组标签"，磁贴版式下用一行
 * group 标题承接，避免一串勾选行失去归属。
 */
@Composable
fun AdminChoiceGroup(title: String, subtitle: String? = null, content: @Composable () -> Unit) {
    Column(modifier = Modifier.fillMaxWidth().padding(top = AdminSpace.md.dp)) {
        Text(
            text = title,
            color = AdminC.textPrimary,
            fontSize = AdminType.body.sp,
        )
        if (subtitle != null) {
            Text(
                text = subtitle,
                color = AdminC.textSecondary,
                fontSize = AdminType.caption.sp,
                modifier = Modifier.padding(top = 2.dp),
            )
        }
        Column(modifier = Modifier.fillMaxWidth().padding(top = AdminSpace.sm.dp)) { content() }
    }
}

/**
 * 分段单选磁贴：对应旧 `segmentedChoice`。
 *
 * 选项数超过 4 个时不要用它——Kuikly 的分段按钮行不换行，会在窄屏把文字压成省略号，
 * 这种场景改用 [AdminDropdownTile]。
 */
@Composable
fun AdminSegmentedTile(
    title: String,
    options: List<TileOption>,
    selectedId: String,
    onSelect: (String) -> Unit,
    subtitle: String? = null,
    enabled: Boolean = true,
) {
    val ids = remember(options) { options.map { it.first } }
    SettingsSegmented(
        title = { Text(text = title) },
        subtitle = subtitle?.let { text -> { Text(text = text) } },
        items = ids,
        selectedItem = selectedId.takeIf { it in ids },
        onItemSelected = onSelect,
        itemTitleMap = { id -> options.firstOrNull { it.first == id }?.second ?: id },
        enabled = enabled,
        colors = adminListItemColors(),
        buttonColors = adminSegmentedButtonColors(),
        shape = adminTileShape(),
    )
}

/**
 * 下拉单选磁贴：对应旧 `dropdownField`。
 *
 * 上游 Compose-Settings 没有下拉组件，这里用 `SettingsMenuLink` 作磁贴骨架
 * （标题 + 当前值 + 箭头），点开后在**同一列**展开选项面板——不做浮层，因为
 * Kuikly Compose 没有 `Popup`/`DropdownMenu`，浮层要自己算坐标与层级，回归风险高；
 * 内联展开在管理台这种长表单里也更稳（不会盖住下面的字段）。
 *
 * 选项面板的每一行是普通可点行（不用 `SettingsRadioButton`）：磁贴套磁贴会出现双层
 * 卡片底色，与旧 `dropdownField` 展开后的观感不一致。
 */
@Composable
fun AdminDropdownTile(
    title: String,
    options: List<TileOption>,
    selectedId: String,
    onSelect: (String) -> Unit,
    subtitle: String? = null,
    placeholder: String = "未选择",
    enabled: Boolean = true,
) {
    var open by remember { mutableStateOf(false) }
    val current = options.firstOrNull { it.first == selectedId }?.second
        ?: selectedId.takeIf { it.isNotEmpty() }
        ?: placeholder

    Column(modifier = Modifier.fillMaxWidth()) {
        SettingsMenuLink(
            title = { Text(text = title) },
            subtitle = subtitle?.let { text -> { Text(text = text) } },
            action = {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Text(
                        text = current,
                        color = if (open) AdminC.textAccent else AdminC.textSecondary,
                        fontSize = AdminType.caption.sp,
                    )
                    Text(
                        text = if (open) " ▴" else " ▾",
                        color = AdminC.textTertiary,
                        fontSize = AdminType.caption.sp,
                    )
                }
            },
            enabled = enabled,
            colors = adminListItemColors(),
            shape = adminTileShape(),
            onClick = { open = !open },
        )
        if (open) {
            Column(
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(top = 4.dp)
                    .clip(adminTileShape())
                    .background(AdminC.insetBg),
            ) {
                for (option in options) {
                    val selected = option.first == selectedId
                    Row(
                        modifier = Modifier
                            .fillMaxWidth()
                            .height(DropdownOptionHeight)
                            .clickable {
                                onSelect(option.first)
                                open = false
                            }
                            .padding(horizontal = AdminSpace.md.dp),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        Text(
                            text = option.second,
                            color = if (selected) AdminC.textAccent else AdminC.textPrimary,
                            fontSize = AdminType.body.sp,
                            modifier = Modifier.weight(1f),
                        )
                        if (selected) {
                            Text(
                                text = "✓",
                                color = AdminC.accent,
                                fontSize = AdminType.body.sp,
                            )
                        }
                    }
                }
            }
        }
    }
}

/** 只读信息磁贴：对应旧 `infoRow`（不可点，仅展示键值）。 */
@Composable
fun AdminInfoTile(title: String, value: String, subtitle: String? = null) {
    SettingsTileScaffold(
        title = { Text(text = title) },
        subtitle = subtitle?.let { text -> { Text(text = text) } },
        colors = adminListItemColors(),
        shape = adminTileShape(),
        action = {
            Box(modifier = Modifier.padding(start = AdminSpace.sm.dp)) {
                Text(text = value, color = AdminC.textSecondary, fontSize = AdminType.caption.sp)
            }
        },
    )
}

/**
 * 文本/数字输入磁贴：对应旧 `labeledField` / `labeledIntField`（上游 Compose-Settings
 * 没有输入类组件，这里用磁贴骨架 + 内嵌 `BasicTextField` 补齐）。
 *
 * 输入框放在**磁贴内部**（作为 supporting 内容），而不是另起一张卡：磁贴与输入框分离时，
 * 长表单里很难一眼看出输入框归属哪个字段（实测首版「接入令牌」的输入框看起来更像下一项的）。
 *
 * ⚠️ 不用 material3 `TextField`：它自带容器底色 + 下划线 + 聚焦态描边，嵌进磁贴会出现
 * 两层边框；`BasicTextField` 只画光标，底色与圆角由这里统一给。
 */
@Composable
fun AdminTextTile(
    title: String,
    value: String,
    onValueChange: (String) -> Unit,
    subtitle: String? = null,
    placeholder: String = "",
    singleLine: Boolean = true,
    enabled: Boolean = true,
) {
    SettingsTileScaffold(
        title = { Text(text = title) },
        subtitle = {
            Column {
                if (subtitle != null) Text(text = subtitle)
                Box(
                    modifier = Modifier
                        .fillMaxWidth()
                        .padding(top = TextInputGap)
                        .clip(adminTileShape())
                        // 输入区用 insetBg：磁贴底是 cardBg，若输入框也用 cardBg 会看不出可编辑区
                        .background(AdminC.insetBg)
                        .padding(horizontal = AdminSpace.md.dp, vertical = AdminSpace.sm.dp),
                ) {
                    AdminTextInput(
                        value = value,
                        onValueChange = onValueChange,
                        placeholder = placeholder,
                        singleLine = singleLine,
                        enabled = enabled,
                    )
                }
            }
        },
        colors = adminListItemColors(),
        shape = adminTileShape(),
    )
}

/** 裸输入框本体（磁贴与内联场景共用）。 */
@Composable
private fun AdminTextInput(
    value: String,
    onValueChange: (String) -> Unit,
    placeholder: String,
    singleLine: Boolean,
    enabled: Boolean,
) {
    BasicTextField(
        value = value,
        onValueChange = onValueChange,
        enabled = enabled,
        singleLine = singleLine,
        textStyle = TextStyle(color = AdminC.textPrimary, fontSize = AdminType.body.sp),
        cursorBrush = SolidColor(AdminC.accent),
        modifier = Modifier.fillMaxWidth(),
        decorationBox = { inner ->
            Box {
                if (value.isEmpty() && placeholder.isNotEmpty()) {
                    Text(
                        text = placeholder,
                        color = AdminC.textPlaceholder,
                        fontSize = AdminType.body.sp,
                    )
                }
                inner()
            }
        },
    )
}

private val DropdownOptionHeight = 40.dp

/** 说明文字与输入框之间的间距。 */
private val TextInputGap = 6.dp
