package com.xiaozhi.admin

import androidx.compose.runtime.Composable
import com.gearui.components.alert.Alert
import com.gearui.components.alert.AlertStatus
import com.gearui.components.button.Button
import com.gearui.components.button.ButtonType
import com.gearui.primitives.composite.Card
import com.gearui.foundation.layout.Spacing
import com.gearui.foundation.primitives.Text
import com.gearui.theme.Theme
import com.tencent.kuikly.compose.foundation.layout.Arrangement
import com.tencent.kuikly.compose.foundation.layout.Box
import com.tencent.kuikly.compose.foundation.layout.Column
import com.tencent.kuikly.compose.foundation.layout.Row
import com.tencent.kuikly.compose.foundation.layout.fillMaxWidth
import com.tencent.kuikly.compose.foundation.layout.padding
import com.tencent.kuikly.compose.ui.Alignment
import com.tencent.kuikly.compose.ui.Modifier
import com.tencent.kuikly.compose.ui.unit.dp
import com.xiaozhi.admin.kit.actionRow
import com.xiaozhi.admin.kit.switchRow

/**
 * 配置 section 的**分发入口** + 每个 section 共用的头部横幅。
 *
 * ## ⚠️ 后续任务只改这个 `when` 的一行（并行协作的关键）
 *
 * T07–T11 每个任务的实现都落在**各自的文件**里，分发只在这里加/换一行：
 *
 * | id | 实现函数 | 所在文件 | 归属 |
 * | --- | --- | --- | --- |
 * | `vad` `asr` `tts` `llm` | `VadConfigCard` … `LlmConfigCard` | `ConfigStageCards.kt` | T07 |
 * | `nlu` | `NluConfigCard` | `ConfigNluCard.kt` | T07 |
 * | `command` | `CommandConfigCard` | `ConfigCommandCard.kt` | T07 |
 * | `wakeup` | `WakeupConfigCard` | `ConfigWakeupCard.kt` | T07 |
 * | `soul` / `memory` | `SoulConfigCard` / `MemoryConfigCard` | `ConfigSoulCard.kt` / `ConfigMemoryCard.kt` | T07 |
 * | `audio` | `AudioConfigCard` | `ConfigAudioCard.kt` | T07 |
 * | `overview` | `OverviewSection` | `ConfigOverviewCard.kt`（含 `EngineHealthCard.kt`） | T08 |
 * | `presets` | `PresetSection` | `ConfigPresetCard.kt` | T08 |
 * | `server` / `aiui` | `ServerSection` / `AiuiSection` | `ConfigInfrastructureCards.kt` | T08 |
 * | `echomuse` | `EchoMuseSection` | `ConfigEchoMuseCard.kt` | T08 |
 * | `mcp` | `McpSection` | `McpToolsCard.kt` | T08 |
 * | `ota` | `OtaSection`（只做入口卡，上传归固件维护面） | `ConfigOtaEntryCard.kt` | T08 |
 *
 * 上表已全部落地（T07 + T08）：新增 section 时只在**自己的文件**里写一个 `@Composable`，
 * 再在这里加一行分支；本文件不承载任何 section 的实现。
 *
 * ## 为什么横幅放在分发之外
 *
 * 「改动影响」「编辑对象」「就绪度」三条横幅是**所有 section 共用**的信息层，
 * 放在分发外层就不会因为某个 section 重写而漏掉其中一条（旧实现正因为把它们写进
 * `renderConfigSection` 的同一棵树里，每加一个 section 都要记得抄一遍）。
 */
