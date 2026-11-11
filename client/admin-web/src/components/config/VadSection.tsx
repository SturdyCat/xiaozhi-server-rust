/**
 * `[vad]` 语音分段配置段（W5 / Config C2）。
 *
 * 字段清单与默认值：`plugins/vad/mod.rs:44-82`；advanced 标记：
 * `plugins/registry/projection.rs:28-30`（vad.model / vad.min_speech_duration /
 * vad.min_silence_duration）。`vad.engine` 与 `vad.threshold` 不在清单 → 常显。
 *
 * 三个 f32 字段走 `NumberField integer={false}` 小数通道；服务端源码未给
 * threshold / 时长的取值上下限，因此这里不自造 min/max。
 * `vad` 不在 `APP_SECTION_HOT`（plugins/registry/mod.rs:298-316），
 * `SectionHotHint` 取不到就返回 null，不自造档位文案。
 */

import type { ReactElement } from "react";
import { SectionCard } from "@/components/config/SectionCard";
import { SectionHotHint } from "@/components/config/SectionHotHint";
import { NumberField } from "@/components/form/NumberField";
import { TextField } from "@/components/form/TextField";

export function VadSection(): ReactElement {
  return (
    <div className="space-y-4">
      <SectionHotHint sectionId="vad" />
      <SectionCard
        title="VAD · 语音分段"
        description="实现选择、触发阈值与切句时序；字段来自 [vad]（plugins/vad/mod.rs:44-82）。"
      >
        <div className="divide-y divide-border">
          <TextField
            path="vad.engine"
            label="实现 id"
            hint="空值由服务端按 silero 兜底（plugins/vad/mod.rs:45、:60-63）；服务端返回未知值时界面不自动纠正。本轮不做下拉联动。"
            placeholder="silero"
          />

          <TextField
            path="vad.model"
            label="Silero VAD 模型路径"
            hint="默认 /data/models/silero_vad.onnx（plugins/vad/mod.rs:68-70）；为空视为未配置。"
            advanced
            placeholder="/data/models/silero_vad.onnx"
          />

          <NumberField
            path="vad.threshold"
            label="语音触发阈值"
            hint="f32，默认 0.5（plugins/vad/mod.rs:72-74）。GET 回包经 clean_widened_f32 收成短小数，此处按小数编辑。"
            integer={false}
            placeholder="0.5"
          />

          <NumberField
            path="vad.min_silence_duration"
            label="最短静音时长（秒）"
            hint="f32，默认 0.25（plugins/vad/mod.rs:76-78）；高级切句时序微调。"
            advanced
            integer={false}
            placeholder="0.25"
          />

          <NumberField
            path="vad.min_speech_duration"
            label="最短语音时长（秒）"
            hint="f32，默认 0.25（plugins/vad/mod.rs:80-82）；高级切句时序微调。"
            advanced
            integer={false}
            placeholder="0.25"
          />
        </div>
      </SectionCard>
    </div>
  );
}
