package com.xiaozhi.admin.compose

import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import com.tencent.kuikly.compose.foundation.background
import com.tencent.kuikly.compose.foundation.clickable
import com.tencent.kuikly.compose.foundation.layout.Box
import com.tencent.kuikly.compose.foundation.layout.Column
import com.tencent.kuikly.compose.foundation.layout.PaddingValues
import com.tencent.kuikly.compose.foundation.layout.Row
import com.tencent.kuikly.compose.foundation.layout.fillMaxHeight
import com.tencent.kuikly.compose.foundation.layout.fillMaxSize
import com.tencent.kuikly.compose.foundation.layout.fillMaxWidth
import com.tencent.kuikly.compose.foundation.layout.height
import com.tencent.kuikly.compose.foundation.layout.padding
import com.tencent.kuikly.compose.foundation.layout.width
import com.tencent.kuikly.compose.foundation.lazy.LazyColumn
import com.tencent.kuikly.compose.material3.Button
import com.tencent.kuikly.compose.material3.ButtonDefaults
import com.tencent.kuikly.compose.material3.Text
import com.tencent.kuikly.compose.ui.Alignment
import com.tencent.kuikly.compose.ui.Modifier
import com.tencent.kuikly.compose.ui.unit.dp
import com.tencent.kuikly.compose.ui.unit.sp
import com.tencent.kuikly.core.nvi.serialization.json.JSONObject
import com.tencent.kuikly.core.pager.Pager
import com.xiaozhi.admin.ConfigSections
import com.xiaozhi.admin.ui.AdminSpace
import com.xiaozhi.admin.ui.AdminType

/**
 * 「全局配置」section 的 Compose 实现（迁移样板）。
 *
 * 结构与旧版 `renderGlobalConfigLayout` 一致：左「段」菜单 │ 1dp 竖线 │ 右表单。
 * 左侧菜单仍由 [ConfigSections.globalGroups] 驱动（与旧版同一份 section 表），
 * 只是渲染换成 Compose。
 *
 * ## 迁移进度
 * 已接真实配置：AIUI 全链路、服务、音频与播放。其余段显示迁移占位 + 旧版入口，
 * 保证任一菜单都能点、都有明确状态，不会出现空白页。
 *
 * ## 表单控件一律走 Settings 磁贴
 * 开关 / 单选 / 多选统一由 [AdminSwitchTile] / [AdminSegmentedTile] / [AdminRadioTile] /
 * [AdminCheckTile] / [AdminDropdownTile] 承载（见 `AdminSettingsControls.kt`），
 * ⛔ 不在页面里直接调用上游 `SettingsXxx` 并手传主题参数，也不要退回旧 DSL 控件。
 *
 * ## 与旧版共用同一套服务端契约
 * 每段一份 [AdminConfigDraft]：`GET /api/config` 读、`POST /api/config` 部分更新写，
 * 带 `expected_revision`，冲突时服务端回 409 而不覆盖。
 */

private val SectionMenuWidth = 200.dp
private val SectionItemHeight = 40.dp

@Composable
internal fun AdminSettingsSection(pager: Pager, onOpenLegacy: (String) -> Unit) {
    // 初始段可由 URL 参数指定（`?page_name=admin-compose&section=audio`）：深链与本地视觉核对都要用
    // 每段一份独立草稿：切段不重复拉整份配置，也不丢未保存的输入
    val drafts = remember {
        mapOf<String, AdminConfigDraft>(
            "aiui" to AiuiDraft(),
            "server" to ServerDraft(),
            "echomuse" to EchoMuseDraft(),
            // ⚠️ `audio` 有流程节点（输入/输出），菜单归属「语音流程」而非本页；
            // 这里保留草稿是为了语音流程页迁完后复用同一份读写实现，深链也不会空白。
            "audio" to AudioDraft(),
        )
    }
    // 初始段可由 URL 参数指定（`?page_name=admin-compose&section=audio`）：深链与本地视觉核对都要用
    var selected by remember {
        mutableStateOf(
            pager.pageData.params.optString("section", "")
                .takeIf { it.isNotEmpty() && drafts.containsKey(it) } ?: "aiui",
        )
    }
    val active = drafts[selected]
    LaunchedEffect(selected) { active?.load(pager) }

    Row(modifier = Modifier.fillMaxSize()) {
        Column(
            modifier = Modifier
                .width(SectionMenuWidth)
                .fillMaxHeight()
                .background(AdminC.windowBg),
        ) {
            LazyColumn(modifier = Modifier.fillMaxSize()) {
                item {
                    for ((groupLabel, sections) in ConfigSections.globalGroups) {
                        Text(
                            text = groupLabel,
                            color = AdminC.textTertiary,
                            fontSize = AdminType.micro.sp,
                            modifier = Modifier.padding(
                                start = AdminSpace.sm.dp,
                                top = AdminSpace.md.dp,
                                bottom = 4.dp,
                            ),
                        )
                        for (section in sections) {
                            SectionMenuItem(
                                label = section.label,
                                selected = selected == section.id,
                                onClick = { selected = section.id },
                            )
                        }
                    }
                }
            }
        }

        Box(modifier = Modifier.width(1.dp).fillMaxHeight().background(AdminC.divider))

        LazyColumn(
            modifier = Modifier.weight(1f).fillMaxHeight(),
            contentPadding = PaddingValues(AdminSpace.pagePadding.dp),
        ) {
            if (active != null) {
                item {
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        AdminButton(
                            text = if (active.loading) "加载中…" else "重新加载",
                            enabled = !active.loading && !active.saving,
                        ) { active.load(pager, force = true) }
                        Box(modifier = Modifier.width(AdminSpace.sm.dp))
                        AdminButton(
                            text = if (active.saving) "保存中…" else "保存",
                            enabled = !active.loading && !active.saving,
                            primary = true,
                        ) { active.save(pager) }
                        if (active.message.isNotEmpty()) {
                            Text(
                                text = active.message,
                                color = if (active.isError) AdminC.danger else AdminC.textSecondary,
                                fontSize = AdminType.caption.sp,
                                modifier = Modifier.padding(start = AdminSpace.sm.dp),
                            )
                        }
                    }
                }
                // ⚠️ 一条字段一个 lazy item：整段塞进单个 item 会因超高出现绘制重叠
                // （实测于「音频与播放」首版）。行模型见 SettingRow。
                for (row in active.rows()) {
                    item { SettingRowView(row) }
                }
            } else {
                item {
                    MigrationNotice(
                        sectionLabel = ConfigSections.all.firstOrNull { it.id == selected }?.label ?: selected,
                        onOpenLegacy = { onOpenLegacy(selected) },
                    )
                }
            }
        }
    }
}

