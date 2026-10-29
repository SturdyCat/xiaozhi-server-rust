package com.xiaozhi.admin

import androidx.compose.runtime.Composable
import com.xiaozhi.admin.kit.labeledField
import com.xiaozhi.admin.kit.switchRow

/**
 * 「EchoMuse 设备接入」section（`echomuse`，`[echomuse]` 段）—— Compose 版。
 *
 * ⚠️ 与侧边栏「EchoMuse」菜单**不是同一处**：那边是设备运行面（设备列表 / 批准 / WiFi 下发，
 * 见 `EchoMuseState`，属另一任务），这里只有三项**配置字段**（开关 / 接入令牌 / 欢迎语）。
 *
 * 语义（服务端为唯一权威）：`enabled` 默认 true（开机即用：挂 em 路由并广播 mDNS）；
 * 显式关闭时零路由、零 mDNS，行为与没有这段完全一致。该段属 **RestartOnly**，
 * 档位提示由 `/api/config/schema` 的 sectionNote 下发，客户端不手写另一套。
 */
@Composable
fun EchoMuseSection(form: ConfigFormState) {
    LabeledCard(title = "EchoMuse · 设备接入") {
        // 整段档位提示（服务端为唯一权威；echomuse 属 RestartOnly → 会说明"重启生效"）
        val note = form.schema.sectionNote("echomuse")
        if (note.isNotEmpty()) HintLine(note)

        switchRow(
            label = "启用 EchoMuse 设备入口",
            checked = form.em.enabled == "true",
            onToggle = { form.toggleStringBool(form.em::enabled) },
            help = "启用后挂载 em 路由并与同网段设备建立连接；关闭时 EchoMuse 页只显示「未启用」提示，" +
                "不挂 em 路由、不广播 mDNS。",
        )
        labeledField(
            label = "接入令牌",
            value = form.em.token,
            onValueChange = {
                form.em.token = it
                form.markDirty()
            },
            note = secretHint(form.em.tokenConfigured),
            help = "同网段设备接入凭据（X-Em-Token）。留空 = 不校验；留空保存不会覆盖已设置的值。",
        )
        labeledField(
            label = "连接欢迎语",
            value = form.em.welcome,
            onValueChange = {
                form.em.welcome = it
                form.markDirty()
            },
            help = "设备接入后立刻说一次（默认含使用提示）。留空 = 不说；改后需重启生效。",
        )
        if (form.em.enabled != "true") {
            HintLine("未启用：EchoMuse 页只显示「未启用」提示，不挂 em 路由、不广播 mDNS。")
        }
    }
}
