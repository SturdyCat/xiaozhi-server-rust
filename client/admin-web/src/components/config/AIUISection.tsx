/**
 * `[aiui]` 全链路模式（W4 / Config C1）。
 *
 * 字段清单：`plugins/aiui/mod.rs:26-59`；默认值：`:61-87`、
 * `client/shared/.../ConfigFormFill.kt:114-130`；
 * 密钥路径：`server/src/gateway/ws/config.rs:78-79`（GET 打码为 `has_api_key` /
 * `has_api_secret`，空串 = 保留原值）；
 * advanced：`plugins/registry/projection.rs:104-105`（`sn_prefix` / `pace_ms`）。
 *
 * `aiui` 属于 `fullchain` 能力，不在 `schema.app_sections` 里 —— `SectionHotHint`
 * 取不到就返回 null，绝不臆造第四种档位文案。
 */

import type { ReactElement } from "react";
import { SectionCard } from "@/components/config/SectionCard";
import { SectionHotHint } from "@/components/config/SectionHotHint";
import { FormNotice } from "@/components/form/FormNotice";
import { NumberField } from "@/components/form/NumberField";
import { SecretField } from "@/components/form/SecretField";
import { SwitchField } from "@/components/form/SwitchField";
import { TextAreaField } from "@/components/form/TextAreaField";
import { TextField } from "@/components/form/TextField";
import { useConfigDraft } from "@/context/ConfigDraftContext";

export function AIUISection(): ReactElement {
  const { getField } = useConfigDraft();
  const enabled = getField("aiui.enabled") === true;

  return (
    <div className="space-y-4">
      <SectionHotHint sectionId="aiui" />

      {enabled ? (
        <FormNotice tone="warning">
          当前使用云端托管流程：本地 ASR / NLU / LLM / TTS 增强不代表在云端链路执行。启用后本地 ASR / LLM / TTS 引擎闲置。
        </FormNotice>
      ) : null}

      <SectionCard
        title="AIUI 全链路模式"
        description="讯飞 AIUI 云端全链路接入。字段来自 [aiui]（plugins/aiui/mod.rs:26-59）。"
      >
        <div className="divide-y divide-border">
          <SwitchField
            path="aiui.enabled"
            label="启用 AIUI 全链路"
            hint="默认关闭；关闭时走本地级联流水线。"
          />

          <TextField
            path="aiui.appid"
            label="AppID"
            placeholder="讯飞平台 appid"
            hint="平台上 appid 下创建应用后填入。"
          />

          <SecretField
            path="aiui.api_key"
            hasPath="aiui.has_api_key"
            label="API Key"
          />

          <SecretField
            path="aiui.api_secret"
            hasPath="aiui.has_api_secret"
            label="API Secret"
          />

          <TextField
            path="aiui.scene"
            label="情景模式"
            placeholder="main_box"
            hint="默认 main_box（测试环境）；生产情景按平台实际配置（aiui/mod.rs:65-68）。"
          />

          <TextField
            path="aiui.sn_prefix"
            label="设备唯一标识前缀"
            hint="默认 xiaozhi；与设备 MAC 关联，用于云端个性化 / 上下文绑定。"
            advanced
            placeholder="xiaozhi"
          />

          <TextField
            path="aiui.voice"
            label="合成发音人"
            placeholder="x6_dongmanshaonv_pro"
            hint="默认 x6_dongmanshaonv_pro（极速超拟人目录见 AIUI 文档 3.9）。"
          />

          <NumberField
            path="aiui.speed"
            label="语速"
            hint="讯飞原生 0~100，50 为常速（aiui/mod.rs:46-47、:75-77）。"
            min={0}
            max={100}
            placeholder="50"
          />

          <NumberField
            path="aiui.volume"
            label="音量"
            hint="讯飞原生 0~100，50 为常速（aiui/mod.rs:48-49、:78-80）。"
            min={0}
            max={100}
            placeholder="50"
          />

          <NumberField
            path="aiui.pitch"
            label="音调"
            hint="讯飞原生 0~100，50 为常速（aiui/mod.rs:50-51、:81-83）。"
            min={0}
            max={100}
            placeholder="50"
          />

          <TextAreaField
            path="aiui.prompt"
            label="自定义人设 prompt"
            placeholder="可空；对应 nlp.prompt"
            hint="可空；对应 nlp.prompt（aiui/mod.rs:53-54）。"
          />

          <NumberField
            path="aiui.pace_ms"
            label="音频分帧推送间隔（ms）"
            hint="默认 10。云端流式 VAD 按实时节奏处理，瞬时灌入整段会被判 Silence；10ms/1280B ≈ 4 倍速实测可用，0 = 不等待（不推荐）（aiui/mod.rs:55-63）。"
            advanced
            min={0}
            placeholder="10"
          />
        </div>
      </SectionCard>
    </div>
  );
}
