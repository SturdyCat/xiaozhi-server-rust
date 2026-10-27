package com.xiaozhi.admin.compose

import androidx.compose.runtime.Composable
import com.tencent.kuikly.compose.foundation.layout.Box
import com.tencent.kuikly.compose.foundation.layout.fillMaxWidth
import com.tencent.kuikly.compose.foundation.layout.padding
import com.tencent.kuikly.compose.material3.Text
import com.tencent.kuikly.compose.ui.Modifier
import com.tencent.kuikly.compose.ui.unit.dp
import com.tencent.kuikly.compose.ui.unit.sp
import com.xiaozhi.admin.ui.AdminSpace
import com.xiaozhi.admin.ui.AdminType

/**
 * 配置段的**行模型**：一个段 = 一串 [SettingRow]，由页面逐行交给 `LazyColumn` 渲染。
 *
 * ## 为什么不让每个段自己吐一整个 Composable
 * Kuikly Compose 的 `LazyColumn` 把"整个表单"当**一个** item 时会出问题：item 高度远超
 * 视口时会出现绘制重叠（实测于「音频与播放」段首版，顶部字段与组标题叠在一起）。
 * 而且单个超高 item 也失去了 lazy 的复用意义——几十个字段一次性全部 compose。
 *
 * 行模型把"声明字段"与"渲染控件"拆开：
 * - 段只描述数据（标题、值、回调），不关心磁贴长什么样；
 * - 页面统一按行渲染，一条字段一个 lazy item，滚动与绘制都正常；
 * - 控件仍全部落到 Settings 磁贴（[AdminSwitchTile] / [AdminSegmentedTile] / ...）。
 */
internal sealed interface SettingRow {

    /** 组标题（磁贴版式里不成卡片，只是分组小标题）。 */
    data class Group(val title: String) : SettingRow

    /** 开关（旧 `switchRow`）。 */
    data class Switch(
        val title: String,
        val checked: Boolean,
        val subtitle: String? = null,
        val enabled: Boolean = true,
        val onChange: (Boolean) -> Unit,
    ) : SettingRow

    /** 多选（旧 `checkBoxRow`）。 */
    data class Check(
        val title: String,
        val checked: Boolean,
        val subtitle: String? = null,
        val enabled: Boolean = true,
        val onChange: (Boolean) -> Unit,
    ) : SettingRow

    /** 单选行（一组 [Radio] 由 [Group] 串起来）。 */
    data class Radio(
        val title: String,
        val selected: Boolean,
        val subtitle: String? = null,
        val enabled: Boolean = true,
        val onClick: () -> Unit,
    ) : SettingRow

    /** 分段单选（旧 `segmentedChoice`）。 */
    data class Segmented(
        val title: String,
        val options: List<TileOption>,
        val selectedId: String,
        val subtitle: String? = null,
        val enabled: Boolean = true,
        val onSelect: (String) -> Unit,
    ) : SettingRow

    /** 下拉单选（旧 `dropdownField`）。 */
    data class Dropdown(
        val title: String,
        val options: List<TileOption>,
        val selectedId: String,
        val subtitle: String? = null,
        val placeholder: String = "未选择",
        val enabled: Boolean = true,
        val onSelect: (String) -> Unit,
    ) : SettingRow

    /** 文本/数字输入（旧 `labeledField` / `labeledIntField`）。 */
    data class TextRow(
        val title: String,
        val value: String,
        val subtitle: String? = null,
        val placeholder: String = "",
        val enabled: Boolean = true,
        val onChange: (String) -> Unit,
    ) : SettingRow

    /** 滑杆（旧 `slider`）。 */
    data class SliderRow(
        val title: String,
        val valueText: String,
        val value: Float,
        val enabled: Boolean = true,
        val onChange: (Float) -> Unit,
    ) : SettingRow

    /** 只读信息（旧 `infoRow`）。 */
    data class Info(val title: String, val value: String, val subtitle: String? = null) : SettingRow
}

/** 行 → 磁贴。全项目唯一的分发点，新增控件只改这里。 */
@Composable
internal fun SettingRowView(row: SettingRow) {
    // 上游 SettingsGroup 组内默认 spacedBy(8dp)；这里每行是独立 lazy item，
    // 间距要自己补，否则相邻磁贴的圆角会直接相接（看起来像一串胶囊）。
    Box(modifier = Modifier.fillMaxWidth().padding(bottom = RowGap)) {
        SettingRowContent(row)
    }
}

@Composable
private fun SettingRowContent(row: SettingRow) {
    when (row) {
        is SettingRow.Group -> GroupTitle(row.title)
        is SettingRow.Switch -> AdminSwitchTile(
            title = row.title,
            state = row.checked,
            subtitle = row.subtitle,
            enabled = row.enabled,
            onCheckedChange = row.onChange,
        )
        is SettingRow.Check -> AdminCheckTile(
            title = row.title,
            state = row.checked,
            subtitle = row.subtitle,
            enabled = row.enabled,
            onCheckedChange = row.onChange,
        )
        is SettingRow.Radio -> AdminRadioTile(
            title = row.title,
            state = row.selected,
            subtitle = row.subtitle,
            enabled = row.enabled,
            onClick = row.onClick,
        )
        is SettingRow.Segmented -> AdminSegmentedTile(
            title = row.title,
            options = row.options,
            selectedId = row.selectedId,
            subtitle = row.subtitle,
            enabled = row.enabled,
            onSelect = row.onSelect,
        )
        is SettingRow.Dropdown -> AdminDropdownTile(
            title = row.title,
            options = row.options,
            selectedId = row.selectedId,
            subtitle = row.subtitle,
            placeholder = row.placeholder,
            enabled = row.enabled,
            onSelect = row.onSelect,
        )
        is SettingRow.TextRow -> AdminTextTile(
            title = row.title,
            value = row.value,
            subtitle = row.subtitle,
            placeholder = row.placeholder,
            enabled = row.enabled,
            onValueChange = row.onChange,
        )
        is SettingRow.SliderRow -> com.alorma.compose.settings.ui.SettingsSlider(
            title = { Text(text = row.title) },
            subtitle = { Text(text = row.valueText) },
            value = row.value,
            onValueChange = row.onChange,
            valueRange = 0f..100f,
            steps = 99,
            enabled = row.enabled,
            colors = adminListItemColors(),
            sliderColors = adminSliderColors(),
            shape = adminTileShape(),
        )
        is SettingRow.Info -> AdminInfoTile(row.title, row.value, row.subtitle)
    }
}

/** 组标题行：与上一个组的磁贴拉开一段距离，明确分组边界。 */
@Composable
private fun GroupTitle(title: String) {
    Box(modifier = Modifier.fillMaxWidth().padding(top = AdminSpace.lg.dp, bottom = 4.dp)) {
        // 用 body 而非 caption：组标题的层级必须高于磁贴标题（bodyMedium），
        // 否则会出现"小标题管着大标题"的视觉倒挂（首版实测）。
        Text(text = title, color = AdminC.textPrimary, fontSize = AdminType.body.sp)
    }
}

/** 磁贴行间距（对齐上游 `SettingsGroup` 的 `Arrangement.spacedBy(8.dp)`）。 */
private val RowGap = 8.dp
