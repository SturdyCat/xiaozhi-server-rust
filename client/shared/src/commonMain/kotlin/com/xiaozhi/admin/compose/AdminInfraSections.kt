package com.xiaozhi.admin.compose

import androidx.compose.runtime.getValue
import androidx.compose.runtime.setValue
import com.tencent.kuikly.core.nvi.serialization.json.JSONObject

/**
 * 「服务（Server）」「音频与播放（Audio / Playback）」「EchoMuse 接入」三个全局配置段。
 *
 * 每段一个 [AdminConfigDraft] 子类：字段声明 + 读写契约 + [rows] 行模型。
 * 界面由 `AdminSettingsSection` 统一渲染（`LazyColumn` 一字段一个 item），
 * 控件全部落到 Settings 磁贴。
 *
 * ## 与服务端契约
 * 字段名与兜底值逐字对齐旧实现（`ConfigFormFill.fill` 读、`ConfigFormDraft.buildConfigDraft` 写）。
 * 两个入口对同一字段必须给出同一默认值——否则会出现"旧版存 24000、新版存 0"的漂移。
 */

/** 服务段草稿：`server.*`。 */
internal class ServerDraft : AdminConfigDraft() {
    private var port by draftState("8000")
    private var token by draftState("")
    private var workerThreads by draftState("2")
    private var adminDir by draftState("")

    override fun fill(config: JSONObject) {
        config.optJSONObject("server")?.let { s ->
            port = DraftFields.int(s, "port", port, 8000)
            token = s.optString("expected_token", token)
            workerThreads = DraftFields.int(s, "worker_threads", workerThreads, 2)
            adminDir = DraftFields.text(s, "admin_dir", adminDir)
        }
    }

    override fun collect(): JSONObject = JSONObject().apply {
        put("server", JSONObject().apply {
            put("port", DraftFields.toInt(port, 8000))
            put("expected_token", token)
            put("worker_threads", DraftFields.toInt(workerThreads, 2))
            put("admin_dir", adminDir)
        })
    }

    override fun rows(): List<SettingRow> = listOf(
        SettingRow.Group("Server · 服务"),
        SettingRow.TextRow(
            title = "监听端口",
            value = port,
            placeholder = "8000",
            subtitle = "WebSocket 与 HTTP 共用端口：设备固件、管理台、测试台都按它连接。" +
                "改端口后必须同步设备端配置，否则设备连不上。",
            onChange = { port = it; markDirty() },
        ),
        SettingRow.TextRow(
            title = "接入令牌",
            value = token,
            placeholder = "留空则不校验",
            subtitle = "设备与测试台连接时校验的令牌（hello 帧携带）。留空 = 不校验，仅建议在内网使用。",
            onChange = { token = it; markDirty() },
        ),
        SettingRow.TextRow(
            title = "工作线程数",
            value = workerThreads,
            placeholder = "2",
            subtitle = "异步运行时的工作线程数；留空/0 = 按 CPU 核数自动决定。CPU 富余时调大能缓解多设备并发。",
            onChange = { workerThreads = it; markDirty() },
        ),
        SettingRow.TextRow(
            title = "管理页目录",
            value = adminDir,
            placeholder = "如 ./web",
            subtitle = "管理台前端静态资源目录（含 index.html 与 assets），由服务端同域托管；填错会 404。",
            onChange = { adminDir = it; markDirty() },
        ),
    )
}

/**
 * 音频段草稿：`audio.*` + `playback.*`。
 *
 * 两段在服务端是两个键，但界面上同属「音频与播放」一页（旧版 `audioConfigCard` 末尾直接
 * 调 `playbackConfigCard`），所以同一份草稿一起读写，避免同一页出现两个保存按钮。
 */
internal class AudioDraft : AdminConfigDraft() {
    private var downlinkSampleRate by draftState("24000")
    private var downlinkFrameMs by draftState("60")
    private var channels by draftState("1")
    private var binaryProtocolVersion by draftState("1")
    private var downlinkLeadMs by draftState("1000")
    private var downlinkQueuePackets by draftState("20")
    private var downlinkBufferMs by draftState("30000")
    private var downlinkBufferStartMs by draftState("1500")
    private var downlinkBufferStartMaxMs by draftState("4000")

    private var playbackMode by draftState("interactive")
    private var announcementStartMs by draftState("5000")
    private var announcementStartMaxMs by draftState("10000")

