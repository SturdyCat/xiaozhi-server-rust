package com.xiaozhi.admin

import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import com.gearui.components.button.Button
import com.gearui.components.button.ButtonType
import com.gearui.foundation.primitives.Text
import com.gearui.theme.Theme
import com.tencent.kuikly.core.nvi.serialization.json.JSONObject
import com.tencent.kuikly.core.pager.Pager
import com.xiaozhi.admin.kit.HintLine
import com.xiaozhi.admin.kit.fieldLabelRow
import com.xiaozhi.admin.kit.labeledField
import com.xiaozhi.admin.kit.labeledSelect
import com.xiaozhi.admin.state.stateVar

/** 控制域展示顺序与中文标题：设备（固件执行）/ 服务器（服务端执行）/ IoT（第三方设备 · tuya）。 */
private val DOMAIN_GROUPS: List<Pair<String, String>> = listOf(
    "device" to "设备域（固件执行）",
    "server" to "服务器域（服务端执行）",
    "iot" to "IoT 域（第三方设备 · tuya）",
)

private val DOMAIN_FILTER_OPTIONS: List<Pair<String, String>> =
    listOf("all" to "全部控制域") + DOMAIN_GROUPS.map { (id, label) -> id to label.substringBefore("（") }

/** 目录里的一条技能：名称 + 控制域 + 摘要（动作、例句数、来源）。 */
data class NluCatalogSkill(
    val name: String,
    val domain: String,
    val summary: String,
)

data class NluCatalogView(
    val busy: Boolean = false,
    val message: String = "",
    val revision: String = "",
    val skills: List<NluCatalogSkill> = emptyList(),
    val capabilities: List<String> = emptyList(),
    val capabilitiesJson: String = "",
)

/** Read-only, saved effective catalog; it never injects defaults into editable drafts. */
class NluSkillCatalogState {
    var view by stateVar(NluCatalogView())
        private set
    private var generation = 0

    fun invalidate() { generation++; view = NluCatalogView() }

    fun refresh(ctx: Pager, form: ConfigFormState) {
        if (view.busy || form.editDeviceId.isNotEmpty()) return
        val ticket = ++generation
        val base = form.serverBase
        val token = ctx.adminRequestToken()
        val access = ctx.adminAccessRevision()
        view = view.copy(busy = true, message = "正在读取服务器保存态技能目录…")
        rpcCall(ctx, "$base/api/skills/catalog", "GET", null) { ok, data, status, error ->
            if (ticket != generation || base != form.serverBase || !ctx.isAdminAccessCurrent(access, token) || form.editDeviceId.isNotEmpty()) return@rpcCall
            if (!ok || data.optString("state", "") != "ok" || data.optString("recognition", "") != "model_only") {
                view = view.copy(busy = false, message = "技能清单读取失败：${data.optString("error", "").orEmpty().ifEmpty { error.ifEmpty { "服务端不支持模型技能目录（HTTP $status）" } }}")
                return@rpcCall
            }
            val intents = data.optJSONObject("effective")?.optJSONArray("intents") ?: data.optJSONArray("intents")
            val skills = mutableListOf<NluCatalogSkill>()
            if (intents != null) for (i in 0 until intents.length()) {
                val skill = intents.optJSONObject(i) ?: continue
                val action = skill.optJSONObject("action")
                val plan = action?.optJSONObject("plan")
                val kind = when (action?.optString("kind", "")) {
                    "plan" -> "能力组合（${plan?.optJSONArray("conditions")?.length() ?: 0} 条条件，${plan?.optJSONArray("steps")?.length() ?: 0} 个步骤；${nluClockSummary(plan).ifEmpty { "系统能力" }}）"
                    "reply" -> "应答"
                    "end_session" -> "结束会话"
                    else -> if (action?.optString("kind", "").orEmpty().startsWith("mcp:")) {
                        "设备工具直调（${action?.optString("kind", "").orEmpty().removePrefix("mcp:")}）"
                    } else "已配置动作"
                }
                val origin = when (skill.optString("source", "")) {
                    "command" -> "断开对话技能训练词表"
                    "default" -> "初始默认定义"
                    "custom" -> "自定义配置"
                    else -> "保存态定义"
                }
                // 旧服务端/旧保存态缺 domain 时缺省 server（与服务端缺省规则一致）。
                val domain = skill.optString("domain", "server").let { d -> if (d in listOf("device", "server", "iot")) d else "server" }
                skills.add(NluCatalogSkill(
                    name = skill.optString("name", "未命名技能"),
                    domain = domain,
                    summary = "$kind · ${skill.optJSONArray("templates")?.length() ?: 0} 条训练例句 · $origin",
                ))
            }
            val definitions = data.optJSONArray("capabilities")
            val capabilities = mutableListOf<String>()
            if (definitions != null) for (i in 0 until definitions.length()) {
                val capability = definitions.optJSONObject(i) ?: continue
                capabilities.add("${capability.optString("api", "")} · ${capability.optString("description", "")}")
            }
            view = NluCatalogView(message = "来自服务器保存态，不包含当前未保存改动；只读查询不会执行动作。来源按定义判定，不代表编辑历史。",
                revision = data.optString("revision", ""), skills = skills, capabilities = capabilities,
                capabilitiesJson = definitions?.toString().orEmpty())
        }
    }
}

