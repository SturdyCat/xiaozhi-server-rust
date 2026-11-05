package com.xiaozhi.admin


import com.tencent.kuikly.core.base.PagerScope
import com.tencent.kuikly.core.nvi.serialization.json.JSONObject
import com.xiaozhi.admin.state.stateVar

/**
 * 「Tuya 接入」（`[tuya]`）的表单状态。
 *
 * 与 [EchoMuseSectionState] 同一做法：`form.tuya.*` 成组出现，JSON 映射放在本文件的
 * `putJson` / `fill`，渲染见 `ConfigTuyaCard.kt`。
 *
 * 字段边界（服务端为唯一权威，见 `server/src/device/tuya/config.rs::TuyaConfig`）：
 * - 这里只编辑 `enabled` 与 `sync_interval_secs` 两个部署级开关；
 * - **凭证（region/access_id/access_secret）不在配置表单里**：唯一录入路径是管理面
 *   `POST /api/tuya/credentials`（设备管理中心的齿轮弹层）。配置表单不回传这几个键，
 *   服务端按部分更新语义保留盘上原值，避免两套 UI 互相覆盖。
 * - `sync_interval_secs` 默认 86400（每日一次）；0 = 仅手动；>0 时服务端收敛到 ≥60s。
 * - 该段**全部热生效**（worker 每 tick 重读配置），档位提示由 `/api/config/schema` 的
 *   sectionNote 下发，客户端不手写另一套。
 */
class TuyaSectionState(private val scope: PagerScope) {

    var enabled by stateVar("true")

    /// 定时云同步周期（秒）。表单里是文本（Input 只吃文本），提交时转 long，非法值兜底默认。
    var syncIntervalSecs by stateVar("86400")

    /**
     * 写入保存请求体（只带本表单编辑的两个键；凭证键刻意不带，见类注释）。
     */
    fun putJson(root: JSONObject) {
        root.put(
            "tuya",
            JSONObject().apply {
                put("enabled", enabled.toBooleanStrictOrNull() ?: true)
                put("sync_interval_secs", syncIntervalSecs.toLongOrNull() ?: 86400L)
            },
        )
    }

    /** 用服务端 `/api/config` 回包填充（缺键保留当前值，不静默改成默认值）。 */
    fun fill(obj: JSONObject) {
        obj.optJSONObject("tuya")?.let { t ->
            enabled = t.boolText("enabled", enabled, true)
            syncIntervalSecs = t.intText("sync_interval_secs", syncIntervalSecs, 86400)
        }
    }
}
