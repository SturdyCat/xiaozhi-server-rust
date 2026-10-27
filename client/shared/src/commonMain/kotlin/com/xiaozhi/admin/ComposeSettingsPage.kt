package com.xiaozhi.admin

import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import com.alorma.compose.settings.ui.SettingsGroup
import com.alorma.compose.settings.ui.SettingsSlider
import com.alorma.compose.settings.ui.SettingsSwitch
import com.tencent.kuikly.compose.ComposeContainer
import com.tencent.kuikly.compose.foundation.layout.Arrangement
import com.tencent.kuikly.compose.foundation.layout.PaddingValues
import com.tencent.kuikly.compose.foundation.layout.Row
import com.tencent.kuikly.compose.foundation.layout.fillMaxSize
import com.tencent.kuikly.compose.foundation.lazy.LazyColumn
import com.tencent.kuikly.compose.material3.Button
import com.tencent.kuikly.compose.material3.Text
import com.tencent.kuikly.compose.setContent
import com.tencent.kuikly.compose.ui.Modifier
import com.tencent.kuikly.compose.ui.unit.dp
import com.tencent.kuikly.core.annotations.Page
import com.tencent.kuikly.core.nvi.serialization.json.JSONObject
import com.tencent.kuikly.core.pager.Pager
import com.tencent.kuiklyx.viewmodel.ViewModel
import com.tencent.kuiklyx.viewmodel.viewModelStore

/**
 * Compose-Settings 试点页（`page_name=compose_settings`）：**接真实配置**的 AIUI 段。
 *
 * ## 为什么是整页
 * [ComposeContainer] 继承 `Pager`，`setContent` 是该 Pager 的唯一根内容；框架没有
 * 「在传统 DSL 布局里嵌一块 Compose」的公开 API（`ComposeSceneMediator` 虽是 public，
 * 但要自行接管触摸/布局/帧驱动，属未公开契约）。所以 Compose 化的最小单位是**一整页**，
 * 本页也因此没有直接替换 `ui/Form`、`ui/Choice` 那批 DSL 控件（详见 `AGENTS.md` 同名条目）。
 *
 * ## 读写口径（与 DSL 版配置页同一套服务端契约）
 * - **读**：`GET /api/config`，只取 `aiui` 段与 `revision`（乐观并发指纹）。
 * - **写**：`POST /api/config`（服务端 `merge_client_patch` 是**部分更新**语义：未提到的
 *   段/字段保持现盘值），因此这里**只提交 `aiui` 段的 5 个字段**，不碰 `api_key` /
 *   `api_secret`——GET 回包里的密钥是打码的，原样回传会把真实密钥覆盖掉。
 * - 必须显式带 `Content-Type: application/json`：Kuikly 的 httpRequest 在非 JSON
 *   Content-Type 下会把 body 编成 form-urlencoded，服务端直接 415（DSL 版踩过）。
 * - 带回 `expected_revision`：别人改过时服务端回 409 而不是覆盖。
 */
@Page("compose_settings")
internal class ComposeSettingsPage : ComposeContainer() {

    private val vm: SettingsPilotViewModel by viewModelStore { pagerId ->
        SettingsPilotViewModel(pagerId)
    }

    override fun willInit() {
        super.willInit()
        val page = this
        setContent {
            // 进页即拉配置；Kuikly 的网络回调由框架投递回 context 线程，
            // 因此这里直接写 Compose 的 snapshot state 是安全的。
            LaunchedEffect(Unit) { vm.load(page) }
            SettingsPilotScreen(
                vm = vm,
                onSave = { vm.save(page) },
                onReload = { vm.load(page) },
            )
        }
    }
}

/**
 * AIUI 段的状态（`[aiui]`）：数值项与 DSL 版 ConfigFormState 的默认值保持一致
 * （speed/volume/pitch=50、pace_ms=10），避免两个入口对同一字段给出不同兜底。
 */
internal class SettingsPilotViewModel(override val pagerId: String) : ViewModel() {

    var loading by mutableStateOf(false)
    var saving by mutableStateOf(false)
    var message by mutableStateOf("")
    var messageIsError by mutableStateOf(false)

    var enabled by mutableStateOf(false)
    var speed by mutableStateOf(50f)
    var volume by mutableStateOf(50f)
    var pitch by mutableStateOf(50f)
    var paceMs by mutableStateOf(10f)

    /** 上次 GET 的配置指纹；保存时作为 `expected_revision` 回传。 */
    private var revision = ""

