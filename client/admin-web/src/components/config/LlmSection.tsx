/**
 * `[llm]` 对话生成配置段（W6 / Config C3）。
 *
 * 字段清单与默认值：`plugins/llm/mod.rs:35-75`（`LlmConfig`）、`:76-108`
 * （各 `default_*`）；advanced 归属：`plugins/registry/projection.rs:93-96`
 * （`llm.max_history` / `llm.temperature` / `llm.stream` 在 `ADVANCED_KEYS`）。
 *
 * ⛔ 对齐旧 Kuikly，**不暴露** `llm.engine` / `llm.sentence_flush_ms` /
 * `llm.fallback.providers`：
 * - `fallback.providers` 是对象数组，前端 `diffPatch` 的 `writePath()` 不处理数组下标，
 *   把它塞进草稿会在保存时把整段替换成 `{}` 破坏结构；
 * - `GET /api/config/schema` 的投影里这段没有任何字段元数据，界面无从正确渲染。
 * 三者留待后续批次专门处理，本段只做 8 个可安全增量提交的字段。
 *
 * `llm` 不在 `APP_SECTION_HOT`（`plugins/registry/mod.rs:298-316`），
 * `SectionHotHint` 取不到就返回 null，不自造档位文案；段内提及档位时复用
 * `SaveBar.HOT_LABEL`，避免第四种口径。
 *
 * 保存只走 `setField("点分路径", value)`，由 `diffPatch` 生成最小叶子补丁；
 * 「获取模型列表」是**只测不存**的旁路诊断端点，不会改写任何配置字段。
 */

import { useState, type ReactElement } from "react";
import { LoaderCircle } from "lucide-react";
import { SectionCard } from "@/components/config/SectionCard";
import { SectionHotHint } from "@/components/config/SectionHotHint";
import { HOT_LABEL } from "@/components/config/SaveBar";
import { FieldRow } from "@/components/config/FieldRow";
import { Button } from "@/components/ui/button";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { FormNotice } from "@/components/form/FormNotice";
import { NumberField } from "@/components/form/NumberField";
import { SecretField } from "@/components/form/SecretField";
import { SwitchField } from "@/components/form/SwitchField";
import { TextAreaField } from "@/components/form/TextAreaField";
import { TextField } from "@/components/form/TextField";
import { toText } from "@/components/form/types";
import { useConfigDraft } from "@/context/ConfigDraftContext";
import {
  catalogueErrorMessage,
  fetchLlmModels,
  type LlmModelsRequest,
} from "@/lib/catalogueApi";

