package com.xiaozhi.admin.kit

import androidx.compose.runtime.Composable
import com.gearui.components.select.Select
import com.gearui.components.select.SelectOption
import com.gearui.foundation.field.FieldVariant
import com.tencent.kuikly.compose.foundation.layout.Column
import com.tencent.kuikly.compose.foundation.layout.fillMaxWidth
import com.tencent.kuikly.compose.ui.Modifier

/**
 * 标签 + 下拉选择（Compose 版）—— 旧 DSL `dropdownField` 的等价物，基座换成 GearUI `Select`。
 *
 * ## 为什么不再需要 `form.openDropdown` / `toggleDropdown` / `isDropdownOpen(槽位)`
 *
 * 旧实现的下拉是**自绘面板**：展开状态得由页面状态持有，于是 `ConfigFormState` 里堆了一组
 * 槽位字符串（`tts_mode` / `llm_model` / `xfyun_voice`…），还要保证"同时只开一个"。
 * GearUI `Select` 自带面板与遮罩（Popover + Overlay），展开状态归组件自身，
 * 所以**调用方只要给值、选项、回调**三件事。
 *
 * ⚠️ 契约层不动：`ConfigFormState.openDropdown` / `toggleDropdown` / `isDropdownOpen` 仍保留
 * （T05 硬约束：字段与方法签名零改动），只是 T07 起不再有调用方；T12 清理时一并删除。
 *
 * @param selectedId 当前值（状态层字段原样）
 * @param options    (id → 文案) 列表；与旧 `Pair` 口径一致，顺序即展示顺序
 */
@Composable
fun labeledSelect(
    label: String,
    selectedId: String,
    options: List<Pair<String, String>>,
    onSelect: (String) -> Unit,
    modifier: Modifier = Modifier,
    help: String = "",
    placeholder: String = "请选择",
) {
    Column(modifier.fillMaxWidth()) {
        fieldLabelRow(label, help)
        Select(
            value = selectedId.takeIf { it.isNotEmpty() },
            options = options.map { (id, text) -> SelectOption(value = id, label = text) },
            onValueChange = onSelect,
            modifier = Modifier.fillMaxWidth(),
            placeholder = placeholder,
            // 卡片内字段统一 SECONDARY 变体（与 labeledField 同一口径）
            variant = FieldVariant.SECONDARY,
        )
    }
}