@Composable
private fun SectionMenuItem(label: String, selected: Boolean, onClick: () -> Unit) {
    Box(
        modifier = Modifier
            .fillMaxWidth()
            .height(SectionItemHeight)
            .background(if (selected) AdminC.sidebarSelectedBg else AdminC.transparent)
            .clickable { onClick() },
    ) {
        if (selected) {
            Box(
                modifier = Modifier
                    .width(3.dp)
                    .fillMaxHeight()
                    .background(AdminC.accent),
            )
        }
        Text(
            text = label,
            color = if (selected) AdminC.textAccent else AdminC.textPrimary,
            fontSize = AdminType.body.sp,
            modifier = Modifier.align(Alignment.CenterStart).padding(start = AdminSpace.sm.dp),
        )
    }
}

@Composable
private fun AdminButton(
    text: String,
    enabled: Boolean,
    primary: Boolean = false,
    onClick: () -> Unit,
) {
    Button(
        onClick = onClick,
        enabled = enabled,
        colors = ButtonDefaults.buttonColors(
            containerColor = if (primary) AdminC.accent else AdminC.fieldBg,
            contentColor = if (primary) AdminC.textOnAccent else AdminC.textPrimary,
            disabledContainerColor = AdminC.disabledBg,
            disabledContentColor = AdminC.disabledText,
        ),
    ) {
        Text(text = text, fontSize = AdminType.body.sp)
    }
}

@Composable
private fun MigrationNotice(sectionLabel: String, onOpenLegacy: () -> Unit) {
    Column(modifier = Modifier.padding(top = AdminSpace.md.dp)) {
        Text(
            text = "「$sectionLabel」尚未迁移到 Compose 磁贴",
            color = AdminC.textPrimary,
            fontSize = AdminType.body.sp,
        )
        Text(
            text = "该段功能仍在旧版管理台完整可用。",
            color = AdminC.textSecondary,
            fontSize = AdminType.caption.sp,
            modifier = Modifier.padding(top = AdminSpace.xs.dp),
        )
        Text(
            text = "前往旧版 →",
            color = AdminC.textAccent,
            fontSize = AdminType.body.sp,
            modifier = Modifier.padding(top = AdminSpace.sm.dp).clickable { onOpenLegacy() },
        )
    }
}

/**
 * AIUI 段草稿。默认值与旧版 `ConfigFormState` 对齐（speed/volume/pitch=50、pace_ms=10），
 * 避免两个入口对同一字段给出不同兜底。
 *
 * 三个滑杆仍用 Float（滑杆需要连续值），提交时取整——服务端字段是整数。
 */
internal class AiuiDraft : AdminConfigDraft() {
    private var enabled by draftBool(false)
    private var speed by mutableStateOf(50f)
    private var volume by mutableStateOf(50f)
    private var pitch by mutableStateOf(50f)
    private var paceMs by mutableStateOf(10f)

    override fun fill(config: JSONObject) {
        config.optJSONObject("aiui")?.let { a ->
            enabled = a.optBoolean("enabled", enabled)
            speed = a.optInt("speed", speed.toInt()).toFloat()
            volume = a.optInt("volume", volume.toInt()).toFloat()
            pitch = a.optInt("pitch", pitch.toInt()).toFloat()
            paceMs = a.optInt("pace_ms", paceMs.toInt()).toFloat()
        }
    }

    override fun collect(): JSONObject = JSONObject().apply {
        put("aiui", JSONObject().apply {
            put("enabled", enabled)
            put("speed", speed.toInt())
            put("volume", volume.toInt())
            put("pitch", pitch.toInt())
            put("pace_ms", paceMs.toInt())
        })
    }

    override fun rows(): List<SettingRow> = listOf(
        SettingRow.Group("AIUI 全链路模式"),
        SettingRow.Switch(
            title = "启用全链路模式",
            checked = enabled,
            subtitle = "关闭后走本地 VAD/ASR/LLM/TTS 流程",
            onChange = { enabled = it; markDirty() },
        ),
        SettingRow.SliderRow(
            title = "语速",
            valueText = "${speed.toInt()}",
            value = speed,
            enabled = enabled,
            onChange = { speed = it; markDirty() },
        ),
        SettingRow.SliderRow(
            title = "音量",
            valueText = "${volume.toInt()}",
            value = volume,
            enabled = enabled,
            onChange = { volume = it; markDirty() },
        ),
        SettingRow.SliderRow(
            title = "音调",
            valueText = "${pitch.toInt()}",
            value = pitch,
            enabled = enabled,
            onChange = { pitch = it; markDirty() },
        ),
        SettingRow.SliderRow(
            title = "句间停顿",
            valueText = "${paceMs.toInt()} ms",
            value = paceMs,
            enabled = enabled,
            onChange = { paceMs = it; markDirty() },
        ),
    )
}