export function LlmSection(): ReactElement {
  const { getField, getSecret, setField } = useConfigDraft();
  const currentModel = toText(getField("llm.model"));

  const [models, setModels] = useState<string[] | null>(null);
  const [modelsBusy, setModelsBusy] = useState(false);
  const [modelsError, setModelsError] = useState<string | null>(null);
  const [modelsMessage, setModelsMessage] = useState<string | null>(null);

  const handleFetchModels = (): void => {
    setModelsBusy(true);
    setModelsError(null);
    setModelsMessage(null);
    setModels(null);

    // 只带非空覆盖项；两个都空时发 {}，服务端回落盘上已存 `[llm]` 凭据
    // （`server/src/benchmark/models.rs:56-74`）。密钥明文只来自 secretValues，绝不进草稿。
    const body: LlmModelsRequest = {};
    const apiBase = toText(getField("llm.api_base")).trim();
    const apiKey = (getSecret("llm.api_key") ?? "").trim();
    if (apiBase !== "") {
      body.api_base = apiBase;
    }
    if (apiKey !== "") {
      body.api_key = apiKey;
    }

    void fetchLlmModels(body)
      .then((result) => {
        setModels(result.models);
        setModelsMessage(
          result.models.length === 0
            ? "上游未返回模型（检查 api_base 是否为 OpenAI 兼容端点）；可直接手填模型名。"
            : `已获取 ${result.models.length} 个模型${result.elapsedMs === null ? "" : `（耗时 ${result.elapsedMs} ms）`}，可从下拉选择。`,
        );
      })
      .catch((error: unknown) => {
        setModelsError(catalogueErrorMessage(error));
      })
      .finally(() => {
        setModelsBusy(false);
      });
  };

  const selectedModel =
    models !== null && models.includes(currentModel) ? currentModel : "";

  return (
    <div className="space-y-4">
      <SectionHotHint sectionId="llm" />

      <FormNotice tone="info">
        本段只编辑下列 8 个字段：实现 id、残句定时出句等待与备用端点链沿用旧管理台口径，不在本页暴露。关闭「启用对话生成」会进入回声模式（技能未命中时把 ASR 文本原样 TTS 下发），不影响技能级联命中（plugins/llm/mod.rs:37-43）。
      </FormNotice>

      <SectionCard
        title="LLM · 对话生成"
        description="OpenAI 兼容 Responses API 接入与生成参数；字段来自 [llm]（plugins/llm/mod.rs:35-75）。"
      >
        <div className="divide-y divide-border">
          <SwitchField
            path="llm.enabled"
            label="启用对话生成"
            hint={`默认 true（plugins/llm/mod.rs:42-43、:100-103）。关闭 = 回声模式，仅用于验证 ASR→TTS 链路。该开关按轮现读全局配置，${HOT_LABEL.live}（plugins/llm/mod.rs:37-43）。`}
          />

          <TextField
            path="llm.api_base"
            label="API 地址"
            placeholder="https://api.example.com/v1/responses"
            hint="默认 https://api.example.com/v1/responses（plugins/llm/mod.rs:76-78）；需为 OpenAI 兼容 Responses 端点。"
          />

          <SecretField
            path="llm.api_key"
            hasPath="llm.has_api_key"
            label="API Key"
            hint="服务端 SECRET_PATHS 含 llm.api_key（server/src/gateway/ws/config.rs:74-86），GET 打码为 has_api_key。"
          />

          <TextField
            path="llm.model"
            label="模型名"
            placeholder="gpt-4o"
            hint="默认 gpt-4o（plugins/llm/mod.rs:80-82）。可直接手填，也可用下方「获取模型列表」的在线结果回填。"
          />

          <FieldRow
            label="在线模型列表"
            hint="只测不存：用上面的 API 地址 / API Key 覆盖项拉取上游 /models；两个覆盖项都为空时发空请求体，服务端回落到盘上已存的 [llm] 凭据。"
          >
            <div className="flex flex-wrap items-center gap-2">
              <Button
                type="button"
                variant="outline"
                size="sm"
                disabled={modelsBusy}
                onClick={handleFetchModels}
              >
                {modelsBusy ? (
                  <LoaderCircle aria-hidden="true" className="size-4 animate-spin" />
                ) : null}
                获取模型列表
              </Button>
              {models !== null && models.length > 0 ? (
                <Select
                  value={selectedModel}
                  onValueChange={(next) => setField("llm.model", next)}
                >
                  <SelectTrigger className="w-[260px]">
                    <SelectValue placeholder="从在线列表选择" />
                  </SelectTrigger>
                  <SelectContent>
                    {models.map((model) => (
                      <SelectItem key={model} value={model}>
                        {model}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              ) : null}
            </div>
            {modelsMessage !== null ? (
              <p className="pt-1 text-sm font-medium text-emerald-800">{modelsMessage}</p>
            ) : null}
            {modelsError !== null ? (
              <div className="pt-1">
                <FormNotice tone="danger">获取模型列表失败：{modelsError}</FormNotice>
              </div>
            ) : null}
          </FieldRow>

          <TextAreaField
            path="llm.system_prompt"
            label="系统提示词"
            rows={4}
            placeholder="你是一个有用且简洁的中文语音助手。"
            hint="默认「你是一个有用且简洁的中文语音助手。」（plugins/llm/mod.rs:84-86）。"
          />

          <NumberField
            path="llm.temperature"
            label="采样温度"
            integer={false}
            advanced
            placeholder="0.7"
            hint="f32，默认 0.7（plugins/llm/mod.rs:58-59、:92-94）；走小数通道，0.7 / 1.0 可直接写入。"
          />

          <SwitchField
            path="llm.stream"
            label="流式返回（SSE）"
            advanced
            hint="默认 true（plugins/llm/mod.rs:62、:96-98）；关闭后退回整段返回，兼容不支持 SSE 的端点。"
          />

          <NumberField
            path="llm.max_history"
            label="携带的历史轮数"
            advanced
            min={0}
            placeholder="10"
            hint="usize，默认 10（plugins/llm/mod.rs:56-57、:88-90）。"
          />
        </div>
      </SectionCard>
    </div>
  );
}
