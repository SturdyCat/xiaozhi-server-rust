/**
 * JSON 文本域 + planJson 形态守卫（W7 / Config C4）。
 *
 * [`nluPlanError`] 移植旧端 `ConfigNluPlanJson.kt:20-33` 的纯客户端
 * Syntax/shape guard，判据逐条对齐：
 * - ≤32768 字节（UTF-8）；
 * - 合法 JSON 且是**对象**；
 * - `conditions` 若存在必须是数组且 ≤16 条；
 * - `steps` 必须是数组且 1~16 条；
 * - 每条 step 必须是对象。
 *
 * **文案必须诚实**：形状检查通过 ≠ 服务端校验通过。能力名 / 条件 / 参数语义
 * 以服务端为准（`SkillPlan` / `SkillStep` 均 `deny_unknown_fields`，未知字段会在
 * 保存时返回 400 纯文本）。禁止把 `/api/workflow/validate` 当 intent plan 的
 * 预提交校验（它校验的是 `[workflow]` 段，`workflow.rs:38-53`）；禁止把
 * 「形状检查通过」写成「服务端校验通过」。
 */

import type { ReactElement } from "react";
import { FieldRow } from "@/components/config/FieldRow";
import { Textarea } from "@/components/ui/textarea";

/** planJson 的最大字节数（UTF-8），与旧端一致。 */
const PLAN_MAX_BYTES = 32768;

/** planJson 形态守卫：返回 null 表示通过，否则返回可展示的中文错误。 */
export function nluPlanError(raw: string): string | null {
  if (new TextEncoder().encode(raw).length > PLAN_MAX_BYTES) {
    return "能力组合 JSON 超过 32768 字节，请缩小配置。";
  }
  let value: unknown;
  try {
    value = JSON.parse(raw);
  } catch {
    return "能力组合不是合法 JSON；输入已保留，修正后才能保存。";
  }
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    return "能力组合必须是 JSON 对象。";
  }
  const plan = value as Record<string, unknown>;
  const conditions = plan["conditions"];
  if (conditions !== undefined) {
    if (!Array.isArray(conditions)) {
      return "条件必须是数组。";
    }
    if (conditions.length > 16) {
      return "条件最多 16 条。";
    }
  }
  const steps = plan["steps"];
  if (!Array.isArray(steps)) {
    return "请提供步骤数组 steps。";
  }
  if (steps.length === 0 || steps.length > 16) {
    return "步骤需要 1～16 条，不能只保存空的能力组合。";
  }
  for (const step of steps) {
    if (typeof step !== "object" || step === null || Array.isArray(step)) {
      return "每个步骤必须是含 api 与 args 的对象。";
    }
  }
  return null;
}

export interface JsonTextFieldProps {
  label: string;
  /** 当前文本态（由调用方持有：非法输入也要保留显示，不能被重置）。 */
  value: string;
  /** 文本变化回调；守卫判定与写回策略由调用方处理。 */
  onChange: (next: string) => void;
  /** 即时守卫错误（[`nluPlanError`] 的返回值），显示在输入框下方。 */
  error: string | null;
  hint?: string;
  placeholder?: string;
  rows?: number;
}

export function JsonTextField({
  label,
  value,
  onChange,
  error,
  hint,
  placeholder,
  rows = 6,
}: JsonTextFieldProps): ReactElement {
  return (
    <FieldRow label={label} hint={hint}>
      <Textarea
        className="max-w-2xl font-mono text-xs"
        rows={rows}
        value={value}
        placeholder={placeholder}
        onChange={(event) => onChange(event.target.value)}
      />
      {error !== null ? (
        <p className="mt-1 text-sm font-medium text-destructive">{error}</p>
      ) : null}
    </FieldRow>
  );
}