    fun load(ctx: Pager) {
        if (loading) return
        loading = true
        getJson(ctx, "${pagerBaseUrl(ctx)}/api/config") { ok, data, reason ->
            loading = false
            if (!ok) {
                message = if (reason.isEmpty()) "加载失败" else "加载失败：$reason"
                messageIsError = true
                return@getJson
            }
            revision = data.optString("revision", revision)
            data.optJSONObject("aiui")?.let { a ->
                enabled = a.optBoolean("enabled", enabled)
                speed = a.optInt("speed", speed.toInt()).toFloat()
                volume = a.optInt("volume", volume.toInt()).toFloat()
                pitch = a.optInt("pitch", pitch.toInt()).toFloat()
                paceMs = a.optInt("pace_ms", paceMs.toInt()).toFloat()
            }
            message = "已加载 AIUI 配置"
            messageIsError = false
        }
    }

    fun save(ctx: Pager) {
        if (saving) return
        saving = true
        val body = JSONObject().apply {
            put("aiui", JSONObject().apply {
                put("enabled", enabled)
                put("speed", speed.toInt())
                put("volume", volume.toInt())
                put("pitch", pitch.toInt())
                put("pace_ms", paceMs.toInt())
            })
            if (revision.isNotEmpty()) put("expected_revision", revision)
        }
        val headers = JSONObject().apply { put("Content-Type", "application/json") }
        network(ctx).httpRequest(
            "${pagerBaseUrl(ctx)}/api/config", true, body, headers,
        ) { data, success, errorMsg, resp ->
            saving = false
            val code = resp.statusCode
            if (success && (code == null || code in 200..299)) {
                revision = data?.optString("revision", revision) ?: revision
                val serverMsg = data?.optString("message", "") ?: ""
                message = if (serverMsg.isEmpty()) "已保存" else serverMsg
                messageIsError = false
            } else if (code == 409) {
                message = "配置已被其他窗口修改，请重新加载后再改"
                messageIsError = true
            } else {
                message = "保存失败：${if (errorMsg.isNotEmpty()) errorMsg else "HTTP $code"}"
                messageIsError = true
            }
        }
    }

    override fun onCleared() {
        // 本页只有一次性 GET/POST，没有长连接或轮询，无额外清理。
    }
}

@Composable
private fun SettingsPilotScreen(
    vm: SettingsPilotViewModel,
    onSave: () -> Unit,
    onReload: () -> Unit,
) {
    // Kuikly Compose 2.29.2 没有实现 Modifier.verticalScroll（制品里只有语义名），
    // 滚动必须用 LazyColumn——也更贴合设置页的「磁贴列表」结构。
    LazyColumn(
        modifier = Modifier.fillMaxSize(),
        contentPadding = PaddingValues(16.dp),
    ) {
        item {
            Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                Button(onClick = onReload, enabled = !vm.loading && !vm.saving) {
                    Text(if (vm.loading) "加载中…" else "重新加载")
                }
                Button(onClick = onSave, enabled = !vm.saving && !vm.loading) {
                    Text(if (vm.saving) "保存中…" else "保存")
                }
            }
        }
        if (vm.message.isNotEmpty()) {
            item { Text(vm.message) }
        }
        item {
            SettingsGroup(title = { Text("AIUI 全链路模式") }) {
                SettingsSwitch(
                    state = vm.enabled,
                    title = { Text("启用全链路模式") },
                    subtitle = { Text("关闭后走本地 VAD/ASR/LLM/TTS 流程") },
                    onCheckedChange = { vm.enabled = it },
                )
                SettingsSlider(
                    title = { Text("语速") },
                    value = vm.speed,
                    onValueChange = { vm.speed = it },
                    valueRange = 0f..100f,
                    steps = 99,
                    enabled = vm.enabled,
                    subtitle = { Text("${vm.speed.toInt()}") },
                )
                SettingsSlider(
                    title = { Text("音量") },
                    value = vm.volume,
                    onValueChange = { vm.volume = it },
                    valueRange = 0f..100f,
                    steps = 99,
                    enabled = vm.enabled,
                    subtitle = { Text("${vm.volume.toInt()}") },
                )
                SettingsSlider(
                    title = { Text("音调") },
                    value = vm.pitch,
                    onValueChange = { vm.pitch = it },
                    valueRange = 0f..100f,
                    steps = 99,
                    enabled = vm.enabled,
                    subtitle = { Text("${vm.pitch.toInt()}") },
                )
                SettingsSlider(
                    title = { Text("句间停顿") },
                    value = vm.paceMs,
                    onValueChange = { vm.paceMs = it },
                    valueRange = 0f..100f,
                    steps = 99,
                    enabled = vm.enabled,
                    subtitle = { Text("${vm.paceMs.toInt()} ms") },
                )
            }
        }
        item {
            Text("本页只提交 aiui 段的 enabled/speed/volume/pitch/pace_ms，其余字段由服务端部分更新保留。")
        }
    }
}
