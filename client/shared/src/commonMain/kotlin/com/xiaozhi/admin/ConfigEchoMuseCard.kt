package com.xiaozhi.admin

import com.tencent.kuikly.core.base.ViewContainer
import com.tencent.kuikly.core.directives.vif
import com.tencent.kuikly.core.views.Text

/**
 * 「EchoMuse」配置卡（`[echomuse]` 设备入口，见 [EchoMuseSectionState]）。
 *
 * 独立成文件的原因与 `ConfigWakeupCard.kt` 相同：`ConfigCards.kt` 已超 600 行。
 * 档位提示来自 `/api/config/schema`（服务端 `[echomuse]` 登记为 RestartOnly →
 * sectionNote 会说明「重启生效」），客户端不手写另一套。
 */
fun ViewContainer<*, *>.echomuseConfigCard(form: ConfigFormState) {
    groupedCard("EchoMuse") {
        // 整段档位提示（服务端为唯一权威；echomuse 属 RestartOnly → 提示重启生效）
        vif({ form.schema.sectionNote("echomuse").isNotEmpty() }) {
            textNote(form.schema.sectionNote("echomuse"), micro = true)
        }
        switchRow("启用 EchoMuse 设备入口", form.em.enabled == "true") {
            form.toggleStringBool(form.em::enabled)
        }
        labeledField(
            "接入令牌",
            { form.em.token },
            { form.em.token = it; form.markDirty() },
            secretHint(form.em.tokenConfigured),
        )
        Text {
            attr {
                fontSize(AdminType.caption)
                color(AdminColors.textTertiary)
                text("同网段设备接入凭据（X-Em-Token），留空则不校验；留空保存不覆盖已设值。")
            }
        }
        vif({ form.em.enabled != "true" }) {
            Text {
                attr {
                    fontSize(AdminType.caption)
                    color(AdminColors.textTertiary)
                    text("未启用：EchoMuse 页只显示「未启用」提示，不挂 em 路由、不广播 mDNS。")
                }
            }
        }
    }
}
