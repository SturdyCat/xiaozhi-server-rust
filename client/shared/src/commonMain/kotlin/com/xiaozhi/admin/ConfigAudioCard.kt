package com.xiaozhi.admin

import androidx.compose.runtime.Composable
import com.xiaozhi.admin.kit.labeledIntField

/**
 * 「音频与播放」配置卡（`[audio]`）—— Compose 版。
 *
 * 下行参数与 `hello`/OTA 协商**强绑定**：改动均需重启 server 生效，设备侧需固件同步调整。
 * 这句话必须留在卡片顶部（旧实现是第一条 `textNote`）——它是这张卡最重要的一条使用约束。
 *
 * ## 字段（全部是整数，走 `labeledIntField` 的 `INT_INPUT_FILTER`）
 *
 * | 字段 | 含义 |
 * | --- | --- |
 * | 下行采样率 / 下行帧长 / 声道数 | 下行音频格式三件套，必须与设备固件一致 |
 * | 二进制协议版本 | 协议版本号，两侧不一致会直接协商失败 |
 * | 下行提前量 | 预送多少毫秒音频，吸收网络抖动 |
 * | 设备解码队列 | 设备侧可缓冲的包数上限 |
 * | 服务器水库容量 / 起跑闸 / 起跑闸最长等待 | 服务端蓄水与开播时机 |
 *
 * `server` / `aiui` 两段属于 T08（全局配置），仍留在 `ConfigInfrastructureCards.kt` 里未动。
 */
@Composable
fun AudioConfigCard(form: ConfigFormState) {
    LabeledCard(title = "Audio · 音频与播放") {
        // 整段档位提示（来自 /api/config/schema 的 app_sections，服务端为唯一权威）
        val note = form.schema.sectionNote("audio")
        if (note.isNotEmpty()) HintLine(note)
        HintLine("下行参数与 hello/OTA 协商强绑定，改动均需重启 server 生效；设备侧需固件同步调整。")
        labeledIntField(
            label = "下行采样率",
            value = form.downlinkSampleRate,
            onValueChange = {
                form.downlinkSampleRate = it
                form.markDirty()
            },
        )
        labeledIntField(
            label = "下行帧长",
            value = form.downlinkFrameMs,
            onValueChange = {
                form.downlinkFrameMs = it
                form.markDirty()
            },
        )
        labeledIntField(
            label = "声道数",
            value = form.channels,
            onValueChange = {
                form.channels = it
                form.markDirty()
            },
        )
        labeledIntField(
            label = "二进制协议版本",
            value = form.binaryProtocolVersion,
            onValueChange = {
                form.binaryProtocolVersion = it
                form.markDirty()
            },
        )
        AdvancedToggleRow(form)
        if (form.showAdvanced) {
            labeledIntField(
                label = "下行提前量",
                value = form.downlinkLeadMs,
                onValueChange = {
                    form.downlinkLeadMs = it
                    form.markDirty()
                },
                help = "预送多少毫秒音频给设备，吸收网络抖动；调大更抗抖动但整体延迟更高。",
            )
            labeledIntField(
                label = "设备解码队列",
                value = form.downlinkQueuePackets,
                onValueChange = {
                    form.downlinkQueuePackets = it
                    form.markDirty()
                },
                help = "设备侧可缓冲的音频包数上限；超过会丢弃旧包。",
            )
            labeledIntField(
                label = "服务器水库容量",
                value = form.downlinkBufferMs,
                onValueChange = {
                    form.downlinkBufferMs = it
                    form.markDirty()
                },
                help = "服务端最多蓄多久的音频；蓄水越多越抗上游变慢，但首字延迟更高。",
            )
            labeledIntField(
                label = "起跑闸",
                value = form.downlinkBufferStartMs,
                onValueChange = {
                    form.downlinkBufferStartMs = it
                    form.markDirty()
                },
                help = "开播前先攒多久音频；攒够即开播，避免一开口就断。",
            )
            labeledIntField(
                label = "起跑闸最长等待",
                value = form.downlinkBufferStartMaxMs,
                onValueChange = {
                    form.downlinkBufferStartMaxMs = it
                    form.markDirty()
                },
                help = "攒不够也强制开播的兜底上限，避免上游慢时一直不播。",
            )
        }
    }
    // 播放场景与下行参数同页（旧 `audioConfigCard` 末尾直接调 `playbackConfigCard`）：
    // 两张卡都是 `[audio]`/`[playback]` 两个键各自的字段，但同属「音频与播放」一页。
    PlaybackConfigCard(form)
}
