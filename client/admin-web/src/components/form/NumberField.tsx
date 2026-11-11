/**
 * 数值字段控件（默认整数；`integer={false}` 走小数通道）。
 *
 * 关键点：本地文本 state 允许「空串 / 正在输入 / 前导零 / `0.`」这类中间态；
 * 只有格式合法、有限、且落在 min/max 内的数值才写进草稿。非法输入**绝不写 draft**，
 * 避免把 `NaN` / `""` 提交给服务端；失焦时回退显示草稿里的最后一次合法值。
 *
 * `integer` 默认 true，保证 W4 的 9 个 audio 整数与 server / aiui 数字行为完全不变；
 * 设为 false 时用于 f32 字段（VAD 的 threshold / 两个时长，GET 回包经
 * `server/src/gateway/ws/config/clean_floats.rs:12` 的 `clean_widened_f32` 收成短小数）。
 */

import { useEffect, useRef, useState, type ReactElement } from "react";
import { FieldRow } from "@/components/config/FieldRow";
import { Input } from "@/components/ui/input";
import { useConfigDraft } from "@/context/ConfigDraftContext";
import { toText, type FieldBaseProps } from "@/components/form/types";

export interface NumberFieldProps extends FieldBaseProps {
  min?: number;
  max?: number;
  placeholder?: string;
  /** 是否只允许整数（默认 true）；false 时允许 `0.5` / `0.` / `-0.25` 这类小数中间态。 */
  integer?: boolean;
}

/** 整数通道：纯十进制整数（排除 `1e3`、空白、小数、Infinity）。 */
function isIntegerText(raw: string): boolean {
  return /^-?\d+$/.test(raw.trim());
}

/** 小数通道：合法十进制数或「正在输入」的中间态（`0.` / `.5` / `-0.25`）。 */
function isDecimalText(raw: string): boolean {
  return /^-?(?:\d+\.?\d*|\.\d+)$/.test(raw.trim());
}

export function NumberField({
  path,
  label,
  hint,
  advanced,
  min,
  max,
  placeholder,
  integer = true,
}: NumberFieldProps): ReactElement {
  const { getField, setField } = useConfigDraft();
  const draftText = toText(getField(path));
  const [text, setText] = useState<string>(draftText);
  const [invalid, setInvalid] = useState(false);
  // 记录本控件最后一次写进草稿的字符串：草稿因外部 reload / discard / 保存回读而变化时
  // 才同步显示；自己写入造成的变化不打断用户正在输入的中间态。
  const lastCommitted = useRef<string>(draftText);

  useEffect(() => {
    if (draftText !== lastCommitted.current) {
      lastCommitted.current = draftText;
      setText(draftText);
      setInvalid(false);
    }
  }, [draftText]);

  const rangeText =
    min !== undefined && max !== undefined
      ? `${min} – ${max}`
      : min !== undefined
        ? `不小于 ${min}`
        : max !== undefined
          ? `不大于 ${max}`
          : integer
            ? "任意整数"
            : "任意数值";

  const handleChange = (raw: string): void => {
    setText(raw);
    const parsed = Number(raw.trim());
    const withinRange =
      (min === undefined || parsed >= min) && (max === undefined || parsed <= max);
    const formatOk = integer ? isIntegerText(raw) : isDecimalText(raw);
    const magnitudeOk = integer ? Number.isInteger(parsed) : true;
    if (formatOk && Number.isFinite(parsed) && magnitudeOk && withinRange) {
      lastCommitted.current = String(parsed);
      setField(path, parsed);
      setInvalid(false);
    } else {
      // 中间态或非法值：只提示，不写草稿。
      setInvalid(true);
    }
  };

  const handleBlur = (): void => {
    if (invalid || text !== draftText) {
      // 非法中间态回退到草稿值；合法但与草稿文本不一致（如 "0123" / "0."）也统一显示。
      lastCommitted.current = draftText;
      setText(draftText);
      setInvalid(false);
    }
  };

  return (
    <FieldRow label={label} hint={hint} advanced={advanced}>
      <Input
        type="text"
        inputMode={integer ? "numeric" : "decimal"}
        className="max-w-xs"
        value={text}
        placeholder={placeholder}
        aria-invalid={invalid}
        onChange={(event) => handleChange(event.target.value)}
        onBlur={handleBlur}
      />
      {invalid ? (
        <p className="mt-1 text-sm font-medium text-destructive">
          请输入合法{integer ? "整数" : "数值"}（范围 {rangeText}）；当前输入未写入草稿，不会提交给服务端。
        </p>
      ) : null}
    </FieldRow>
  );
}
