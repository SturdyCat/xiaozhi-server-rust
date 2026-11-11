/**
 * `[soul]` 人格档案配置段（W8 / Config C5）。
 *
 * 字段清单与默认值：`plugins/soul/mod.rs:71-144`（22 字段；emoji 默认 **false**——
 * TTS 会把 emoji 念出来；max_sentences=2 / max_chars=40 / colloquial=true）。
 * advanced 归属：`plugins/registry/projection.rs:42-92`——仅 enabled / preset / name /
 * address_user 四个非高级字段，其余 18 个全部 `advanced`（简洁模式由 FieldRow 自动隐藏）。
 *
 * 与旧端 Kuikly `ConfigSoulCard.kt` 的对应与差异：
 * - 简单模式提示逐字移植（ConfigSoulCard.kt:84）；未启用提示逐字（:280）。
 * - 四个高级动作保留：用内置默认人格启用 / 载入预设内容到表单 / 清空人格字段 /
 *   预览最终提示词。预览**只读**，超过 2000 字符截断并标注（对齐
 *   `ConfigContextState.kt` previewSoul 的截断口径），绝不伪装成已保存。
 * - 「载入预设内容到表单」走 preview 的 `effective` 整段回填：逐键 `setField("soul.<key>")`
 *   ——服务端返回的每个键写回草稿，未返回的键不动（等价旧端 `fill` 的「整段喂」语义，
 *   且不会在缺失键上把用户原文冲掉）。preset === "none" 时拦截（:192-196 同口径）。
 * - ⚠️ 5 个列表字段（traits / values / boundaries / catchphrases / examples）必须用
 *   `StringListField` 写 `string[]`，**禁止 TextAreaField**：schema 误把它们声明成 Text，
 *   照抄旧端「TextArea + lines()」的表单语义没问题，但把值当普通文本提交会在
 *   `soul.rs:68-74` 的数组校验处 400。
 * - 合并语义（留空=预设补、填写=覆盖）只实现在服务端（`soul::effective`）；客户端只做
 *   两件事：把当前草稿发过去、把回答显示出来，不在这里复制一份合并规则。
 *
 * 端点契约（与 W7 相反的三点，见 `soulMemoryApi.ts` 头注释）：presets / preview 均为
 * native-plugins 门控 + 无鉴权 + 裸 Value 请求体；形状不符按能力未启用处理
 * （`UnavailableNotice`），绝不伪装成空清单。
 */

import { useEffect, useRef, useState, type ReactElement } from "react";
import { LoaderCircle } from "lucide-react";
import { SectionCard } from "@/components/config/SectionCard";
import { SectionHotHint } from "@/components/config/SectionHotHint";
import { UnavailableNotice } from "@/components/feedback/UnavailableNotice";
import { Button } from "@/components/ui/button";
import { FormNotice } from "@/components/form/FormNotice";
import { NumberField } from "@/components/form/NumberField";
import { SelectField } from "@/components/form/SelectField";
import { StringListField } from "@/components/form/StringListField";
import { SwitchField } from "@/components/form/SwitchField";
import { TextAreaField } from "@/components/form/TextAreaField";
import { TextField } from "@/components/form/TextField";
import type { SelectOption } from "@/components/form/types";
import { useAdvancedMode } from "@/context/AdvancedModeContext";
import { useConfigDraft } from "@/context/ConfigDraftContext";
import type { JsonObject, JsonValue } from "@/lib/configApi";
import {
  getSoulPresets,
  isSoulMemoryUnavailable,
  previewSoul,
  soulMemoryErrorMessage,
  type SoulPresets,
  type SoulPreview,
} from "@/lib/soulMemoryApi";

/** 内置预设的兜底两项；服务端预设清单会追加合并（旧端 syncSoulPresets 同口径）。 */
const STATIC_PRESET_OPTIONS: SelectOption[] = [
  { value: "xiaozhi", label: "小智（内置默认人格）" },
  { value: "none", label: "不使用预设（完全自定义）" },
];

