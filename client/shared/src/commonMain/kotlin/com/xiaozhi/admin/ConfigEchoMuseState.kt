package com.xiaozhi.admin

import com.xiaozhi.admin.ui.*

import com.tencent.kuikly.core.base.PagerScope
import com.tencent.kuikly.core.nvi.serialization.json.JSONObject
import com.tencent.kuikly.core.reactive.handler.observable

/**
 * 「EchoMuse 设备入口」（`[echomuse]`）的表单状态。
 *
 * 为什么单独一个类（而不是继续往 `ConfigFormState` 里加字段）：`AGENTS.md` §5.10
 * 要求逻辑文件 ≤ 600 行；这里与 `WakeupSectionState` / `CommandSectionState` 同一做法——
 * `form.em.*` 成组出现，JSON 映射放在本文件的 `putJson` / `fill`，渲染见
 * `ConfigCards.echomuseConfigCard`。
 *
 * 语义（服务端为唯一权威，见 `server/src/app/echomuse/mod.rs::EchomuseConfig`）：
 * `enabled` 默认 **true**（开机即用：挂 em 路由并广播 mDNS）；显式关闭时零路由、零 mDNS，
 * 行为与没有这段完全一致；
 * `token` 是同网段设备的接入凭据（`X-Em-Token` 头，空 = 不校验）。
 * ⚠️ 该段属 **RestartOnly**：开启/关闭需重启服务端生效（生效档位提示由
 * `/api/config/schema` 的 sectionNote 下发，客户端不手写另一套）。
 */
class EchoMuseSectionState(private val scope: PagerScope) {

    var enabled by scope.observable("true")
    /// 接入令牌。服务端已打码（GET 只回 `has_token` presence），表单里永远没有明文；
    /// 留空保存 = 「不修改盘上已存值」（见 `putSecret` 与服务端 `SECRET_PATHS`）。
    var token by scope.observable("")
    /// 密钥 presence（服务端打码，只回 `has_token`）；为 true 时提示「已配置」。
    var tokenConfigured by scope.observable(false)
    /// 连接欢迎语（服务端 `[echomuse].welcome`，默认带使用提示；留空 = 不说）。
    /// RestartOnly：保存后需重启服务端生效（档位提示由 sectionNote 下发）。
    var welcome by scope.observable("")

    /**
     * 写入保存请求体（`[echomuse]` 整段回传）。
     *
     * 与其它段一样**必须整段回传**：服务端虽是部分更新语义，但前端不回传就等于
     * "这个段在 UI 里不可配置"。`token` 走 [putSecret]：留空 = 不发送该字段。
     */
    fun putJson(root: JSONObject) {
        root.put(
            "echomuse",
            JSONObject().apply {
                put("enabled", enabled.toBooleanStrictOrNull() ?: true)
                putSecret("token", token)
                put("welcome", welcome)
            },
        )
    }

    /** 用服务端 `/api/config` 回包填充（缺键保留当前值，不静默改成默认值）。 */
    fun fill(obj: JSONObject) {
        obj.optJSONObject("echomuse")?.let { e ->
            enabled = e.boolText("enabled", enabled, true)
            token = e.optString("token", token)
            tokenConfigured = e.optBoolean("has_token", token.isNotBlank())
            welcome = e.optString("welcome", welcome)
        }
    }
}