    override fun fill(config: JSONObject) {
        config.optJSONObject("audio")?.let { a ->
            downlinkSampleRate = DraftFields.int(a, "downlink_sample_rate", downlinkSampleRate, 24000)
            downlinkFrameMs = DraftFields.int(a, "downlink_frame_duration_ms", downlinkFrameMs, 60)
            channels = DraftFields.int(a, "channels", channels, 1)
            binaryProtocolVersion = DraftFields.int(a, "binary_protocol_version", binaryProtocolVersion, 1)
            downlinkLeadMs = DraftFields.int(a, "downlink_lead_ms", downlinkLeadMs, 1000)
            downlinkQueuePackets = DraftFields.int(a, "downlink_queue_packets", downlinkQueuePackets, 20)
            downlinkBufferMs = DraftFields.int(a, "downlink_buffer_ms", downlinkBufferMs, 30000)
            downlinkBufferStartMs = DraftFields.int(a, "downlink_buffer_start_ms", downlinkBufferStartMs, 1500)
            downlinkBufferStartMaxMs =
                DraftFields.int(a, "downlink_buffer_start_max_ms", downlinkBufferStartMaxMs, 4000)
        }
        config.optJSONObject("playback")?.let { p ->
            playbackMode = DraftFields.text(p, "mode", playbackMode, "interactive")
            announcementStartMs = DraftFields.int(p, "announcement_start_ms", announcementStartMs, 5000)
            announcementStartMaxMs = DraftFields.int(p, "announcement_start_max_ms", announcementStartMaxMs, 10000)
        }
    }

    override fun collect(): JSONObject = JSONObject().apply {
        put("audio", JSONObject().apply {
            put("downlink_sample_rate", DraftFields.toInt(downlinkSampleRate, 24000))
            put("downlink_frame_duration_ms", DraftFields.toInt(downlinkFrameMs, 60))
            put("channels", DraftFields.toInt(channels, 1))
            put("binary_protocol_version", DraftFields.toInt(binaryProtocolVersion, 1))
            put("downlink_lead_ms", DraftFields.toInt(downlinkLeadMs, 1000))
            put("downlink_queue_packets", DraftFields.toInt(downlinkQueuePackets, 20))
            put("downlink_buffer_ms", DraftFields.toInt(downlinkBufferMs, 30000))
            put("downlink_buffer_start_ms", DraftFields.toInt(downlinkBufferStartMs, 1500))
            put("downlink_buffer_start_max_ms", DraftFields.toInt(downlinkBufferStartMaxMs, 4000))
        })
        put("playback", JSONObject().apply {
            put("mode", playbackMode)
            put("announcement_start_ms", DraftFields.toInt(announcementStartMs, 5000))
            put("announcement_start_max_ms", DraftFields.toInt(announcementStartMaxMs, 10000))
        })
    }

    override fun rows(): List<SettingRow> = buildList {
        add(SettingRow.Group("Audio · 音频与播放"))
        add(
            SettingRow.TextRow(
                title = "下行采样率",
                value = downlinkSampleRate,
                placeholder = "24000",
                subtitle = "下发给设备的音频采样率（Hz），在 hello/OTA 协商时确认。必须与固件解码配置一致，" +
                    "不一致会变调或杂音，且只能重启生效。",
                onChange = { downlinkSampleRate = it; markDirty() },
            ),
        )
        add(
            SettingRow.TextRow(
                title = "下行帧长",
                value = downlinkFrameMs,
                placeholder = "60",
                subtitle = "每包音频的时长（ms）。帧短首包更快但包数更多，帧长更省带宽但延迟更高；改它要同步固件解码队列。",
                onChange = { downlinkFrameMs = it; markDirty() },
            ),
        )
        add(
            SettingRow.TextRow(
                title = "声道数",
                value = channels,
                placeholder = "1",
                subtitle = "通常 1（单声道）；固件按协商值解码，填错会只有噪声。",
                onChange = { channels = it; markDirty() },
            ),
        )
        add(
            SettingRow.TextRow(
                title = "二进制协议版本",
                value = binaryProtocolVersion,
                placeholder = "1",
                subtitle = "与固件协商的二进制帧协议版本。固件不认识时会退回 JSON 文本帧（更慢、更费流量）。",
                onChange = { binaryProtocolVersion = it; markDirty() },
            ),
        )
        add(
            SettingRow.TextRow(
                title = "下行提前量",
                value = downlinkLeadMs,
                placeholder = "1000",
                subtitle = "下发时间表比实时提前该时长，让设备解码队列常备存货，吸收网络抖动与上游停顿" +
                    "（长回复断音的主要来源）。默认 1000ms，接近设备端极限 1080ms。",
                onChange = { downlinkLeadMs = it; markDirty() },
            ),
        )
        add(
            SettingRow.TextRow(
                title = "设备解码队列",
                value = downlinkQueuePackets,
                placeholder = "20",
                subtitle = "设备端解码队列可容纳的包数，决定「提前量」能开多大（约 1.2s 上限）。" +
                    "改这里要同时改固件的 MAX_DECODE_PACKETS_IN_QUEUE。",
                onChange = { downlinkQueuePackets = it; markDirty() },
            ),
        )
        add(
            SettingRow.TextRow(
                title = "服务器水库容量",
                value = downlinkBufferMs,
                placeholder = "30000",
                subtitle = "服务器侧缓冲总时长上限（默认 30s ≈ 60KB/会话）。上游停顿不超过" +
                    "「水库水位 + 设备队列」时听感连续；上游持续慢于实时时缺口会累积，缓冲只能推迟断音。",
                onChange = { downlinkBufferMs = it; markDirty() },
            ),
        )
        add(
            SettingRow.TextRow(
                title = "起跑闸",
                value = downlinkBufferStartMs,
                placeholder = "1500",
                subtitle = "开始播放前先攒多久的音频（默认 1.5s）。攒够即开播；调大更抗抖动，但第一声更晚。",
                onChange = { downlinkBufferStartMs = it; markDirty() },
            ),
        )
        add(
            SettingRow.TextRow(
                title = "起跑闸最长等待",
                value = downlinkBufferStartMaxMs,
                placeholder = "4000",
                subtitle = "起跑闸的兜底上限：等这么久还没攒够也强制开播，避免上游慢时一直不出声。",
                onChange = { downlinkBufferStartMaxMs = it; markDirty() },
            ),
        )

        add(SettingRow.Group("Playback · 播放场景"))
        add(
            SettingRow.Segmented(
                title = "场景策略",
                options = PlaybackModes,
                selectedId = playbackMode,
                subtitle = "交互：继承终端协商、起跑闸与节拍，保持 ESP 兼容。" +
                    "播报：只增加有上限的服务端起播蓄水，不扩大固件播放队列。",
                onSelect = { playbackMode = it; markDirty() },
            ),
        )
        if (playbackMode == "announcement") {
            add(
                SettingRow.TextRow(
                    title = "播报起播蓄水",
                    value = announcementStartMs,
                    placeholder = "5000",
                    subtitle = "播报场景下开播前先攒多久的音频（默认 5s）：攒得越多越抗上游变慢，但开始更晚。",
                    onChange = { announcementStartMs = it; markDirty() },
                ),
            )
            add(
                SettingRow.TextRow(
                    title = "播报最长等待",
                    value = announcementStartMaxMs,
                    placeholder = "10000",
                    subtitle = "攒不够也强制开播的兜底上限（默认 10s），避免上游慢时一直不播。",
                    onChange = { announcementStartMaxMs = it; markDirty() },
                ),
            )
        }
    }
}