@Composable
fun ConfigSectionHost(
    form: ConfigFormState,
    meta: PluginMetaState,
    health: EngineHealthState,
    mcp: McpToolsState,
    ota: OtaState,
    toast: ToastState,
    id: String,
) {
    Column(modifier = Modifier.fillMaxWidth()) {
        // 改动影响高亮：上次保存的 hot 投影到当前 section 头部（仅确有非即时影响时显示）
        if (form.saveImpactNote.isNotEmpty()) {
            Alert(
                title = "本次保存的影响",
                description = form.saveImpactNote,
                status = AlertStatus.WARNING,
                modifier = Modifier.fillMaxWidth().padding(bottom = Spacing.sm),
            )
        }
        // 编辑对象横幅（P1-5）：设备个性化目标时显示（全局模式不渲染）
        EditTargetBanner(form, toast)
        // 就绪度徽标：对应能力 phase 偏离（degraded/failed）时显示状态 + 原因
        SectionReadinessBanner(meta, id)

        // ===== 分发：每个分支一行 =====
        when (id) {
            "vad" -> VadConfigCard(form)
            "asr" -> AsrConfigCard(form)
            "nlu" -> NluConfigCard(form)
            "llm" -> LlmConfigCard(form)
            "tts" -> TtsConfigCard(form)
            "command" -> CommandConfigCard(form)
            "wakeup" -> WakeupConfigCard(form)
            "soul" -> SoulConfigCard(form)
            "memory" -> MemoryConfigCard(form)
            "audio" -> AudioConfigCard(form)
            // ---- 以下 7 个属于「全局配置」（T08），各自实现落在自己的文件 ----
            "overview" -> OverviewSection(form, meta, health)
            "presets" -> PresetSection(form)
            "server" -> ServerSection(form)
            "aiui" -> AiuiSection(form)
            "echomuse" -> EchoMuseSection(form)
            "mcp" -> McpSection(mcp)
            "ota" -> OtaSection(ota)
            // 未注册内容不要静默留白（"新加了 section 却忘了写内容"会表现成点进去什么都没有）
            else -> UnknownSectionCard(id)
        }
    }
}

/**
 * 取某 section 对应的能力行（来自 [PluginMetaState]，即 `GET /api/plugins` 的权威就绪态）。
 * 无对应能力返回 null。
 */
internal fun sectionCapability(meta: PluginMetaState, sectionId: String): CapabilityRow? {
    val capId = ConfigSections.all.firstOrNull { it.id == sectionId }?.capabilityId ?: return null
    for (r in meta.requiredRows) if (r.id == capId) return r
    for (r in meta.optionalRows) if (r.id == capId) return r
    return null
}

/**
 * section 头部「就绪度」横幅：仅当对应能力 phase 偏离（`failed` / `degraded`）时显示状态 + 原因。
 * 正常（`active` / 未加载 / 无对应能力）不打扰 —— 一屏绿点等于没有信息。
 */
@Composable
private fun SectionReadinessBanner(meta: PluginMetaState, sectionId: String) {
    val row = sectionCapability(meta, sectionId) ?: return
    if (row.phase != "failed" && row.phase != "degraded") return
    val failed = row.phase == "failed"
    Alert(
        title = if (failed) "该能力当前异常" else "该能力当前降级",
        description = if (row.reason.isNotEmpty()) row.reason else "服务端未给出原因",
        status = if (failed) AlertStatus.DANGER else AlertStatus.WARNING,
        modifier = Modifier.fillMaxWidth().padding(bottom = Spacing.sm),
    )
}

/**
 * 「高级选项」开关行（P0-1）：放在卡片基础字段之后，其后的高级字段区由各卡片自己
 * `if (form.showAdvanced) { … }` 展开。开关状态全局共享（同一时刻只看一个 section，无歧义）。
 */
@Composable
fun AdvancedToggleRow(form: ConfigFormState) {
    switchRow(
        label = "显示高级选项",
        checked = form.showAdvanced,
        onToggle = { form.showAdvanced = it },
        help = "高级字段是调优项（缓存容量、切句时序、检索预算、上下文长度等），" +
            "默认折叠以免淹没常用配置；展开状态全局共享，不影响保存内容。",
    )
}

/**
 * 密钥输入框的占位提示。
 *
 * 服务端 `GET /api/config` 已打码（只回 `has_<field>` presence 标记），因此：
 * - `configured = true` 且输入框为空 → 提示"留空则不修改"，避免用户误以为密钥丢了；
 * - `configured = false` → 明确提示尚未配置。
 */
