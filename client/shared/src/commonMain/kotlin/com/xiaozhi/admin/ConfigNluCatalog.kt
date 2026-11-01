package com.xiaozhi.admin

import androidx.compose.runtime.Composable
import com.gearui.components.button.Button
import com.gearui.components.button.ButtonType
import com.gearui.foundation.primitives.Text
import com.gearui.theme.Theme
import com.tencent.kuikly.core.nvi.serialization.json.JSONObject
import com.tencent.kuikly.core.pager.Pager
import com.xiaozhi.admin.kit.fieldLabelRow
import com.xiaozhi.admin.state.stateVar

data class NluCatalogView(
    val busy: Boolean = false,
    val message: String = "",
    val revision: String = "",
    val skills: List<String> = emptyList(),
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
        network(ctx).httpRequest("$base/api/skills/catalog", false, JSONObject(), ctx.adminRequestHeaders()) { data, ok, error, response ->
            if (ticket != generation || base != form.serverBase || !ctx.isAdminAccessCurrent(access, token) || form.editDeviceId.isNotEmpty()) return@httpRequest
            if (!ok || response.statusCode?.let { it !in 200..299 } == true || data == null || data.optString("state", "") != "ok" || data.optString("recognition", "") != "model_only") {
                view = view.copy(busy = false, message = "技能清单读取失败：${data?.optString("error", "").orEmpty().ifEmpty { error.ifEmpty { "服务端不支持模型技能目录（HTTP ${response.statusCode}）" } }}")
                return@httpRequest
            }
            val intents = data.optJSONObject("effective")?.optJSONArray("intents") ?: data.optJSONArray("intents")
            val skills = mutableListOf<String>()
            if (intents != null) for (i in 0 until intents.length()) {
                val skill = intents.optJSONObject(i) ?: continue
                val action = skill.optJSONObject("action")
                val plan = action?.optJSONObject("plan")
                val kind = when (action?.optString("kind", "")) {
                    "plan" -> "能力组合（${plan?.optJSONArray("conditions")?.length() ?: 0} 条条件，${plan?.optJSONArray("steps")?.length() ?: 0} 个步骤；${nluClockSummary(plan).ifEmpty { "系统能力" }}）"
                    "reply" -> "应答"
                    "end_session" -> "结束会话"
                    else -> "已配置动作"
                }
                val origin = when (skill.optString("source", "")) {
                    "command" -> "结束会话目录"
                    "default" -> "初始默认定义"
                    "custom" -> "自定义配置"
                    else -> "保存态定义"
                }
                skills.add("${skill.optString("name", "未命名技能")} · $kind · ${skill.optJSONArray("templates")?.length() ?: 0} 条训练例句 · $origin")
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
    fieldLabelRow("保存态技能与系统能力", help = "初始时钟包由服务端默认配置提供，包含倒计时、单次定时、固定间隔、每小时、每日、周一至周五、每周、每月、每年，以及查询和取消。实际可用技能以读取的保存态目录为准；显式清空不会在页面重建。能力组合只允许调用服务器注册的 Rust 系统能力，不支持任意脚本、命令或新编译代码。",
        trailing = { Button(text = "读取技能清单", onClick = { state.refresh(ctx, form) }, type = ButtonType.OUTLINE, loading = view.busy, disabled = view.busy || form.editDeviceId.isNotEmpty()) })
    HintLine("默认时钟按北京时间；工作日仅周一至周五，不自动处理法定节假日。每月31日或每年2月29日遇不存在的日期跳过，不夹到月底。")
    HintLine("支持今天、明天、后天、大后天及上午、下午、晚上。未说日期的钟点按下一次到达，创建结果回读具体日期与时间。")
    HintLine("到期提醒先短铃声，再读文案；排队或提交音频不等于设备已实际播报。")
    if (view.message.isNotEmpty()) HintLine(view.message)
    if (view.revision.isNotEmpty()) {
        HintLine("目录配置版本：${view.revision}；${view.skills.size} 个有效技能。")
        if (view.revision != form.revision) HintLine("此清单不是当前配置版本，请保存或重新读取后核对。")
        for (skill in view.skills) Text(skill, style = Theme.typography.caption, color = Theme.colors.mutedForeground)
    }
    if (view.capabilities.isNotEmpty()) {
        Text("已注册系统能力", style = Theme.typography.label)
        for (capability in view.capabilities) Text(capability, style = Theme.typography.caption, color = Theme.colors.mutedForeground)
        if (form.showAdvanced) Text(view.capabilitiesJson, style = Theme.typography.caption, color = Theme.colors.mutedForeground)
    }
}
