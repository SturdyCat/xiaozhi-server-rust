/**
 * `[audio]` 配置段（W4 / Config C1）：9 个整数。
 *
 * 字段清单：`server/src/config.rs:224-296`；默认值：`:540-576`、
 * `client/shared/.../ConfigFormFill.kt:30-38`。
 *
 * ⚠️ advanced 门控严格按服务端 `ADVANCED_APP_KEYS`
 * （`plugins/registry/projection.rs:112-126`）逐字段对照：
 * - `audio.downlink_sample_rate` **在**清单里（:115），标 advanced；
 * - `audio.channels` **不在**清单里，简洁模式下仍可见；
 * - `playback.mode` 也在清单里（:123），但它挂在下方 `PlaybackSection` 且是
 *   另两个字段的显示条件，按 Kuikly `ConfigPlaybackCard.kt` 的既有行为保持常显。
 */

import type { ReactElement } from "react";
import { SectionCard } from "@/components/config/SectionCard";
import { SectionHotHint } from "@/components/config/SectionHotHint";
import { FormNotice } from "@/components/form/FormNotice";
import { NumberField } from "@/components/form/NumberField";
import { SelectField } from "@/components/form/SelectField";
import type { JsonValue } from "@/lib/configApi";

/** 配置里采样率 / 协议版本是整数，SelectField 选项用字符串表达，提交前转回 number。 */
const toInteger = (raw: string): JsonValue => Number(raw);

const SAMPLE_RATE_OPTIONS = [
  { value: "16000", label: "16000 Hz" },
  { value: "24000", label: "24000 Hz（推荐）" },
  { value: "48000", label: "48000 Hz" },
];

const BINARY_VERSION_OPTIONS = [
  { value: "1", label: "v1（默认）" },
  { value: "2", label: "v2" },
  { value: "3", label: "v3" },
];

export function AudioSection(): ReactElement {
  return (
    <div className="space-y-4">
      <SectionHotHint sectionId="audio" />
      <SectionCard
        title="音频与播放 · 下行参数"
        description="服务器下行（TTS）协商与缓冲参数；字段来自 [audio]（server/src/config.rs:224-296）。"
      >
        <FormNotice tone="warning">
          设备解码队列约 1.2s（20 包 × 60ms 帧）是硬上限，超出队容量的下行帧会被固件静默丢弃；服务器侧水库用服务器内存换下行连续性，起跑闸的代价是首帧延迟。
        </FormNotice>

        <div className="divide-y divide-border">
          <SelectField
            path="audio.downlink_sample_rate"
            label="下行采样率"
            hint="只允许 16000 / 24000 / 48000（config.rs:226-231），非法值由 AudioConfig::normalize 回落 24k；推荐 24000。默认 24000。"
            advanced
            options={SAMPLE_RATE_OPTIONS}
            coerce={toInteger}
          />

          <NumberField
            path="audio.downlink_frame_duration_ms"
            label="下行帧长（ms）"
            hint="默认 60（config.rs:543-545）；设备队列能装多少音频取决于协商帧长。"
            advanced
            min={1}
            placeholder="60"
          />

          <NumberField
            path="audio.channels"
            label="声道数"
            hint="默认 1（config.rs:571-573）。该字段不在 ADVANCED_APP_KEYS，简洁模式下仍可见。"
            min={1}
            placeholder="1"
          />

          <SelectField
            path="audio.binary_protocol_version"
            label="下行二进制协议版本"
            hint="只允许 1 / 2 / 3（config.rs:237-239），默认 1（:574-576）。"
            advanced
            options={BINARY_VERSION_OPTIONS}
            coerce={toInteger}
          />

          <NumberField
            path="audio.downlink_lead_ms"
            label="下行抖动缓冲提前量（ms）"
            hint="默认 1000。设备 ESP 固件 20 包队列（60ms 帧）只装得下 ~1.1s，安全上限 =(队列包数−2)×帧长；提前量越大，打断后的「停口」越晚（config.rs:240-249）。"
            advanced
            min={0}
            placeholder="1000"
          />

          <NumberField
            path="audio.downlink_queue_packets"
            label="设备解码队列包数"
            hint="默认 20（官方固件 MAX_DECODE_PACKETS_IN_QUEUE）。单位是「包」不是毫秒：20 包 × 60ms 帧 ≈ 1200ms；只改服务端不改固件无效，超出队容量的下行帧会被固件静默丢弃（config.rs:251-262、:552-555）。"
            advanced
            min={1}
            placeholder="20"
          />

          <NumberField
            path="audio.downlink_buffer_ms"
            label="服务器侧下行水库容量（ms）"
            hint="默认 30000。用服务器内存换下行连续性（已编码帧约 2KB/s，30s ≈ 60KB/会话）；解决的是上游「间歇」停顿，不是「持续」慢于实时；0 = 关闭水库（config.rs:264-284、:559-561）。"
            advanced
            min={0}
            placeholder="30000"
          />

          <NumberField
            path="audio.downlink_buffer_start_ms"
            label="起跑闸等待（ms）"
            hint="默认 1500。本轮首帧下发前先等水库攒够音频再开播，代价是一次性起跑等待（首帧延迟）；短回复生产端已收工时立即放行，0 = 关闭（config.rs:286-292、:564-566）。"
            advanced
            min={0}
            placeholder="1500"
          />

          <NumberField
            path="audio.downlink_buffer_start_max_ms"
            label="起跑闸最长等待（ms）"
            hint="默认 4000。上游持续慢于实时时到点即以当前水位起跑，避免首帧被无限期拖住（config.rs:294-296、:568-570）。"
            advanced
            min={0}
            placeholder="4000"
          />
        </div>
      </SectionCard>
    </div>
  );
}
