/**
 * 多行文本字段控件（`aiui.prompt` 等人设 / 提示词）。
 */

import type { ReactElement } from "react";
import { FieldRow } from "@/components/config/FieldRow";
import { Textarea } from "@/components/ui/textarea";
import { useConfigDraft } from "@/context/ConfigDraftContext";
import { toText, type FieldBaseProps } from "@/components/form/types";

export interface TextAreaFieldProps extends FieldBaseProps {
  placeholder?: string;
  rows?: number;
}

export function TextAreaField({
  path,
  label,
  hint,
  advanced,
  placeholder,
  rows = 5,
}: TextAreaFieldProps): ReactElement {
  const { getField, setField } = useConfigDraft();

  return (
    <FieldRow label={label} hint={hint} advanced={advanced}>
      <Textarea
        className="max-w-2xl"
        rows={rows}
        value={toText(getField(path))}
        placeholder={placeholder}
        onChange={(event) => setField(path, event.target.value)}
      />
    </FieldRow>
  );
}
