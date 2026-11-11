/**
 * `[asr]` 语音识别配置段（W5 / Config C2）。
 *
 * 字段清单与默认值：`plugins/asr/mod.rs:43-107`；advanced 标记：
 * `plugins/registry/projection.rs:21-25`（model / tokens / provider / use_itn /
 * num_threads）。`asr.engine` / `asr.language` / `asr.max_concurrent`
 * 不在清单 → 常显。
 *
 * `asr` 不在 `APP_SECTION_HOT`（plugins/registry/mod.rs:298-316），
 * `SectionHotHint` 取不到就返回 null，不自造档位文案。
 */

import type { ReactElement } from "react";
import { SectionCard } from "@/components/config/SectionCard";
import { SectionHotHint } from "@/components/config/SectionHotHint";
import { NumberField } from "@/components/form/NumberField";
import { SwitchField } from "@/components/form/SwitchField";
import { TextField } from "@/components/form/TextField";

export function AsrSection(): ReactElement {
  return (
    <div className="space-y-4">
      <SectionHotHint sectionId="asr" />
      <SectionCard
        title="ASR · 语音识别"
        description="实现选择、模型资产与识别调优；字段来自 [asr]（plugins/asr/mod.rs:43-67）。"
      >
        <div className="divide-y divide-border">
          <TextField
            path="asr.engine"
            label="实现 id"
            hint="空值由服务端按 sensevoice 兜底（plugins/asr/mod.rs:44-46、:71-73）；服务端返回未知值时界面不自动纠正。本轮不做下拉联动。"
            placeholder="sensevoice"
          />

          <TextField
            path="asr.model"
            label="SenseVoice INT8 模型路径"
            hint="默认 /data/models/SenseVoiceSmall/model.int8.onnx（plugins/asr/mod.rs:80-82）。"
            advanced
            placeholder="/data/models/SenseVoiceSmall/model.int8.onnx"
          />

          <TextField
            path="asr.tokens"
            label="词表文件路径"
            hint="默认 /data/models/SenseVoiceSmall/tokens.txt（plugins/asr/mod.rs:84-86）。"
            advanced
            placeholder="/data/models/SenseVoiceSmall/tokens.txt"
          />

          <TextField
            path="asr.language"
            label="识别语言"
            hint="默认 auto（plugins/asr/mod.rs:88-90）。"
            placeholder="auto"
          />

          <SwitchField
            path="asr.use_itn"
            label="启用逆文本正则（ITN）"
            hint="默认 true（plugins/asr/mod.rs:92-94）。"
            advanced
          />

          <NumberField
            path="asr.num_threads"
            label="推理线程数"
            hint="u32，默认 2（plugins/asr/mod.rs:96-98）。"
            advanced
            min={1}
            placeholder="2"
          />

          <TextField
            path="asr.provider"
            label="推理后端"
            hint="默认 cpu（plugins/asr/mod.rs:100-102）。"
            advanced
            placeholder="cpu"
          />

          <NumberField
            path="asr.max_concurrent"
            label="同时进行 ASR 推理路上限"
            hint="u32，默认 1（plugins/asr/mod.rs:65-66、:105-107）。服务端 normalize() 会 clamp ≥1（:74-78），故这里最小填 1；每路占用 num_threads 线程，多设备同时说话时在闸门处排队而不是压垮 CPU。"
            min={1}
            placeholder="1"
          />
        </div>
      </SectionCard>
    </div>
  );
}
