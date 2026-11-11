/**
 * `[playback]` 播放场景（W4 / Config C1）。
 *
 * 它不是独立分区：`configSections.ts` 里没有 playback，按 Kuikly
 * `ConfigPlaybackCard.kt:18-23` 挂在「音频与播放」页末尾，一页只出现一个保存按钮
 * （保存按钮由 `ConfigPage` 的 `SaveBar` 统一提供）。
 *
 * 字段：`server/src/config/playback.rs:14-22`（默认 interactive / 5000 / 10000）；
 * hot：`plugins/registry/mod.rs:301`（NextSession）；
 * 未知模式只告警不纠正：`ConfigPlaybackCard.kt:35-38、:96-104`；
 * 两个蓄水字段仅在 announcement 时显示：`ConfigPlaybackCard.kt:69-70`，
 * 且标 advanced（`plugins/registry/projection.rs:124-125`）。
 */

import type { ReactElement } from "react";
import { SectionCard } from "@/components/config/SectionCard";
import { SectionHotHint } from "@/components/config/SectionHotHint";
import { NumberField } from "@/components/form/NumberField";
import { SelectField } from "@/components/form/SelectField";
import { useConfigDraft } from "@/context/ConfigDraftContext";

const PLAYBACK_MODE_OPTIONS = [
  { value: "interactive", label: "交互" },
  { value: "announcement", label: "播报" },
];

export function PlaybackSection(): ReactElement {
  const { getField } = useConfigDraft();
  const mode = getField("playback.mode");
  const announcement = mode === "announcement";

  return (
    <div className="space-y-4">
      <SectionHotHint sectionId="playback" />
      <SectionCard
        title="Playback · 播放场景"
        description="只调「开播时机」（交互继承现场 vs 播报先蓄水），不改协商档位；挂在「音频与播放」页末尾。"
      >
        <div className="divide-y divide-border">
          <SelectField
            path="playback.mode"
            label="场景策略"
            hint="交互：继承已有终端协商、起跑闸与节拍，保持 ESP 兼容。播报：只增加有上限的服务端起播蓄水，不会扩大固件播放队列，也不保证慢于实时的上游永不断音。"
            options={PLAYBACK_MODE_OPTIONS}
          />

          {announcement ? (
            <>
              <NumberField
                path="playback.announcement_start_ms"
                label="播报起播蓄水（ms）"
                hint="默认 5000。播报场景下开播前先攒多久的音频：攒得越多越抗上游变慢，但播报开始更晚；目标蓄水会按服务器水库容量自动收敛，短回复立即放行。服务端 normalize 上限 120000ms（playback.rs:29-30）。"
                advanced
                min={0}
                max={120000}
                placeholder="5000"
              />
              <NumberField
                path="playback.announcement_start_max_ms"
                label="播报最长等待（ms）"
                hint="默认 10000。攒不够也强制开播的兜底上限，避免上游慢时一直不播；服务端会 clamp 到不小于起播蓄水值且不超过 120000ms（playback.rs:31-33）。"
                advanced
                min={0}
                max={120000}
                placeholder="10000"
              />
            </>
          ) : null}
        </div>
      </SectionCard>
    </div>
  );
}