internal fun secretHint(configured: Boolean): String =
    if (configured) "已配置（留空则不修改）" else "未配置"

/**
 * 「当前编辑对象」横幅：编辑目标为某设备时显示（全局模式不渲染）。
 *
 * 说清覆盖范围（只有会话内参数可按设备覆盖，结构型字段始终走全局）——不说清楚，
 * 用户会在设备模式里改 Server 端口、以为只影响这台设备（实际始终是全局值）。
 */
@Composable
private fun EditTargetBanner(form: ConfigFormState, toast: ToastState) {
    if (form.editDeviceId.isEmpty()) return
    // Pager 只能在 @Composable 作用域取（onClick 闭包里不是 composable 上下文）→ 先取再闭包捕获
    val pager = LocalAdminPagerOrThrow()
    LabeledCard(title = "当前编辑对象：设备 ${form.editDeviceId}") {
        Text(
            text = "正在编辑设备「${form.editDeviceId}」的个性化配置：仅音色 / 语速 / 提示词 / 人格 / " +
                "唤醒问候 / 指令 / 记忆召回等会话内参数可按设备覆盖；Server / Audio / 模型与引擎等" +
                "结构型字段始终沿用全局。保存按钮提交的是相对全局的差量。",
            style = Theme.typography.caption,
            color = Theme.colors.mutedForeground,
        )
        actionRow {
            Button(
                text = "返回全局",
                onClick = { form.backToGlobal(pager) },
                type = ButtonType.OUTLINE,
            )
            Button(
                text = "恢复全局配置",
                onClick = {
                    form.clearDeviceOverlay(pager) { msg, level -> toast.show(msg, level) }
                },
                type = ButtonType.OUTLINE,
            )
        }
    }
}

/**
 * 取当前页面的 `Pager`（Compose 里原来靠 `ctx` 参数传，改用 CompositionLocal，
 * 免得每个卡片都多一个 `ctx: Pager` 形参；壳层已在根容器 provide）。
 */
@Composable
internal fun LocalAdminPagerOrThrow(): com.tencent.kuikly.core.pager.Pager =
    requireNotNull(com.xiaozhi.admin.kit.LocalAdminPager.current) {
        "LocalAdminPager 未提供：配置卡片必须在 AdminScopeHost 内渲染"
    }

/**
 * 带标题的卡片壳（`Card` + 标题行）：所有 section 卡片共用的最小外壳。
 *
 * 旧 DSL 的 `groupedCard(title, withDivider)` 在这里只有"卡片 + 标题"两层，
 * 内部分隔线交给各组自己用 `dividerH()`（Compose 版见 `kit/GearLayout.kt`）。
 */
@Composable
fun LabeledCard(
    title: String,
    modifier: Modifier = Modifier,
    trailing: (@Composable () -> Unit)? = null,
    content: @Composable () -> Unit,
) {
    Card(modifier = modifier.fillMaxWidth().padding(bottom = Spacing.lg)) {
        Column(modifier = Modifier.fillMaxWidth().padding(Spacing.lg)) {
            Row(
                modifier = Modifier.fillMaxWidth().padding(bottom = Spacing.sm),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.SpaceBetween,
            ) {
                Text(
                    text = title,
                    style = Theme.typography.titleSmall,
                    color = Theme.colors.foreground,
                )
                Box { if (trailing != null) trailing() }
            }
            content()
        }
    }
}

/** 未注册 section：明确报错而不是留白（"新加 section 忘了写内容"会表现成空页）。 */
@Composable
private fun UnknownSectionCard(id: String) {
    LabeledCard(title = "未知配置页") {
        Text(
            text = "管理页没有为 section「$id」注册内容（请检查 ConfigSections.all 与 ConfigSectionHost 的 when）。",
            style = Theme.typography.bodyMedium,
            color = Theme.colors.destructive,
        )
    }
}