/** Summarize actual saved steps, never hard-code skill-name execution branches. */
private fun nluClockSummary(plan: JSONObject?): String {
    val steps = plan?.optJSONArray("steps") ?: return ""
    val kinds = mutableSetOf<String>()
    for (i in 0 until steps.length()) {
        val step = steps.optJSONObject(i) ?: continue
        when (step.optString("api", "")) {
            "tasks.create" -> {
                val args = step.optJSONObject("args")
                val kind = listOf(
                    "after" to "倒计时", "at" to "单次定时", "interval" to "固定间隔",
                    "hourly" to "每小时", "daily" to "每日", "workdays" to "周一至周五",
                    "weekly" to "每周", "monthly" to "每月", "yearly" to "每年",
                ).firstOrNull { args?.has(it.first) == true }?.second ?: "创建提醒"
                kinds.add(kind)
            }
            "tasks.list", "tasks.get" -> kinds.add("查询提醒")
            "tasks.cancel" -> kinds.add("取消提醒")
        }
    }
    return kinds.joinToString("、")
}

@Composable
internal fun NluCatalogSection(form: ConfigFormState) {
    val ctx = LocalAdminPagerOrThrow()
    val state = form.nlu.catalog
    val view = state.view
    // 域筛选与搜索是纯展示态：放 remember，不进状态层（AGENTS 约定）。
    var domainFilter by remember { mutableStateOf("all") }
    var search by remember { mutableStateOf("") }
    fieldLabelRow("保存态技能与系统能力", help = "技能按控制域分组：设备域由设备固件工具执行（含断开对话、音量与播放控制示例），服务器域由服务端执行（提醒、天气等能力组合），IoT 域由服务端执行（tuya 第三方设备，如语音开关插座）。实际可用技能以读取的保存态目录为准；显式清空不会在页面重建。能力组合只允许调用服务器注册的 Rust 系统能力，不支持任意脚本、命令或新编译代码。",
        trailing = { Button(text = "读取技能清单", onClick = { state.refresh(ctx, form) }, type = ButtonType.OUTLINE, loading = view.busy, disabled = view.busy || form.editDeviceId.isNotEmpty()) })
    HintLine("默认时钟按北京时间；工作日仅周一至周五，不自动处理法定节假日。每月31日或每年2月29日遇不存在的日期跳过，不夹到月底。")
    HintLine("支持今天、明天、后天、大后天及上午、下午、晚上。未说日期的钟点按下一次到达，创建结果回读具体日期与时间。")
    HintLine("到期提醒先短铃声，再读文案；排队或提交音频不等于设备已实际播报。")
    if (view.message.isNotEmpty()) HintLine(view.message)
    if (view.revision.isNotEmpty()) {
        HintLine("目录配置版本：${view.revision}；${view.skills.size} 个有效技能。")
        if (view.revision != form.revision) HintLine("此清单不是当前配置版本，请保存或重新读取后核对。")
        labeledSelect(
            label = "控制域筛选",
            selectedId = domainFilter,
            options = DOMAIN_FILTER_OPTIONS,
            onSelect = { domainFilter = it },
        )
        labeledField(
            label = "技能搜索",
            value = search,
            onValueChange = { search = it },
            placeholder = "输入技能名关键字",
        )
        val keyword = search.trim()
        for ((domainId, groupLabel) in DOMAIN_GROUPS) {
            if (domainFilter != "all" && domainFilter != domainId) continue
            val groupSkills = view.skills.filter {
                it.domain == domainId && (keyword.isEmpty() || it.name.contains(keyword, ignoreCase = true))
            }
            Text(groupLabel, style = Theme.typography.label)
            if (groupSkills.isEmpty()) {
                if (domainId == "iot") {
                    HintLine("IoT 域暂无技能。")
                } else {
                    HintLine(if (keyword.isEmpty()) "该控制域暂无技能。" else "该控制域没有匹配「$keyword」的技能。")
                }
            } else {
                for (skill in groupSkills) {
                    Text("${skill.name} · ${skill.summary}", style = Theme.typography.caption, color = Theme.colors.mutedForeground)
                }
            }
        }
    }
    if (view.capabilities.isNotEmpty()) {
        Text("已注册系统能力", style = Theme.typography.label)
        for (capability in view.capabilities) Text(capability, style = Theme.typography.caption, color = Theme.colors.mutedForeground)
        if (form.showAdvanced) Text(view.capabilitiesJson, style = Theme.typography.caption, color = Theme.colors.mutedForeground)
    }
}
