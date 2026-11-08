package com.xiaozhi.admin

import androidx.compose.runtime.Composable
import com.gearui.components.button.Button
import com.gearui.components.button.ButtonType
import com.xiaozhi.admin.kit.HintLine
import com.xiaozhi.admin.kit.LabeledCard
import com.xiaozhi.admin.kit.actionRow
import com.xiaozhi.admin.kit.labeledIntField
import com.xiaozhi.admin.kit.switchRow

/**
 * 「Tuya 接入」section（`tuya`，`[tuya]` 段）—— 与 [EchoMuseSection] 并列的设备接入配置。
 *
 * 只放两个部署级开关（启用 / 同步周期）；**云凭证不在这里**——唯一录入路径是
 * `POST /api/tuya/credentials`（设备管理中心的齿轮弹层），配置表单刻意不回传凭证键，
 * 两套 UI 不互相覆盖（见 [TuyaSectionState] 类注释）。
 *
 * 语义（服务端为唯一权威）：`enabled` 默认 true；该段**全部热生效**（保存即生效，
 * 无需重启），档位提示由 `/api/config/schema` 的 sectionNote 下发。
 */
@Composable
fun TuyaSection(form: ConfigFormState) {
    LabeledCard(title = "Tuya · 设备接入") {
        // 整段档位提示（服务端为唯一权威；tuya 全字段热生效）
        val note = form.schema.sectionNote("tuya")
        if (note.isNotEmpty()) HintLine(note)

        switchRow(
            label = "启用涂鸦设备",
            checked = form.tuya.enabled == "true",
            onToggle = { form.toggleStringBool(form.tuya::enabled) },
            help = "启用后开放涂鸦云目录同步与 /api/tuya/* 管理面；关闭即停用全部同步与管理面" +
                "（保存即生效，无需重启）。局域网控制不依赖云凭证。",
        )
        labeledIntField(
            label = "自动同步间隔（秒）",
            value = form.tuya.syncIntervalSecs,
            onValueChange = {
                form.tuya.syncIntervalSecs = it
                form.markDirty()
            },
            help = "定时拉取涂鸦云设备目录的周期，默认 86400（每天一次）；0 = 仅手动同步，" +
                "非零最小 60 秒（服务端会自动收敛）。",
        )
        HintLine("云凭证（区域 / access_id / access_secret）在「设备管理」页右上角齿轮里录入，这里不重复编辑。")
        if (form.tuya.enabled != "true") {
            HintLine("未启用：/api/tuya/* 全部 404，设备管理页的涂鸦分区只显示未启用提示。")
        }
        // 多入口：配置 → 设备管理中心（侧栏「设备管理」）。单页 config 模式无侧栏，槽位为 null 不渲染。
        form.onOpenDeviceHub?.let { openHub ->
            actionRow {
                Button(
                    text = "管理设备",
                    type = ButtonType.OUTLINE,
                    onClick = openHub,
                )
            }
        }
    }
}
