package com.xiaozhi.admin

import com.xiaozhi.admin.ui.*

import com.tencent.kuikly.core.base.ViewContainer
import com.tencent.kuikly.core.directives.vfor
import com.tencent.kuikly.core.directives.vif
import com.tencent.kuikly.core.pager.Pager
import com.tencent.kuikly.core.views.Text
import com.tencent.kuikly.core.views.View

/**
 * section id → 对应能力 capability id（`GET /api/plugins` 的 `capabilities[].id`，见
 * `server/src/plugins/registry/mod.rs` 的 `Capability::id`）。除 AIUI 段在注册表里叫
 * `fullchain` 外，多数 section 与其能力 id 同名。server / audio / ota / mcp / overview
 * 无单一能力对应（不在表中 → 查询返回 null，不显示就绪点）。
 */

/**
 * 取某 section 对应的能力行（来自 [PluginMetaState]，即 `GET /api/plugins` 的权威就绪态）。
 * 无对应能力返回 null。
 *
 * ⚠️ 在闭包内读 `meta` 的 observable 列表 → meta 加载/刷新后自动重算（Kuikly 响应式铁律：
 * 闭包内读取才被跟踪）。[CapabilityRow] 本身是不可变快照，meta 整表替换（clear + add）时
 * 列表被观察、触发重渲染。
 */
internal fun sectionCapability(meta: PluginMetaState, sectionId: String): CapabilityRow? {
    val capId = ConfigSections.all.firstOrNull { it.id == sectionId }?.capabilityId ?: return null
    for (r in meta.requiredRows) if (r.id == capId) return r
    for (r in meta.optionalRows) if (r.id == capId) return r
    return null
}

/**
 * section 头部「就绪度」横幅：仅当对应能力 phase 偏离（`failed` / `degraded`）时显示
 * 状态徽标 + 原因（复用总览页同款 [statusBadge]，不另造样式）。
 * 正常（`active` / 未加载 / 无对应能力）不打扰。
 */
private fun ViewContainer<*, *>.sectionReadinessBanner(meta: PluginMetaState, sectionId: String) {
    vif({ sectionCapability(meta, sectionId)?.phase in setOf("failed", "degraded") }) {
        val row = sectionCapability(meta, sectionId)!!
        val (badgeState, badgeText) = when (row.phase) {
            "failed" -> "error" to "异常"
            "degraded" -> "busy" to "降级"
            else -> "idle" to row.phase
        }
        View {
            attr {
                flexDirectionRow()
                alignItemsCenter()
                marginBottom(AdminSpace.sm)
            }
            statusBadge(state = { badgeState }, text = { badgeText })
            vif({ row.reason.isNotEmpty() }) {
                hGap(AdminSpace.sm)
                Text {
                    attr {
                        flex(1f)
                        fontSize(AdminType.micro)
                        color(if (row.phase == "failed") AdminColors.dangerTintText else AdminColors.warningTintText)
                        text(row.reason)
                    }
                }
            }
        }
    }
}

/**
 * 渲染单个配置 section（按 id 分发到既有卡片函数）。
 * 把 renderForm 里"全部 14 页塞进 PageList"的 `when` 收敛为"按需渲染一个"，
 * 卡片函数本身零改动、零重复。
 */
fun ViewContainer<*, *>.renderConfigSection(
    form: ConfigFormState,
    meta: PluginMetaState,
    health: EngineHealthState,
    mcp: McpToolsState,
    ota: OtaState,
    toast: ToastState,
    ctx: Pager,
    id: String,
) {
    // ⚠️ vfor 项闭包必须且仅能创建一个孩子节点（2.29.1 起严格校验，多个根节点直接抛
    // "vfor creator闭包内必须需要且仅一个孩子节点"），所以 section 的全部内容
    // （影响提示 / 横幅 / when 分支的多张卡）统一收进这一个列容器。
    View {
        attr {
            flex(1f)
            flexDirectionColumn()
        }
        // 改动影响高亮：上次保存的 hot 投影到当前 section 头部（仅确有非即时影响时显示）
        vif({ form.saveImpactNote.isNotEmpty() }) {
            textNote(form.saveImpactNote, tone = "warn", micro = true)
            vGap(AdminSpace.sm)
        }
        // 编辑对象横幅（P1-5）：设备个性化目标时显示（全局模式不渲染）
        editTargetBanner(form, toast, ctx)
        // 就绪度徽标：对应能力 phase 偏离（degraded/failed）时显示状态 + 原因（位于 section 卡片上方）
        sectionReadinessBanner(meta, id)
        when (id) {
            // 总览：能力总览卡（配置级状态）之下接能力健康卡（运行期探测合并结论，P1-6）
            "overview" -> {
                overviewCard(meta, ctx)
                engineHealthCard(health, ctx) { section -> form.openConfigSection(section) }
            }
            // 预设（P1-5）：快照 / 应用 / 删除全局配置；设备个性化模式下不可用
            "presets" -> {
                vif({ form.editDeviceId.isEmpty() }) {
                    presetCard(form, ctx)
                }
                vif({ form.editDeviceId.isNotEmpty() }) {
                    groupedCard("预设") {
                        textNote(
                            "预设仅作用于全局配置；当前正在编辑设备的个性化配置，请先点顶部「返回全局」再管理预设。",
                            micro = true,
                        )
                    }
                }
            }
            "mcp" -> mcpToolsCard(mcp, ctx)
            "ota" -> otaCard(ota, toast, ctx)
            "server" -> serverConfigCard(form)
            "audio" -> audioConfigCard(form)
            "asr" -> asrConfigCard(form)
            "vad" -> vadConfigCard(form, ctx)
            "tts" -> ttsConfigCard(form, ctx)
            "llm" -> llmConfigCard(form, ctx)
            "command" -> commandConfigCard(form)
            "nlu" -> nluConfigCard(form, ctx)
            "wakeup" -> wakeupConfigCard(form)
            "aiui" -> aiuiConfigCard(form)
            "echomuse" -> echomuseConfigCard(form)
            "soul" -> soulConfigCard(form, ctx)
            "memory" -> memoryConfigCard(form, ctx)
            // 未注册内容不要静默留白（"新加了 section 却忘了写内容"会表现成点进去什么都没有）
            else -> groupedCard("未知配置页") {
                textNote("管理页没有为 section「$id」注册内容（请检查 ConfigSections.all）。")
            }
        }
    }
}