/**
 * 「EchoMuse 接入」段草稿：`echomuse.*`。
 *
 * ⚠️ 该段属 **RestartOnly**（开/关需重启服务端生效），档位提示由 `/api/config/schema`
 * 下发，客户端不手写另一套。
 */
internal class EchoMuseDraft : AdminConfigDraft() {
    private var enabled by draftBool(true)
    private var token by draftState("")
    private var tokenConfigured by draftBool(false)
    private var welcome by draftState("")

    override fun fill(config: JSONObject) {
        config.optJSONObject("echomuse")?.let { e ->
            enabled = e.optBoolean("enabled", enabled)
            token = e.optString("token", token)
            tokenConfigured = e.optBoolean("has_token", token.isNotBlank())
            welcome = e.optString("welcome", welcome)
        }
    }

    override fun collect(): JSONObject = JSONObject().apply {
        put("echomuse", JSONObject().apply {
            put("enabled", enabled)
            // 密钥沿用旧版 putSecret 规则：留空 = 不发送该字段（不覆盖盘上已存值）
            if (token.isNotEmpty()) put("token", token)
            put("welcome", welcome)
        })
    }

    override fun rows(): List<SettingRow> = buildList {
        add(SettingRow.Group("EchoMuse · 设备接入"))
        add(
            SettingRow.Switch(
                title = "启用 EchoMuse 设备入口",
                checked = enabled,
                subtitle = "启用后挂载 em 路由并与同网段设备建立连接；关闭时 EchoMuse 页只显示「未启用」提示，" +
                    "不挂 em 路由、不广播 mDNS。重启生效。",
                onChange = { enabled = it; markDirty() },
            ),
        )
        add(
            SettingRow.TextRow(
                title = "接入令牌",
                value = token,
                placeholder = if (tokenConfigured) "已配置（留空则不修改）" else "未配置",
                subtitle = "同网段设备接入凭据（X-Em-Token）。留空 = 不校验；留空保存不会覆盖已设置的值。",
                onChange = { token = it; markDirty() },
            ),
        )
        add(
            SettingRow.TextRow(
                title = "连接欢迎语",
                value = welcome,
                placeholder = "留空则不说",
                subtitle = "设备接入后立刻说一次（默认含使用提示）。留空 = 不说；改后需重启生效。",
                onChange = { welcome = it; markDirty() },
            ),
        )
        if (!enabled) {
            add(
                SettingRow.Info(
                    title = "当前状态",
                    value = "未启用",
                    subtitle = "EchoMuse 页只显示「未启用」提示，不挂 em 路由、不广播 mDNS。",
                ),
            )
        }
    }
}

/** 播放场景选项（服务端 `playback.mode` 的合法值）。 */
internal val PlaybackModes: List<TileOption> = listOf(
    "interactive" to "交互",
    "announcement" to "播报",
)