/** 清空动作覆盖的 16 个填写字段（不清 preset / colloquial / emoji / 数值 / enabled）。 */
const SOUL_TEXT_FIELDS: readonly string[] = [
  "name",
  "address_user",
  "self_intro",
  "form",
  "age_feel",
  "worldview",
  "backstory",
  "relationship_origin",
  "tone",
  "scenario",
  "device_hint",
];
/** 列表字段清空成 `[]`（与 StringListField 写回类型一致）。 */
const SOUL_LIST_FIELDS: readonly string[] = [
  "traits",
  "values",
  "boundaries",
  "catchphrases",
  "examples",
];

/** 安全把草稿值当 JSON 对象读（数组 / null / 缺失都不算）。 */
function asJsonObject(value: JsonValue | undefined): JsonObject | null {
  return typeof value === "object" && value !== null && !Array.isArray(value)
    ? (value as JsonObject)
    : null;
}

/** 预览正文截断：超过 2000 字符截断并标注（对齐旧端 previewSoul）。 */
function truncatePreview(text: string): string {
  return text.length > 2000 ? `${text.slice(0, 2000)}\n…（已截断）` : text;
}

export function SoulSection(): ReactElement {
  const { advancedMode } = useAdvancedMode();
  const { getField, setField } = useConfigDraft();

  const enabled = getField("soul.enabled") === true;
  const preset = (() => {
    const value = getField("soul.preset");
    return typeof value === "string" ? value : "";
  })();
  const soulDraft = asJsonObject(getField("soul"));

  // ---- 预设清单（GET /api/soul/presets，仅用于补全下拉；失败不影响表单保存）----
  const mountedRef = useRef(true);
  const [presets, setPresets] = useState<SoulPresets | null>(null);
  const [presetsLoading, setPresetsLoading] = useState(false);
  const [presetsError, setPresetsError] = useState<string | null>(null);
  const [presetsUnavailable, setPresetsUnavailable] = useState(false);
  const [presetsUnavailableMessage, setPresetsUnavailableMessage] = useState<string | null>(null);

  // ---- 高级动作共用状态（busy 门禁统一：任一动作在飞，四个按钮全部禁用）----
  const [busy, setBusy] = useState(false);
  const [actionMessage, setActionMessage] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const [preview, setPreview] = useState<SoulPreview | null>(null);

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
    };
  }, []);

  useEffect(() => {
    setPresetsLoading(true);
    void (async () => {
      try {
        const result = await getSoulPresets();
        if (!mountedRef.current) {
          return;
        }
        setPresets(result);
        setPresetsUnavailable(false);
        setPresetsUnavailableMessage(null);
        setPresetsError(null);
      } catch (error) {
        if (!mountedRef.current) {
          return;
        }
        setPresets(null);
        if (isSoulMemoryUnavailable(error)) {
          setPresetsUnavailable(true);
          setPresetsUnavailableMessage(soulMemoryErrorMessage(error));
          setPresetsError(null);
        } else {
          setPresetsError(soulMemoryErrorMessage(error));
        }
      } finally {
        if (mountedRef.current) {
          setPresetsLoading(false);
        }
      }
    })();
  }, []);

  /** 静态两项 + 服务端预设合并（按 id 去重；服务端新增预设无需前端发版）。 */
  const presetOptions: SelectOption[] = [...STATIC_PRESET_OPTIONS];
  if (presets !== null) {
    for (const item of presets.presets) {
      if (!presetOptions.some((option) => option.value === item.id)) {
        presetOptions.push({
          value: item.id,
          label: item.name === "" ? item.id : `${item.name}（${item.id}）`,
        });
      }
    }
  }

  /** 用内置默认人格启用：纯草稿操作（对齐 ConfigSoulCard.kt:62-66），不动端点。 */
  const handleEnableDefault = (): void => {
    setField("soul.preset", "xiaozhi");
    setField("soul.enabled", true);
    setActionError(null);
    setActionMessage(
      "已把预设切换为内置默认人格「小智」并启用人格档案；改动还在草稿里，点底部「保存」后生效。",
    );
  };

  /** 载入预设内容到表单：preview 的 effective 整段逐键回填（preset=none 拦截）。 */
  const handleLoadPreset = (): void => {
    if (preset === "none") {
      setActionError(null);
      setActionMessage(
        "当前预设为「不使用预设（完全自定义）」，没有可载入的内容；想用内置默认人格请先把预设改成「小智」。",
      );
      return;
    }
    setBusy(true);
    setActionMessage("载入中…");
    setActionError(null);
    void (async () => {
      try {
        const result = await previewSoul(soulDraft);
        if (!mountedRef.current) {
          return;
        }
        if (result.effective === null) {
          setActionMessage(null);
          setActionError("载入失败：服务端未返回 effective 档案（版本不匹配？）");
          return;
        }
        for (const [key, value] of Object.entries(result.effective)) {
          setField(`soul.${key}`, value);
        }
        setActionMessage(
          `已把「${result.presetName ?? preset}」的人格内容展开到表单（可直接改，改完点保存）。留空的字段运行时会自动用预设补。`,
        );
      } catch (error) {
        if (!mountedRef.current) {
          return;
        }
        setActionMessage(null);
        setActionError(`载入失败：${soulMemoryErrorMessage(error)}`);
      } finally {
        if (mountedRef.current) {
          setBusy(false);
        }
      }
    })();
  };

  /** 清空 16 个填写字段（列表清成 []）；不清 preset / colloquial / emoji / 数值 / enabled。 */
  const handleClearProfile = (): void => {
    for (const key of SOUL_TEXT_FIELDS) {
      setField(`soul.${key}`, "");
    }
    for (const key of SOUL_LIST_FIELDS) {
      setField(`soul.${key}`, []);
    }
    setPreview(null);
    setActionError(null);
    setActionMessage(
      preset === "none"
        ? "已清空人格字段。当前预设是「不使用预设」，因此 name / address_user 必须自己填，否则保存后人格不生效。"
        : `已清空人格字段：留空的字段运行时会自动用预设「${preset}」补。改动还在草稿里，点底部「保存」后生效。`,
    );
  };

  /** 预览最终提示词（只读）：把当前草稿发过去，回答原样显示，不落盘。 */
  const handlePreview = (): void => {
    setBusy(true);
    setActionMessage("生成预览…");
    setActionError(null);
    void (async () => {
      try {
        const result = await previewSoul(soulDraft);
        if (!mountedRef.current) {
          return;
        }
        setPreview(result);
        setActionMessage(null);
      } catch (error) {
        if (!mountedRef.current) {
          return;
        }
        setPreview(null);
        setActionError(`预览失败：${soulMemoryErrorMessage(error)}`);
      } finally {
        if (mountedRef.current) {
          setBusy(false);
        }
      }
    })();
  };

  return (
    <div className="space-y-4">
      <SectionHotHint sectionId="soul" />

      <SectionCard
        title="人格档案（[soul]）"
        description="启用后，填写的内容会在调用大模型前拼成系统提示词；留空的字段由「预设」补，填了的字段逐字段覆盖预设（合并在服务端实现）。"
      >
        <div className="divide-y divide-border">
          <SwitchField
            path="soul.enabled"
            label="启用人格档案"
            hint="开启后，下面填写的内容会在调用大模型前拼成系统提示词；留空的字段由「预设」补，填了的字段逐字段覆盖预设。关闭则完全使用 LLM 卡里的系统提示词。"
          />

          {enabled ? (
            <>
              <SelectField
                path="soul.preset"
                label="内置预设"
                options={presetOptions}
                hint="选一个预设作为基线：留空的字段由它补，填了的字段覆盖它。「不使用预设」= 完全自定义，此时 name / address_user 必须自己填。"
              />

              {presetsLoading ? (
                <p className="flex items-center gap-2 py-2 text-sm text-muted-foreground">
                  <LoaderCircle aria-hidden="true" className="size-4 animate-spin" />
                  读取服务端预设清单…
                </p>
              ) : null}
              {presetsUnavailable ? (
                <div className="space-y-2 py-2">
                  <UnavailableNotice
                    title="人格预设清单能力未启用"
                    description={
                      presetsUnavailableMessage ??
                      "服务端未挂载 /api/soul/presets（通常是构建未启用 native-plugins）。"
                    }
                  />
                  <FormNotice tone="warning">
                    预设清单端点未启用或被管理页 HTML 回退接走：下拉仍保留内置两项，高级模式下的动作与预览不可用，但 [soul] 配置字段仍可正常保存。
                  </FormNotice>
                </div>
              ) : null}
              {presetsError !== null ? (
                <div className="py-2">
                  <FormNotice tone="danger">读取人格预设清单失败：{presetsError}</FormNotice>
                </div>
              ) : null}
              {presets !== null && presets.note !== null ? (
                <div className="py-2">
                  <FormNotice tone="info">{presets.note}</FormNotice>
                </div>
              ) : null}

              <TextField path="soul.name" label="自称" hint="自称；默认「小智」。" />
              <TextField path="soul.address_user" label="称呼用户" />

              <TextField
                path="soul.self_intro"
                label="自我定位"
                advanced
                hint="一段话：你是谁、在做什么。"
              />
              <TextField
                path="soul.form"
                label="形象 / 物种"
                advanced
                hint="如「一只住在音箱里的小狐狸」。"
              />
              <TextField path="soul.age_feel" label="年龄感" advanced hint="给人的年龄感。" />
              <TextAreaField
                path="soul.worldview"
                label="世界观"
                advanced
                hint="世界是什么样的。"
                rows={3}
              />
              <TextAreaField path="soul.backstory" label="来历 / 背景故事" advanced rows={3} />
              <TextAreaField
                path="soul.relationship_origin"
                label="与用户的关系起点"
                advanced
                hint="你与用户的关系是怎么开始的（比「关系」更能锁定语气）。"
                rows={3}
              />
              <StringListField
                path="soul.traits"
                label="性格"
                advanced
                rows={4}
                placeholder="每行一条：标签：遇到 X 会怎么做"
                hint="性格：每条写「标签：遇到 X 会怎么做」。只写形容词会失效。"
              />
              <StringListField path="soul.values" label="价值观" advanced rows={3} hint="坚持的价值。" />
              <StringListField
                path="soul.boundaries"
                label="红线 / 拒绝清单"
                advanced
                rows={3}
                hint="拒绝清单 / 红线。"
              />
              <TextField
                path="soul.tone"
                label="语气"
                advanced
                hint="如「轻快、温和、带一点好奇」。"
              />
              <SwitchField
                path="soul.colloquial"
                label="说人话（短句、可省略主语）"
                advanced
                hint="口语化表达；默认开启。"
              />
              <StringListField
                path="soul.catchphrases"
                label="口头禅"
                advanced
                rows={3}
                hint="口头禅（自然使用，不必每句都用）。"
              />
              <SwitchField
                path="soul.emoji"
                label="允许 emoji"
                advanced
                hint="语音默认关闭：TTS 会把 emoji 念出来。"
              />
              <NumberField
                path="soul.max_sentences"
                label="回复最多句数"
                advanced
                min={1}
                placeholder="2"
                hint="u32，默认 2；写进平台段的硬约束。"
              />
              <NumberField
                path="soul.max_chars"
                label="回复最多字数"
                advanced
                min={1}
                placeholder="40"
                hint="u32，默认 40；写进平台段的硬约束。"
              />
              <TextField
                path="soul.scenario"
                label="当前情境"
                advanced
                hint="当前情境 / 场景（如「书房里，晚上」）。"
              />
              <TextField
                path="soul.device_hint"
                label="设备提示"
                advanced
                hint="设备相关提示（如「这是放在客厅的音箱」）。"
              />
              <StringListField
                path="soul.examples"
                label="风格范例"
                advanced
                rows={4}
                placeholder={"用户：今天几度？\n你：我这边看不到天气，你手机上看一眼？"}
                hint="每条一组 Q/A（两行）：锁风格最有效的手段，优先于长篇描述。"
              />

              {!advancedMode ? (
                <p className="py-2 text-xs text-muted-foreground">
                  预设与既有人格细节继续保留；高级模式可调整完整档案与预览，不会因模式切换恢复默认。
                </p>
              ) : null}
            </>
          ) : (
            <p className="py-2 text-xs text-muted-foreground">
              未启用：使用「LLM · 对话生成」里的系统提示词（行为与加人格之前一致）。
            </p>
          )}
        </div>
      </SectionCard>

      {enabled && advancedMode ? (
        <SectionCard
          title="人格动作（高级）"
          description="四个动作只读写当前草稿与服务端合并结果，不会直接改盘上配置；改动要生效请点底部「保存」。预览是只读的，不会保存任何内容。"
        >
          <div className="space-y-3">
            <div className="flex flex-wrap items-center gap-2">
              <Button
                type="button"
                variant="outline"
                size="sm"
                disabled={busy}
                onClick={handleEnableDefault}
              >
                用内置默认人格启用
              </Button>
              <Button
                type="button"
                variant="outline"
                size="sm"
                disabled={busy}
                onClick={handleLoadPreset}
              >
                载入预设内容到表单
              </Button>
              <Button
                type="button"
                variant="outline"
                size="sm"
                disabled={busy}
                onClick={handleClearProfile}
              >
                清空人格字段
              </Button>
              <Button
                type="button"
                variant="outline"
                size="sm"
                disabled={busy}
                onClick={handlePreview}
              >
                {busy ? <LoaderCircle aria-hidden="true" className="size-4 animate-spin" /> : null}
                预览最终提示词
              </Button>
            </div>

            {busy && actionMessage !== null ? (
              <p className="flex items-center gap-2 text-sm font-medium">
                <LoaderCircle aria-hidden="true" className="size-4 animate-spin" />
                {actionMessage}
              </p>
            ) : null}
            {!busy && actionMessage !== null ? (
              <FormNotice tone="info">{actionMessage}</FormNotice>
            ) : null}
            {actionError !== null ? (
              <FormNotice tone="danger">{actionError}</FormNotice>
            ) : null}

            {preview !== null ? (
              <div className="space-y-2">
                <p className="text-sm font-medium">
                  最终提示词（预览，只读；不会保存）
                </p>
                <div className="space-y-1 text-sm text-muted-foreground">
                  <p>
                    来源：
                    {preview.source === "soul"
                      ? "人格档案"
                      : "[llm].system_prompt（人格未启用）"}
                    {preview.presetName !== null ? ` · 预设：${preview.presetName}` : ""}
                  </p>
                  <p>
                    {preview.chars !== null ? `${preview.chars} 字符` : "字符数未知"} · 约{" "}
                    {preview.approxTokens ?? "?"} tokens
                  </p>
                  {preview.fromPreset.length > 0 ? (
                    <p>来自预设（你留空的字段）：{preview.fromPreset.join("、")}</p>
                  ) : null}
                  {preview.overridden.length > 0 ? (
                    <p>你已覆盖的字段：{preview.overridden.join("、")}</p>
                  ) : null}
                </div>
                {preview.note !== null ? (
                  <FormNotice tone="warning">{preview.note}</FormNotice>
                ) : null}
                {preview.instructions !== "" ? (
                  <pre className="max-h-72 overflow-auto whitespace-pre-wrap rounded-md border border-border bg-muted/40 px-3 py-2 text-xs leading-relaxed text-muted-foreground">
                    {truncatePreview(preview.instructions)}
                  </pre>
                ) : (
                  <FormNotice tone="warning">
                    服务端返回的提示词正文为空：通常是人格未启用或档案全空，请核对上方「来源」与 enabled 开关。
                  </FormNotice>
                )}
              </div>
            ) : null}
          </div>
        </SectionCard>
      ) : null}
    </div>
  );
}