/**
 * 「高级选项」开关行（P0-1）：放在卡片基础字段之后，其后的 `vif({ form.showAdvanced })`
 * 块即高级字段区。开关状态全局共享（同一时刻只看一个 section，无歧义）。
 */
fun ViewContainer<*, *>.advancedToggleRow(form: ConfigFormState) {
    switchRow(
        "显示高级选项", form.showAdvanced,
        help = "高级字段是调优项（缓存容量、切句时序、检索预算、上下文长度等），" +
            "默认折叠以免淹没常用配置；展开状态全局共享，不影响保存内容。",
    ) { form.showAdvanced = !form.showAdvanced }
}

// ============================================================
// 各配置分组卡（renderForm 与测试台 tab 右栏共用；字段变更统一 markDirty）
// ============================================================

/**
 * 密钥输入框的占位提示。
 *
 * 服务端 `GET /api/config` 已打码（只回 `has_<field>` presence 标记），因此：
 * - `configured = true` 且输入框为空 → 提示"留空则不修改"，避免用户误以为密钥丢了、或
 *   以为必须重填（重填会覆盖）；
 * - `configured = false` → 明确提示尚未配置。
 */
internal fun secretHint(configured: Boolean): String =
    if (configured) "已配置（留空则不修改）" else "未配置"

// ============================================================
// 设备个性化（P1-5）：编辑对象横幅
// ============================================================

/**
 * 「当前编辑对象」横幅：编辑目标为某设备时显示（全局模式不渲染）。
 *
 * 说清覆盖范围（只有会话内参数可按设备覆盖，结构型字段始终走全局）——不说清楚，
 * 用户会在设备模式里改 Server 端口、以为只影响这台设备（实际始终是全局值）。
 * 出口两个：「返回全局」只是切回全局编辑（不动设备 overlay）；
 * 「恢复全局配置」删除该设备 overlay（POST .../overlay/delete）后切回全局。
 */
fun ViewContainer<*, *>.editTargetBanner(form: ConfigFormState, toast: ToastState, ctx: Pager) {
    vif({ form.editDeviceId.isNotEmpty() }) {
        groupedCard("当前编辑对象：设备 ${form.editDeviceId}") {
            textNote(
                "正在编辑设备「${form.editDeviceId}」的个性化配置：仅音色 / 语速 / 提示词 / 人格 / " +
                    "唤醒问候 / 指令 / 记忆召回等会话内参数可按设备覆盖；Server / Audio / 模型与引擎等" +
                    "结构型字段始终沿用全局。保存按钮提交的是相对全局的差量。",
                micro = true,
            )
            actionRow {
                secondaryButton("返回全局") { form.backToGlobal(ctx) }
                hGap(AdminSpace.sm)
                secondaryButton("恢复全局配置") {
                    form.clearDeviceOverlay(ctx) { msg, level -> toast.show(msg, level) }
                }
            }
        }
    }
}

// ============================================================
// 配置预设（P1-5）：快照 / 应用 / 删除
// ============================================================

/**
 * 配置预设卡：把当前全局配置快照为预设（服务端读盘），列表内逐项应用 / 删除。
 * 应用走 `PUT /api/config` 同款部分更新管线（服务端），成功后自动重载表单。
 */
fun ViewContainer<*, *>.presetCard(form: ConfigFormState, ctx: Pager) {
    groupedCard("配置预设") {
        textNote(
            "把当前全局配置快照为预设，随时一键应用（回滚）。预设仅作用于全局配置；" +
                "保存的是服务端盘上的生效值（不是表单草稿）。",
            micro = true,
        )
        labeledField(
            "预设名",
            { form.presetNameDraft },
            { form.presetNameDraft = it; },
            "如 night（字母数字与 - _ .）",
        )
        actionRow {
            primaryButton(
                "保存当前配置为预设",
                loading = { form.presetBusy },
                loadingText = "保存中…",
            ) { form.savePreset(ctx, form.serverBase) }
        }
        vif({ form.presetMsg.isNotEmpty() }) {
            textNote(
                form.presetMsg,
                tone = if (form.presetMsg.contains("失败")) "error" else "tertiary",
                micro = true,
            )
        }
        dividerH()
        vif({ form.presetNames.size == 0 }) {
            textNote("还没有预设。填好名称点「保存当前配置为预设」即可创建。", micro = true)
        }
        vfor({ form.presetNames }) { name ->
            View {
                attr {
                    flexDirectionRow()
                    alignItemsCenter()
                    marginBottom(AdminSpace.xs)
                }
                Text {
                    attr {
                        flex(1f)
                        fontSize(AdminType.body)
                        color(AdminColors.textPrimary)
                        text(name)
                    }
                }
                secondaryButton("应用") { form.applyPreset(ctx, form.serverBase, name) }
                hGap(AdminSpace.sm)
                secondaryButton("删除") { form.deletePreset(ctx, form.serverBase, name) }
            }
        }
    }
}
